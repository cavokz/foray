//! Remote store backed by a foray MCP stdio server subprocess.

use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, Content, ErrorData};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{Peer, RoleClient, ServiceError, serve_client};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::migrate::{self, MigrateResult};
use crate::store::{Store, StoreError};
use crate::types::{JournalFile, JournalItem, JournalSummary, Pagination};

// ── Wire response types ──────────────────────────────────────────────

/// Typed wire response for the `hello` tool.
///
/// Fields must match the current server's `HelloResponse` exactly.
/// `adapt_receive` normalises old server responses to this shape before
/// deserialization, so `deny_unknown_fields` is safe and guarantees that
/// `adapt_receive` tells the whole compatibility story.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HelloWire {
    #[allow(dead_code)]
    version: String,
    nuance: String,
    #[allow(dead_code)]
    protocol: u32,
    stores: Vec<StoreInfoWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreInfoWire {
    name: String,
    #[allow(dead_code)]
    description: String,
}

/// Typed wire response for `sync_journal`.
///
/// `adapt_receive` inserts `schema: 0` for pre-versioning servers.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct SyncJournalWire {
    schema: u32,
    name: String,
    title: String,
    items: Vec<Value>,
    added_ids: Vec<String>,
    cursor: usize,
    total: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenJournalWire {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    title: String,
    #[allow(dead_code)]
    item_count: usize,
    created: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListJournalsWire {
    journals: Vec<JournalSummary>,
    total: usize,
    #[allow(dead_code)]
    limit: Option<usize>,
    #[allow(dead_code)]
    offset: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveWire {
    #[allow(dead_code)]
    archived: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnarchiveWire {
    #[allow(dead_code)]
    unarchived: String,
}

// ── Connection ───────────────────────────────────────────────────────

/// Bounded stderr buffer — caps collection at 4 KB to avoid unbounded growth.
const STDERR_BUF_LIMIT: usize = 4 * 1024;

/// A live connection to a remote foray MCP server.
struct Connection {
    /// Keep-alive: the background task shuts down when this is dropped.
    _service: RunningService<RoleClient, ()>,
    /// Clone-able handle for making calls without holding the lock.
    peer: Peer<RoleClient>,
    nuance: String,
    store_name: String,
    /// Wire protocol version reported by the remote server's `hello` response.
    protocol: u32,
    /// Stderr collected by the background drain task. Read on connection failure.
    stderr_buf: Arc<Mutex<String>>,
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
    async fn connect(&self) -> Result<(Peer<RoleClient>, String, String, u32), StoreError> {
        // Fast path: already connected — return cheaply without any I/O.
        {
            let guard = self.conn.lock().await;
            if let Some(conn) = guard.as_ref() {
                return Ok((
                    conn.peer.clone(),
                    conn.nuance.clone(),
                    conn.store_name.clone(),
                    conn.protocol,
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

        let (transport, stderr_handle) = TokioChildProcess::builder(cmd)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(StoreError::Io)?;

        // MCP initialize handshake.
        // Hold stderr_handle here — we start the background drain only on
        // success so that on failure we can read it directly without any race.
        let service: RunningService<RoleClient, ()> = match serve_client((), transport).await {
            Ok(s) => s,
            Err(e) => {
                // On handshake failure, drain stderr with a short timeout:
                // if the subprocess died, EOF arrives immediately;
                // if it's still alive (higher-level failure), we get whatever
                // arrived during the handshake and move on.
                let stderr_output = if let Some(stderr) = stderr_handle {
                    let mut buf = Vec::new();
                    let _ = tokio::time::timeout(
                        Duration::from_millis(500),
                        stderr.take(STDERR_BUF_LIMIT as u64).read_to_end(&mut buf),
                    )
                    .await;
                    let s = String::from_utf8_lossy(&buf).into_owned();
                    if !s.is_empty() {
                        eprint!("[remote stderr] {s}");
                    }
                    s
                } else {
                    String::new()
                };
                let base = e.to_string();
                let msg = if stderr_output.trim().is_empty() {
                    base
                } else {
                    format!("{base}: {}", stderr_output.trim())
                };
                return Err(io_err(msg));
            }
        };

        // Handshake succeeded — now start the background stderr drain.
        // It forwards output to the server log and accumulates into a bounded
        // buffer so any future transport failure can include the subprocess
        // stderr in its error message.
        let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        if let Some(mut stderr) = stderr_handle {
            let buf = stderr_buf.clone();
            tokio::spawn(async move {
                let mut chunk = [0u8; 512];
                loop {
                    match stderr.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let s = String::from_utf8_lossy(&chunk[..n]);
                            eprint!("[remote stderr] {s}");
                            // Keep the most recent output: if adding this chunk
                            // would exceed the cap, evict enough from the front
                            // first so errors always reflect the latest stderr.
                            // Snap offsets to char boundaries to avoid panics
                            // on multi-byte UTF-8 sequences in stderr.
                            let s_capped = {
                                let start = if s.len() > STDERR_BUF_LIMIT {
                                    let raw = s.len() - STDERR_BUF_LIMIT;
                                    (raw..=s.len())
                                        .find(|&i| s.is_char_boundary(i))
                                        .unwrap_or(s.len())
                                } else {
                                    0
                                };
                                &s[start..]
                            };
                            let mut guard = buf.lock().await;
                            let excess = guard.len() + s_capped.len();
                            if excess > STDERR_BUF_LIMIT {
                                let drain_to = excess - STDERR_BUF_LIMIT;
                                let safe = (drain_to..=guard.len())
                                    .find(|&i| guard.is_char_boundary(i))
                                    .unwrap_or(guard.len());
                                guard.drain(..safe);
                            }
                            guard.push_str(s_capped);
                        }
                    }
                }
            });
        }

        // Call hello to get nuance + store list.
        let hello_result = service
            .call_tool(CallToolRequestParams::new("hello"))
            .await
            .map_err(|e| io_err(e.to_string()))?;

        let text =
            first_text(&hello_result.content).ok_or_else(|| io_err("empty hello response"))?;

        // Peek protocol before adaptation: adapt_receive needs it to know what
        // to fill in, and the field may be absent on old servers (defaults to 0).
        // Use try_from to avoid truncation on values > u32::MAX (same guard as
        // schema parsing in migrate::migrate).
        let raw: Value = serde_json::from_str(text).map_err(|e| io_err(e.to_string()))?;
        let server_protocol = raw["protocol"]
            .as_u64()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
            .unwrap_or(0);
        // Check protocol before typed deserialization: HelloWire uses
        // deny_unknown_fields, so a newer server adding a field would fail
        // serde before check_protocol runs. Checking on the raw protocol value
        // ensures ProtocolTooNew is always surfaced correctly.
        check_protocol(server_protocol)?;
        let adapted = migrate::adapt_receive(server_protocol, "hello", raw).map_err(io_err)?;
        let hello: HelloWire =
            serde_json::from_value(adapted).map_err(|e| io_err(e.to_string()))?;

        let nuance = hello.nuance;

        let store_name = self
            .store_hint
            .clone()
            .or_else(|| hello.stores.into_iter().next().map(|s| s.name))
            .ok_or_else(|| io_err("no stores available in hello response"))?;

        // Protocol 0 servers have a single implicit store and no `store` param.
        // If the configured store_hint names something other than the synthetic
        // implicit store, fail early with a clear message rather than letting
        // adapt_send fail on the first tool call.
        if server_protocol == 0 && store_name != migrate::PROTOCOL_0_IMPLICIT_STORE {
            return Err(io_err(format!(
                "store '{store_name}' not found: protocol 0 server exposes a single implicit \
                 store '{}'; remove the `store` field from the config entry or upgrade the \
                 remote foray",
                migrate::PROTOCOL_0_IMPLICIT_STORE
            )));
        }

        let peer = service.peer().clone();

        // Re-lock to install.  Another task may have connected concurrently;
        // if so, drop the new service and return the existing connection.
        let mut guard = self.conn.lock().await;
        if let Some(conn) = guard.as_ref() {
            return Ok((
                conn.peer.clone(),
                conn.nuance.clone(),
                conn.store_name.clone(),
                conn.protocol,
            ));
        }
        let protocol = server_protocol;
        *guard = Some(Connection {
            _service: service,
            peer: peer.clone(),
            nuance: nuance.clone(),
            store_name: store_name.clone(),
            protocol,
            stderr_buf,
        });

        Ok((peer, nuance, store_name, protocol))
    }

    /// Call an MCP tool and return the parsed typed response.
    ///
    /// Injects `nuance` and `store` into the arguments automatically.
    /// Maps MCP-level errors to `StoreError` variants.
    /// Clears the cached connection on transport failures or nuance mismatches
    /// so the next call reconnects with a fresh subprocess.
    async fn call_mcp<T: DeserializeOwned>(
        &self,
        tool: &'static str,
        args: Value,
    ) -> Result<T, StoreError> {
        let (peer, nuance, store_name, server_protocol) = self.connect().await?;

        // Inject session tokens first so adapt_send sees `store` and can
        // strip it for protocol 0 servers that do not accept that param.
        let mut args = args;
        args["nuance"] = Value::String(nuance);
        args["store"] = Value::String(store_name);
        let args = migrate::adapt_send(server_protocol, tool, args).map_err(io_err)?;

        let arguments = match args {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };

        let params = CallToolRequestParams::new(tool).with_arguments(arguments);

        match peer.call_tool(params).await {
            Ok(result) => {
                let text =
                    first_text(&result.content).ok_or_else(|| io_err("empty tool response"))?;
                let raw: Value = serde_json::from_str(text).map_err(|e| io_err(e.to_string()))?;
                let adapted = migrate::adapt_receive(server_protocol, tool, raw).map_err(io_err)?;
                let value = serde_json::from_value(adapted).map_err(|e| io_err(e.to_string()))?;
                // Clear the stderr buffer only after full success so that
                // parse/adapt failures don't silently discard stderr context.
                // Clone the Arc out first to avoid awaiting while holding the
                // self.conn guard (violates the "don't hold conn across await"
                // convention).
                let stderr_buf = self
                    .conn
                    .lock()
                    .await
                    .as_ref()
                    .map(|c| c.stderr_buf.clone());
                if let Some(buf) = stderr_buf {
                    buf.lock().await.clear();
                }
                Ok(value)
            }
            Err(ServiceError::McpError(e)) => {
                let err = classify_mcp_error(&e);
                // Nuance mismatch means the remote server restarted or was
                // reconfigured — clear the connection so the next call
                // reconnects and fetches a fresh nuance.
                // The server puts the hint in data["hint"]; message is
                // "nuance missing or wrong". Check both for robustness.
                let nuance_mismatch = e
                    .data
                    .as_ref()
                    .and_then(|d| d.get("hint"))
                    .and_then(Value::as_str)
                    .map(is_nuance_mismatch)
                    .unwrap_or(false)
                    || is_nuance_mismatch(&e.message);
                if nuance_mismatch {
                    *self.conn.lock().await = None;
                }
                Err(err)
            }
            Err(e) => {
                // Transport failure — subprocess died or pipe broke.
                // Atomically take conn (clearing it) and extract stderr_buf in
                // one critical section, then read the buffer after releasing
                // the self.conn guard (don't await while holding it).
                let stderr_buf = self.conn.lock().await.take().map(|c| c.stderr_buf);
                let stderr_output = match stderr_buf {
                    Some(buf) => buf.lock().await.clone(),
                    None => String::new(),
                };
                let base = e.to_string();
                let msg = if stderr_output.trim().is_empty() {
                    base
                } else {
                    format!("{base}: {}", stderr_output.trim())
                };
                Err(io_err(msg))
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

/// Returns `Ok(())` if `found` is a protocol version this client supports.
///
/// Returns [`StoreError::ProtocolTooNew`] if the server's protocol is newer
/// than [`migrate::CURRENT_PROTOCOL`]. Old servers that omit `protocol` from
/// their `hello` response are treated as protocol 0 (pre-versioning era).
fn check_protocol(found: u32) -> Result<(), StoreError> {
    if found > migrate::CURRENT_PROTOCOL {
        Err(StoreError::ProtocolTooNew {
            found,
            max: migrate::CURRENT_PROTOCOL,
        })
    } else {
        Ok(())
    }
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
            "protocol_too_new" => {
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
                return StoreError::ProtocolTooNew { found, max };
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
    async fn create(
        &self,
        name: &str,
        title: String,
        meta: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Result<(), StoreError> {
        let mut args = serde_json::json!({ "name": name, "title": title });
        if let Some(meta) = meta {
            args["meta"] = serde_json::to_value(meta).unwrap_or_default();
        }
        let resp: OpenJournalWire = self.call_mcp("open_journal", args).await?;
        // `created: false` means the journal already existed — treat as conflict.
        if !resp.created {
            return Err(StoreError::AlreadyExists(name.to_string()));
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

        let wire: SyncJournalWire = self.call_mcp("sync_journal", args).await?;

        let total = wire.total;

        // Run migrate() on the wire response to handle version mismatches.
        // Construct a JournalFile-shaped Value so the migration chain can
        // inspect and transform the items array.
        let migrate_input = serde_json::json!({
            "schema": wire.schema,
            "name":   wire.name,
            "title":  wire.title,
            "items":  wire.items,
            "meta":   null,
        });
        let migrated = match migrate::migrate(migrate_input) {
            MigrateResult::Current(v) | MigrateResult::Migrated(v) => v,
            MigrateResult::TooNew { found, max } => {
                return Err(StoreError::SchemaTooNew {
                    found,
                    max,
                    origin: crate::store::SchemaOrigin::Wire,
                });
            }
            // migrate_input is always a Value::Object constructed above.
            MigrateResult::Invalid => unreachable!("wire migrate input is always an object"),
        };

        let items: Vec<JournalItem> =
            serde_json::from_value(migrated["items"].clone()).map_err(|e| io_err(e.to_string()))?;

        let journal = JournalFile {
            schema: migrate::CURRENT_SCHEMA,
            name: wire.name,
            title: wire.title,
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
        let resp: SyncJournalWire = self.call_mcp("sync_journal", args).await?;
        Ok(resp.total)
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

        let resp: ListJournalsWire = self.call_mcp("list_journals", args).await?;

        Ok((resp.journals, resp.total))
    }

    async fn exists(&self, name: &str) -> Result<bool, StoreError> {
        // limit: 0 — we only need to know if the journal exists, not its items.
        let args = serde_json::json!({ "name": name, "limit": 0 });
        match self.call_mcp::<SyncJournalWire>("sync_journal", args).await {
            Ok(_) => Ok(true),
            Err(StoreError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn delete(&self, _name: &str) -> Result<(), StoreError> {
        Err(unsupported_err("delete"))
    }

    async fn archive(&self, name: &str) -> Result<(), StoreError> {
        let args = serde_json::json!({ "name": name });
        self.call_mcp::<ArchiveWire>("archive_journal", args)
            .await?;
        Ok(())
    }

    async fn unarchive(&self, name: &str) -> Result<(), StoreError> {
        let args = serde_json::json!({ "name": name });
        self.call_mcp::<UnarchiveWire>("unarchive_journal", args)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── connect: stderr propagation ───────────────────────────────────

    /// A subprocess that writes to stderr and exits non-zero should produce
    /// an I/O error whose message includes the stderr output.
    #[tokio::test]
    #[cfg(unix)]
    async fn connect_failure_includes_stderr() {
        let store = StdioStore::new(
            "sh".to_string(),
            vec![
                "-c".to_string(),
                "echo 'no route to host' >&2; exit 1".to_string(),
            ],
            vec![],
            None,
        );
        let err = store.create("x", "T".into(), None).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no route to host"),
            "expected stderr in error message, got: {msg}"
        );
    }

    // ── check_protocol ────────────────────────────────────────────────

    #[test]
    fn protocol_check_accepts_zero() {
        assert!(check_protocol(0).is_ok());
    }

    #[test]
    fn protocol_check_accepts_current() {
        assert!(check_protocol(migrate::CURRENT_PROTOCOL).is_ok());
    }

    #[test]
    fn protocol_check_rejects_too_new() {
        let err = check_protocol(migrate::CURRENT_PROTOCOL + 1).unwrap_err();
        assert!(
            matches!(err, StoreError::ProtocolTooNew { found, max }
                if found == migrate::CURRENT_PROTOCOL + 1 && max == migrate::CURRENT_PROTOCOL),
            "unexpected error: {err:?}"
        );
    }

    // ── HelloWire deserialization ─────────────────────────────────────

    #[test]
    fn hello_wire_deserializes_fully_formed_response() {
        let s = r#"{"version":"1.0","nuance":"abc","protocol":1,"stores":[{"name":"local","description":"Local store"}]}"#;
        let h: HelloWire = serde_json::from_str(s).unwrap();
        assert_eq!(h.protocol, 1);
        assert_eq!(h.nuance, "abc");
        assert_eq!(h.stores.len(), 1);
        assert_eq!(h.stores[0].name, "local");
    }

    #[test]
    fn hello_wire_rejects_unknown_fields() {
        let s = r#"{"version":"1.0","nuance":"abc","protocol":1,"stores":[],"future_field":"x"}"#;
        assert!(serde_json::from_str::<HelloWire>(s).is_err());
    }

    // ── SyncJournalWire deserialization ───────────────────────────────

    #[test]
    fn sync_journal_wire_deserializes_fully_formed_response() {
        let s = r#"{"schema":1,"name":"j","title":"My Journal","items":[],"added_ids":[],"cursor":0,"total":0}"#;
        let w: SyncJournalWire = serde_json::from_str(s).unwrap();
        assert_eq!(w.schema, 1);
        assert_eq!(w.name, "j");
    }

    #[test]
    fn sync_journal_wire_rejects_unknown_fields() {
        let s = r#"{"schema":1,"name":"j","title":"My Journal","items":[],"added_ids":[],"cursor":0,"total":0,"future_field":42}"#;
        assert!(serde_json::from_str::<SyncJournalWire>(s).is_err());
    }

    // ── classify_mcp_error ────────────────────────────────────────────

    fn make_error(type_val: &str, found: u64, max: u64) -> ErrorData {
        ErrorData::internal_error(
            "test".to_string(),
            Some(serde_json::json!({
                "type": type_val,
                "found": found,
                "max": max,
            })),
        )
    }

    #[test]
    fn classify_protocol_too_new_structured() {
        let e = make_error("protocol_too_new", 5, 1);
        let err = classify_mcp_error(&e);
        assert!(
            matches!(err, StoreError::ProtocolTooNew { found: 5, max: 1 }),
            "unexpected: {err:?}"
        );
    }

    #[test]
    fn classify_schema_too_new_structured() {
        let e = make_error("schema_too_new", 9999, 1);
        let err = classify_mcp_error(&e);
        assert!(
            matches!(
                err,
                StoreError::SchemaTooNew {
                    found: 9999,
                    max: 1,
                    ..
                }
            ),
            "unexpected: {err:?}"
        );
    }
}
