//! Remote store backed by a foray MCP stdio server subprocess.

use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, Content, ErrorData};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{Peer, RoleClient, ServiceError, serve_client};
use serde_json::Value;
use std::io;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::migrate::{self, MigrateResult};
use crate::store::{Store, StoreError};
use crate::types::{JournalFile, JournalItem, JournalSummary, Pagination};

// ── Connection ───────────────────────────────────────────────────────

/// A live connection to a remote foray MCP server.
struct Connection {
    /// Keep-alive: the background task shuts down when this is dropped.
    _service: RunningService<RoleClient, ()>,
    /// Clone-able handle for making calls without holding the lock.
    peer: Peer<RoleClient>,
    nuance: String,
    store_name: String,
}

// ── StdioStore ───────────────────────────────────────────────────────

/// A `Store` backed by a remote foray server accessed over MCP stdio.
///
/// The subprocess is spawned lazily on the first store operation and reused
/// for all subsequent calls. Dropping the `StdioStore` shuts down the
/// subprocess via the `RunningService` drop guard.
pub struct StdioStore {
    command: String,
    args: Vec<String>,
    /// Environment variable overrides passed to the subprocess.
    env: Vec<(String, String)>,
    /// Preferred store name on the remote server.
    /// When `None`, the first store from the `hello` response is used.
    store_hint: Option<String>,
    conn: Mutex<Option<Connection>>,
}

impl StdioStore {
    pub fn new(
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        store_hint: Option<String>,
    ) -> Self {
        Self {
            command,
            args,
            env,
            store_hint,
            conn: Mutex::new(None),
        }
    }

    /// Connect to the remote server if not already connected.
    ///
    /// Returns a cloned `(Peer, nuance, store_name)` tuple — the peer is
    /// channel-backed and cheap to clone, so it can be used for calls
    /// *outside* the lock, avoiding holding the mutex across `.await`.
    ///
    /// Uses a double-checked pattern: the lock is released before any async
    /// work so concurrent store operations are never blocked during subprocess
    /// spawn or MCP handshake.  If two callers both find `None` and race to
    /// connect, the second one's connection is dropped and the first one's is
    /// reused.
    async fn connect(&self) -> Result<(Peer<RoleClient>, String, String), StoreError> {
        // Fast path: already connected — return cheaply without any I/O.
        {
            let guard = self.conn.lock().await;
            if let Some(conn) = guard.as_ref() {
                return Ok((
                    conn.peer.clone(),
                    conn.nuance.clone(),
                    conn.store_name.clone(),
                ));
            }
        } // lock released before any .await

        eprintln!("Connecting to remote foray...");

        // Build subprocess command.
        // `foray_stdio` is foray-specific: we always append `serve` so that
        // `args` can stay transport-focused (e.g. SSH flags + `--` + binary
        // name) without repeating the subcommand in every config entry.
        let mut cmd = Command::new(&self.command);
        for arg in &self.args {
            cmd.arg(arg);
        }
        cmd.arg("serve");
        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        let transport = TokioChildProcess::new(cmd).map_err(StoreError::Io)?;

        // MCP initialize handshake.
        let service: RunningService<RoleClient, ()> = serve_client((), transport)
            .await
            .map_err(|e| io_err(e.to_string()))?;

        // Call hello to get nuance + store list.
        let hello_result = service
            .call_tool(CallToolRequestParams::new("hello"))
            .await
            .map_err(|e| io_err(e.to_string()))?;

        let text =
            first_text(&hello_result.content).ok_or_else(|| io_err("empty hello response"))?;

        let hello: Value = serde_json::from_str(text).map_err(|e| io_err(e.to_string()))?;

        let nuance = hello["nuance"]
            .as_str()
            .ok_or_else(|| io_err("missing nuance in hello response"))?
            .to_string();

        let store_name = self
            .store_hint
            .clone()
            .or_else(|| {
                hello["stores"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|s| s["name"].as_str())
                    .map(String::from)
            })
            .ok_or_else(|| io_err("no stores available in hello response"))?;

        let peer = service.peer().clone();

        // Re-lock to install.  Another task may have connected concurrently;
        // if so, drop the new service and return the existing connection.
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok((
                conn.peer.clone(),
                conn.nuance.clone(),
                conn.store_name.clone(),
            ));
        }
        *guard = Some(Connection {
            _service: service,
            peer: peer.clone(),
            nuance: nuance.clone(),
            store_name: store_name.clone(),
        });

