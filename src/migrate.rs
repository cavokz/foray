//! Schema migration for journal files.
//!
//! The migration chain runs on raw [`serde_json::Value`] **before** serde
//! deserialization, so it can add, remove, or reshape known versioned fields
//! freely (add, remove, reshape). A `Current` or `Migrated` result means the
//! value is a JSON object after that version-aware normalization. It does **not**
//! guarantee that serde deserialization will succeed — callers must still handle
//! errors for missing required fields or unexpected keys rejected by
//! `#[serde(deny_unknown_fields)]`.
//!
//! # Versioning
//! - `schema` absent in the JSON → version 0 (pre-versioning era)
//! - `schema == CURRENT_SCHEMA` → already up to date, no-op
//! - `schema > CURRENT_SCHEMA` → written by a newer foray; return [`MigrateResult::TooNew`]
//! - non-object input → return [`MigrateResult::Invalid`]

use serde_json::{Map, Value};

/// The schema version produced by this build.
pub const CURRENT_SCHEMA: u32 = 1;

/// The wire protocol version produced by this build.
///
/// Tracks envelope-level changes to `SyncJournalResponse` (fields like
/// `cursor`, `added_ids`, etc.) that are independent of `CURRENT_SCHEMA`.
/// `StdioStore` checks this at connect time and returns
/// [`StoreError::ProtocolTooNew`] if the server's protocol is newer.
pub const CURRENT_PROTOCOL: u32 = 1;

/// Synthetic store name injected into `hello` responses from protocol 0
/// servers that do not emit a `stores` list. `adapt_send` strips the `store`
/// param when it matches this value, since protocol 0 servers do not accept
/// a `store` argument.
pub const PROTOCOL_0_IMPLICIT_STORE: &str = "local";

/// Synthetic journal ID inserted into `sync_journal` responses from protocol 0
/// servers that did not emit an `id` field. Visually distinct so it is
/// obvious in logs or debug output that the value was synthesised.
pub const PROTOCOL_0_IMPLICIT_ID: &str = "<unknown>";

/// Result of running [`migrate`].
pub enum MigrateResult {
    /// The value was already at the current schema — returned unchanged.
    Current(Value),
    /// The value was migrated — the caller should rewrite the file.
    Migrated(Value),
    /// The value was written by a newer foray — cannot safely read.
    TooNew { found: u32, max: u32 },
    /// The value is not a JSON object and cannot be migrated.
    Invalid,
}

/// Migrate a raw journal [`Value`] to the current schema.
///
/// Consumes the value and returns a [`MigrateResult`].
///
/// See `doc/compatibility.md` — *Axis 1 — Schema* for detection/resolution
/// scenarios, and *Bumping the schema version* for the checklist to follow
/// when adding a new schema version.
pub fn migrate(value: Value) -> MigrateResult {
    let schema = value
        .get("schema")
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
        .unwrap_or(0);

    if schema > CURRENT_SCHEMA {
        return MigrateResult::TooNew {
            found: schema,
            max: CURRENT_SCHEMA,
        };
    }

    if schema == CURRENT_SCHEMA {
        return MigrateResult::Current(value);
    }

    // Run the migration chain from `schema` up to CURRENT_SCHEMA.
    let mut obj = match value {
        Value::Object(m) => m,
        _ => return MigrateResult::Invalid,
    };

    if schema < 1 {
        obj = v0_to_v1(obj);
    }

    MigrateResult::Migrated(Value::Object(obj))
}

/// Migration 0 → 1: remove `created_at` and `updated_at` from the journal
/// root and from every item, then inject `"schema": 1`.
fn v0_to_v1(mut obj: Map<String, Value>) -> Map<String, Value> {
    obj.remove("created_at");
    obj.remove("updated_at");

    // Strip timestamps from items array too.
    if let Some(Value::Array(items)) = obj.get_mut("items") {
        for item in items.iter_mut() {
            if let Value::Object(item_obj) = item {
                item_obj.remove("created_at");
                item_obj.remove("updated_at");
            }
        }
    }

    obj.insert("schema".to_string(), Value::from(1u32));
    obj
}

