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
}