        Ok((peer, nuance, store_name))
    }

    /// Call an MCP tool and return the parsed JSON response.
    ///
    /// Injects `nuance` and `store` into the arguments automatically.
    /// Maps MCP-level errors to `StoreError` variants.
    /// Clears the cached connection on transport failures or nuance mismatches
    /// so the next call reconnects with a fresh subprocess.
    async fn call_mcp(&self, tool: &'static str, mut args: Value) -> Result<Value, StoreError> {
        let (peer, nuance, store_name) = self.connect().await?;

        args["nuance"] = Value::String(nuance);
        args["store"] = Value::String(store_name);

        let arguments = match args {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };

        let params = CallToolRequestParams::new(tool).with_arguments(arguments);

        match peer.call_tool(params).await {
            Ok(result) => {
                let text =
                    first_text(&result.content).ok_or_else(|| io_err("empty tool response"))?;
                serde_json::from_str(text).map_err(|e| io_err(e.to_string()))
            }
            Err(ServiceError::McpError(e)) => {
                let err = classify_mcp_error(&e);
                // Nuance mismatch means the remote server restarted or was
                // reconfigured — clear the connection so the next call
                // reconnects and fetches a fresh nuance.
                if is_nuance_mismatch(&e.message) {
                    *self.conn.lock().await = None;
                }
                Err(err)
            }
            Err(e) => {
                // Transport failure — subprocess died or pipe broke.
                // Clear the connection so the next call spawns a fresh subprocess.
                *self.conn.lock().await = None;
                Err(io_err(e.to_string()))
            }
        }
    }

    /// Poison the cached nuance for testing reconnect behaviour.
    #[cfg(test)]
    pub async fn poison_nuance(&self) {
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_mut() {
            conn.nuance = "poisoned-nuance".to_string();
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

fn first_text(content: &[Content]) -> Option<&str> {
    content.first()?.as_text().map(|t| t.text.as_str())
}

fn io_err(msg: impl Into<String>) -> StoreError {
    StoreError::Io(io::Error::other(msg.into()))
}

fn unsupported_err(op: &str) -> StoreError {
    StoreError::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("StdioStore: '{op}' is not exposed by the remote MCP server"),
    ))
}

/// Returns true if the error message indicates a stale nuance token.
fn is_nuance_mismatch(msg: &str) -> bool {
    msg.contains("call 'hello' to get the current nuance")
}

/// Map a foray server error to the appropriate `StoreError` variant.
///
/// Branches on `data["type"]` first (structured errors from schema-aware
/// servers), then falls back to message-prefix matching for older servers.
fn classify_mcp_error(e: &ErrorData) -> StoreError {
    use crate::store::SchemaOrigin;
    // Structured path: check data["type"] first.
    if let Some(t) = e.data.as_ref().and_then(|d| d["type"].as_str()) {
        match t {
            "journal_not_found" => {
                let name = e
                    .data
                    .as_ref()
                    .and_then(|d| d["name"].as_str())
                    .unwrap_or("")
                    .to_string();
                return StoreError::NotFound(name);
            }
            "journal_already_exists" => {
                let name = e
                    .data
                    .as_ref()
                    .and_then(|d| d["name"].as_str())
                    .unwrap_or("")
                    .to_string();
                return StoreError::AlreadyExists(name);
            }
            "journal_archived" => {
                let name = e
                    .data
                    .as_ref()
                    .and_then(|d| d["name"].as_str())
                    .unwrap_or("")
                    .to_string();
                return StoreError::Archived(name);
            }
            "schema_too_new" => {
                let found = e
                    .data
                    .as_ref()
                    .and_then(|d| d["found"].as_u64())
                    .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
                    .unwrap_or(u32::MAX);
                let max = e
                    .data
                    .as_ref()
                    .and_then(|d| d["max"].as_u64())
                    .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
                    .unwrap_or(0);
                return StoreError::SchemaTooNew {
                    found,
                    max,
                    origin: SchemaOrigin::Storage,
                };
            }
            _ => {}
        }
    }
    // Fallback: message-prefix matching for pre-structured-errors servers.
    let msg = &e.message;
    if let Some(rest) = msg.strip_prefix("journal not found:") {
        return StoreError::NotFound(rest.trim().to_string());
    }
    if let Some(rest) = msg.strip_prefix("journal already exists:") {
        return StoreError::AlreadyExists(rest.trim().to_string());
    }
    if let Some(rest) = msg.strip_prefix("journal is archived:") {
        return StoreError::Archived(rest.trim().to_string());
    }
    io_err(msg.to_string())
}

// ── Store impl ───────────────────────────────────────────────────────

#[async_trait]
impl Store for StdioStore {
    async fn create(&self, journal: JournalFile) -> Result<(), StoreError> {
        let mut args = serde_json::json!({ "name": journal.name });
        if let Some(title) = &journal.title {
            args["title"] = serde_json::Value::String(title.clone());
        }
        if let Some(meta) = &journal.meta {
            args["meta"] = serde_json::to_value(meta).unwrap_or_default();
        }
        let resp = self.call_mcp("open_journal", args).await?;
        // `created: false` means the journal already existed — treat as conflict.
        if resp["created"].as_bool() == Some(false) {
            return Err(StoreError::AlreadyExists(journal.name));
        }
        // Persist any initial items (e.g. from `foray import` or `fork_journal`).
        if !journal.items.is_empty() {
            self.add_items(&journal.name, journal.items).await?;
        }
        Ok(())
    }