/// Adapt outbound request arguments before sending to a remote server with an
/// older protocol version.
///
/// Each `if server_protocol < N` block documents every field that was added or
/// changed at that protocol boundary, stripping or transforming anything the
/// old server does not understand. Wire structs use `deny_unknown_fields`, so
/// an unhandled protocol gap here will surface as a deserialization failure at
/// the call site rather than silent misbehaviour.
///
/// Returns `Err(String)` if a required adaptation cannot be performed.
///
/// See `doc/compatibility.md` — *Protocol 0 (v0.2.0 Servers)* for the
/// current adaptation rules, and *Bumping the protocol version* for the
/// checklist to follow when adding a new protocol version.
pub fn adapt_send(server_protocol: u32, tool: &str, mut args: Value) -> Result<Value, String> {
    // Protocol 0 → 1: several params were added that old servers reject via
    // `deny_unknown_fields`:
    //   all tools:          `store` (protocol 0 servers have a single implicit store)
    //   list_journals:      `archived` (archive feature did not exist)
    //   archive_journal:    entire tool did not exist
    //   unarchive_journal:  entire tool did not exist
    if server_protocol < 1 {
        match tool {
            "archive_journal" | "unarchive_journal" => {
                return Err(format!(
                    "'{tool}' is not supported by protocol 0 server; upgrade the remote foray"
                ));
            }
            _ => {}
        }
        if let Value::Object(ref mut obj) = args {
            // Strip or validate `store`.
            match obj.remove("store") {
                Some(Value::String(ref s)) if s == PROTOCOL_0_IMPLICIT_STORE => {
                    // expected — strip silently
                }
                Some(Value::String(s)) => {
                    return Err(format!(
                        "store '{s}' not found: protocol 0 server exposes a single implicit \
                         store '{PROTOCOL_0_IMPLICIT_STORE}'; remove the `store` field from \
                         the config entry or upgrade the remote foray"
                    ));
                }
                _ => {
                    // absent or non-string — pass through
                }
            }
            if tool == "list_journals" {
                match obj.get("archived").and_then(Value::as_bool) {
                    Some(true) => {
                        return Err("archived journals not supported by protocol 0 server; \
                             upgrade the remote foray"
                            .to_string());
                    }
                    _ => {
                        obj.remove("archived");
                    }
                }
            }
        }
    }
    Ok(args)
}

/// Adapt an inbound response received from a remote server with an older
/// protocol version, normalising it to the current wire shape before typed
/// deserialization.
///
/// Each `if server_protocol < N` block documents every field that was added or
/// changed at that protocol boundary, inserting synthesised defaults for fields
/// that old servers did not emit. Wire structs use `deny_unknown_fields`, so
/// every field the server might send must be explicitly declared in the struct,
/// and every field the struct requires must be inserted here for old servers.
///
/// See `doc/compatibility.md` — *Protocol 0 (v0.2.0 Servers)* for the
/// current adaptation rules, and *Bumping the protocol version* for the
/// checklist to follow when adding a new protocol version.
///
/// Returns `Err(String)` if the response is not a JSON object (adaptation is
/// not possible) or if a required field cannot be synthesised.
pub fn adapt_receive(
    server_protocol: u32,
    tool: &str,
    mut response: Value,
) -> Result<Value, String> {
    // Protocol 0 → 1: the following fields were added in this transition:
    //   hello:            `protocol`, `stores`  (version was already present)
    //   sync_journal:     `schema`, `id`
    //   open_journal:     `name`, `title`, `item_count`
    //   list_journals:    `limit`, `offset`
    //   archive_journal:  `archived`
    //   unarchive_journal:`unarchived`
    if server_protocol < 1 {
        let obj = response
            .as_object_mut()
            .ok_or_else(|| format!("adapt_receive({tool}): response is not a JSON object"))?;
        match tool {
            "hello" => {
                obj.entry("version")
                    .or_insert_with(|| Value::String(String::new()));
                obj.entry("protocol").or_insert_with(|| Value::from(0u32));
                // Synthesise a single implicit store so the client can select
                // it as `store_name`. `adapt_send` strips it before sending.
                obj.entry("stores").or_insert_with(|| {
                    serde_json::json!([
                        {"name": PROTOCOL_0_IMPLICIT_STORE,
                         "description": "implicit store (protocol 0 server)"}
                    ])
                });
            }
            "sync_journal" => {
                obj.entry("schema").or_insert_with(|| Value::from(0u32));
                obj.entry("id")
                    .or_insert_with(|| Value::String(PROTOCOL_0_IMPLICIT_ID.to_string()));
            }
            "open_journal" => {
                obj.entry("name")
                    .or_insert_with(|| Value::String(String::new()));
                obj.entry("title").or_insert(Value::Null);
                obj.entry("item_count")
                    .or_insert_with(|| Value::from(0usize));
            }
            "list_journals" => {
                obj.entry("limit").or_insert(Value::Null);
                obj.entry("offset").or_insert(Value::Null);
            }
            "archive_journal" => {
                obj.entry("archived")
                    .or_insert_with(|| Value::String(String::new()));
            }
            "unarchive_journal" => {
                obj.entry("unarchived")
                    .or_insert_with(|| Value::String(String::new()));
            }
            _ => {}
        }
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn migrate_current() {
        let v = json!({ "schema": 1, "id": "abc", "name": "test", "items": [] });
        let original = v.clone();
        match migrate(v) {
            MigrateResult::Current(out) => assert_eq!(out, original),
            _ => panic!("expected Current"),
        }
    }

    #[test]
    fn migrate_v0_removes_timestamps() {
        let v = json!({
            "id": "abc",
            "name": "test",
            "items": [
                {
                    "id": "x",
                    "type": "note",
                    "content": "hi",
                    "added_at": "2026-01-01T00:00:00Z",
                    "created_at": "2026-01-01T00:00:00Z"
                }
            ],
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z"
        });
        match migrate(v) {
            MigrateResult::Migrated(out) => {
                assert!(
                    out.get("created_at").is_none(),
                    "created_at should be removed"
                );
                assert!(
                    out.get("updated_at").is_none(),
                    "updated_at should be removed"
                );
                assert_eq!(out["schema"], json!(CURRENT_SCHEMA));
                let item = &out["items"][0];
                assert!(
                    item.get("created_at").is_none(),
                    "item created_at should be removed"
                );
                // added_at on items is kept
                assert!(
                    item.get("added_at").is_some(),
                    "item added_at should be preserved"
                );
            }
            _ => panic!("expected Migrated"),
        }
    }

    #[test]
    fn migrate_v0_no_timestamps() {
        // v0 file that never had the timestamp fields — should still get current schema
        let v = json!({ "id": "abc", "name": "test", "items": [] });
        match migrate(v) {
            MigrateResult::Migrated(out) => {
                assert_eq!(out["schema"], json!(CURRENT_SCHEMA));
            }
            _ => panic!("expected Migrated"),
        }
    }

    #[test]
    fn migrate_schema_overflow() {
        // Values > u32::MAX must not bypass the TooNew guard via truncation.
        let v =
            json!({ "schema": (u32::MAX as u64) + 1, "id": "abc", "name": "test", "items": [] });
        match migrate(v) {
            MigrateResult::TooNew { found, max } => {
                assert_eq!(found, u32::MAX);
                assert_eq!(max, CURRENT_SCHEMA);
            }
            _ => panic!("expected TooNew"),
        }
    }

    #[test]
    fn migrate_too_new() {
        let v = json!({ "schema": 9999, "id": "abc", "name": "test", "items": [] });
        match migrate(v) {
            MigrateResult::TooNew { found, max } => {
                assert_eq!(found, 9999);
                assert_eq!(max, CURRENT_SCHEMA);
            }
            _ => panic!("expected TooNew"),
        }
    }

    #[test]
    fn migrate_non_object_returns_invalid() {
        for v in [json!(null), json!(42), json!("string"), json!([1, 2, 3])] {
            assert!(
                matches!(migrate(v), MigrateResult::Invalid),
                "expected Invalid for non-object input"
            );
        }
    }

    // ── adapt_send ────────────────────────────────────────────────────

    #[test]
    fn adapt_send_strips_store_and_archived_false_for_protocol_0() {
        let args = json!({ "store": PROTOCOL_0_IMPLICIT_STORE, "limit": 10, "archived": false });
        let result = adapt_send(0, "list_journals", args).unwrap();
        assert!(result.get("store").is_none(), "store should be stripped");
        assert!(
            result.get("archived").is_none(),
            "archived false should be stripped"
        );
        assert_eq!(result["limit"], json!(10));
    }

    #[test]
    fn adapt_send_errors_on_archived_true_for_protocol_0() {
        let args = json!({ "store": PROTOCOL_0_IMPLICIT_STORE, "archived": true });
        let err = adapt_send(0, "list_journals", args).unwrap_err();
        assert!(
            err.contains("archived journals not supported"),
            "got: {err}"
        );
    }

    #[test]
    fn adapt_send_errors_on_unknown_store_for_protocol_0() {
        let args = json!({ "store": "remote", "name": "j" });
        let err = adapt_send(0, "open_journal", args).unwrap_err();
        assert!(err.contains("store 'remote' not found"), "got: {err}");
    }

    #[test]
    fn adapt_send_errors_on_archive_journal_for_protocol_0() {
        let args = json!({ "store": PROTOCOL_0_IMPLICIT_STORE, "name": "j" });
        let err = adapt_send(0, "archive_journal", args).unwrap_err();
        assert!(err.contains("archive_journal"), "got: {err}");
        assert!(err.contains("not supported"), "got: {err}");
    }

    #[test]
    fn adapt_send_errors_on_unarchive_journal_for_protocol_0() {
        let args = json!({ "store": PROTOCOL_0_IMPLICIT_STORE, "name": "j" });
        let err = adapt_send(0, "unarchive_journal", args).unwrap_err();
        assert!(err.contains("unarchive_journal"), "got: {err}");
        assert!(err.contains("not supported"), "got: {err}");
    }

    #[test]
    fn adapt_send_keeps_archived_for_protocol_1() {
        let args = json!({ "limit": 10, "archived": true });
        let result = adapt_send(1, "list_journals", args).unwrap();
        assert_eq!(result["archived"], json!(true));
    }

    #[test]
    fn adapt_send_noop_for_protocol_0_open_journal_with_implicit_store() {
        let args = json!({ "store": PROTOCOL_0_IMPLICIT_STORE, "name": "foo" });
        let result = adapt_send(0, "open_journal", args).unwrap();
        assert!(result.get("store").is_none(), "store should be stripped");
        assert_eq!(result["name"], json!("foo"));
    }

    // ── adapt_receive ─────────────────────────────────────────────────

    #[test]
    fn adapt_receive_hello_inserts_synthetic_store_for_protocol_0() {
        let raw = json!({ "version": "0.2.0", "nuance": "abc" });
        let result = adapt_receive(0, "hello", raw).unwrap();
        assert_eq!(result["protocol"], json!(0));
        assert_eq!(
            result["stores"][0]["name"],
            json!(PROTOCOL_0_IMPLICIT_STORE)
        );
        assert!(result["stores"][0]["description"].is_string());
        assert_eq!(result["nuance"], json!("abc"));
    }

    #[test]
    fn adapt_receive_hello_preserves_existing_stores_for_protocol_0() {
        // If server somehow sends stores already, do not overwrite them.
        let raw = json!({ "nuance": "abc", "stores": [{"name": "x", "description": "y"}] });
        let result = adapt_receive(0, "hello", raw).unwrap();
        assert_eq!(result["stores"][0]["name"], json!("x"));
    }

    #[test]
    fn adapt_receive_hello_passthrough_for_protocol_1() {
        let raw = json!({
            "version": "1.0",
            "nuance": "abc",
            "protocol": 1,
            "stores": [{"name": "local", "description": "Local store"}]
        });
        let result = adapt_receive(1, "hello", raw.clone()).unwrap();
        assert_eq!(result, raw);
    }

    #[test]
    fn adapt_receive_sync_journal_inserts_id_and_schema_for_protocol_0() {
        // v0.2.0 sync_journal response has no `id` or `schema`.
        let raw = json!({
            "name": "j", "title": null,
            "items": [], "added_ids": [], "cursor": 0, "total": 0
        });
        let result = adapt_receive(0, "sync_journal", raw).unwrap();
        assert_eq!(result["schema"], json!(0));
        assert_eq!(result["id"], json!(PROTOCOL_0_IMPLICIT_ID));
    }

    #[test]
    fn adapt_receive_list_journals_inserts_pagination_for_protocol_0() {
        let raw = json!({ "journals": [], "total": 0 });
        let result = adapt_receive(0, "list_journals", raw).unwrap();
        assert!(result["limit"].is_null());
        assert!(result["offset"].is_null());
    }

    #[test]
    fn adapt_receive_unknown_tool_is_noop() {
        let raw = json!({ "foo": "bar" });
        let result = adapt_receive(0, "some_future_tool", raw.clone()).unwrap();
        assert_eq!(result, raw);
    }

    #[test]
    fn adapt_receive_non_object_returns_err() {
        let raw = json!([1, 2, 3]);
        assert!(adapt_receive(0, "hello", raw).is_err());
    }
}