    async fn load(
        &self,
        name: &str,
        pagination: &Pagination,
    ) -> Result<(JournalFile, usize), StoreError> {
        let mut args = serde_json::json!({ "name": name });
        // `cursor` is the item offset; omit if 0 (server default).
        if let Some(offset) = pagination.offset.filter(|&o| o > 0) {
            args["cursor"] = Value::from(offset);
        }
        if let Some(limit) = pagination.limit {
            args["limit"] = Value::from(limit);
        }

        let v = self.call_mcp("sync_journal", args).await?;

        let total = v["total"].as_u64().unwrap_or(0) as usize;

        // Run migrate() on the wire response to handle version mismatches.
        // Construct a JournalFile-shaped Value so the migration chain can
        // inspect and transform the items array.
        let wire = serde_json::json!({
            "schema": v["schema"],
            "id":     v["id"],
            "name":   v["name"],
            "title":  v["title"],
            "items":  v["items"],
            "_note":  null,
            "meta":   null,
        });
        let migrated = match migrate::migrate(wire) {
            MigrateResult::Current(v) | MigrateResult::Migrated(v) => v,
            MigrateResult::TooNew { found, max } => {
                return Err(StoreError::SchemaTooNew {
                    found,
                    max,
                    origin: crate::store::SchemaOrigin::Wire,
                });
            }
        };

        let items: Vec<JournalItem> =
            serde_json::from_value(migrated["items"].clone()).map_err(|e| io_err(e.to_string()))?;

        let journal = JournalFile {
            _note: None,
            schema: migrate::CURRENT_SCHEMA,
            id: migrated["id"].as_str().unwrap_or("unknown").to_string(),
            name: migrated["name"].as_str().unwrap_or(name).to_string(),
            title: migrated["title"].as_str().map(String::from),
            items,
            meta: None,
        };

        Ok((journal, total))
    }

    async fn add_items(&self, name: &str, items: Vec<JournalItem>) -> Result<usize, StoreError> {
        let items_json: Vec<Value> = items
            .iter()
            .map(|item| {
                // Serialize ItemType to its snake_case string representation.
                let type_str = serde_json::to_value(&item.item_type)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "note".to_string());

                let mut obj = serde_json::json!({
                    "content": item.content,
                    "item_type": type_str,
                });
                if let Some(r) = &item.file_ref {
                    obj["ref"] = Value::String(r.clone());
                }
                if let Some(tags) = &item.tags {
                    obj["tags"] = serde_json::to_value(tags).unwrap_or_default();
                }
                if let Some(meta) = &item.meta {
                    obj["meta"] = serde_json::to_value(meta).unwrap_or_default();
                }
                obj
            })
            .collect();

        let args = serde_json::json!({ "name": name, "items": items_json });
        let v = self.call_mcp("sync_journal", args).await?;
        Ok(v["total"].as_u64().unwrap_or(0) as usize)
    }

    async fn list(
        &self,
        pagination: &Pagination,
        archived: bool,
    ) -> Result<(Vec<JournalSummary>, usize), StoreError> {
        let mut args = serde_json::json!({ "archived": archived });
        if let Some(limit) = pagination.limit {
            args["limit"] = Value::from(limit);
        }
        if let Some(offset) = pagination.offset {
            args["offset"] = Value::from(offset);
        }

        let v = self.call_mcp("list_journals", args).await?;

        let total = v["total"].as_u64().unwrap_or(0) as usize;
        let summaries: Vec<JournalSummary> =
            serde_json::from_value(v["journals"].clone()).map_err(|e| io_err(e.to_string()))?;

        Ok((summaries, total))
    }

    async fn exists(&self, name: &str) -> Result<bool, StoreError> {
        // limit: 0 — we only need to know if the journal exists, not its items.
        let args = serde_json::json!({ "name": name, "limit": 0 });
        match self.call_mcp("sync_journal", args).await {
            Ok(_) => Ok(true),
            Err(StoreError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn delete(&self, _name: &str) -> Result<(), StoreError> {
        Err(unsupported_err("delete"))
    }

    async fn archive(&self, name: &str) -> Result<String, StoreError> {
        let args = serde_json::json!({ "name": name });
        let v = self.call_mcp("archive_journal", args).await?;
        v["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| io_err("archive_journal: missing id"))
    }

    async fn unarchive(&self, name: &str) -> Result<String, StoreError> {
        let args = serde_json::json!({ "name": name });
        let v = self.call_mcp("unarchive_journal", args).await?;
        v["id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| io_err("unarchive_journal: missing id"))
    }
}
