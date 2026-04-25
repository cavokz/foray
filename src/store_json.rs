use crate::migrate::{self, MigrateResult};
use crate::store::{Store, StoreError};
use crate::types::{JournalFile, JournalItem, JournalSummary, Pagination};
use async_trait::async_trait;
use fs2::FileExt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Flat-file JSON store at `~/.foray/journals/`.
pub struct JsonFileStore {
    base_dir: PathBuf,
}

impl JsonFileStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    pub fn default_dir() -> Result<PathBuf, StoreError> {
        Ok(home::home_dir()
            .ok_or_else(|| {
                StoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "cannot determine home directory",
                ))
            })?
            .join(".foray")
            .join("journals"))
    }

    fn journal_path(&self, name: &str) -> PathBuf {
        self.base_dir.join(format!("{name}.json"))
    }

    fn archive_path(&self, name: &str) -> PathBuf {
        self.base_dir.join("archive").join(format!("{name}.json"))
    }

    fn lock_path(&self, name: &str) -> PathBuf {
        self.base_dir.join(format!("{name}.lock"))
    }

    fn with_lock(&self, name: &str) -> Result<fs::File, StoreError> {
        if let Some(parent) = self.lock_path(name).parent() {
            fs::create_dir_all(parent)?;
        }
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(self.lock_path(name))?;
        lock_file.lock_exclusive()?;
        Ok(lock_file)
    }

    fn find(&self, name: &str) -> Option<(PathBuf, bool)> {
        let active = self.journal_path(name);
        if active.exists() {
            return Some((active, false));
        }
        let archived = self.archive_path(name);
        if archived.exists() {
            return Some((archived, true));
        }
        None
    }

    pub fn read_journal(&self, path: &Path) -> Result<JournalFile, StoreError> {
        let data = fs::read_to_string(path)?;
        let raw: serde_json::Value = serde_json::from_str(&data)?;
        let value = match migrate::migrate(raw) {
            MigrateResult::Current(v) | MigrateResult::Migrated(v) => v,
            MigrateResult::TooNew { found, max } => {
                return Err(StoreError::SchemaTooNew {
                    found,
                    max,
                    origin: crate::store::SchemaOrigin::Storage,
                });
            }
            MigrateResult::Invalid => {
                return Err(StoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "journal file is not a JSON object",
                )));
            }
        };
        Ok(serde_json::from_value(value)?)
    }

    fn write_journal(&self, path: &Path, journal: &JournalFile) -> Result<(), StoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(journal)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(data.as_bytes())?;
        tmp.write_all(b"\n")?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(std::io::Error::other)?;
        Ok(())
    }

    fn list_dir(&self, dir: &Path) -> Result<Vec<JournalFile>, StoreError> {
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut journals = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") && path.is_file() {
                match self.read_journal(&path) {
                    Ok(j) => journals.push(j),
                    // Forward-compat: propagate schema-too-new so the caller
                    // knows a newer-version journal exists rather than silently
                    // omitting it from results.
                    Err(e @ StoreError::SchemaTooNew { .. }) => return Err(e),
                    // Other errors (corrupt JSON, permission denied, etc.) are
                    // skipped so one bad file doesn't break the entire listing.
                    Err(_) => continue,
                }
            }
        }
        journals.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(journals)
    }
}

#[async_trait]
impl Store for JsonFileStore {
    async fn load(
        &self,
        name: &str,
        pagination: &Pagination,
    ) -> Result<(JournalFile, usize), StoreError> {
        let (path, _) = self
            .find(name)
            .ok_or_else(|| StoreError::NotFound(name.into()))?;
        let mut journal = self.read_journal(&path)?;
        let total = journal.items.len();
        let (items, _) = pagination.apply(&journal.items);
        journal.items = items;
        Ok((journal, total))
    }

    async fn create(
        &self,
        name: &str,
        title: Option<String>,
        meta: Option<std::collections::HashMap<String, serde_json::Value>>,
    ) -> Result<(), StoreError> {
        let path = self.journal_path(name);
        if path.exists() || self.archive_path(name).exists() {
            return Err(StoreError::AlreadyExists(name.to_string()));
        }
        let journal = JournalFile::new(name, title, meta);
        self.write_journal(&path, &journal)
    }

    async fn add_items(&self, name: &str, items: Vec<JournalItem>) -> Result<usize, StoreError> {
        let (path, is_archived) = self
            .find(name)
            .ok_or_else(|| StoreError::NotFound(name.into()))?;
        if is_archived {
            return Err(StoreError::Archived(name.into()));
        }
        let _lock = self.with_lock(name)?;
        let mut journal = self.read_journal(&path)?;
        journal.items.extend(items);
        let count = journal.items.len();
        self.write_journal(&path, &journal)?;
        Ok(count)
    }

    async fn list(
        &self,
        pagination: &Pagination,
        archived: bool,
    ) -> Result<(Vec<JournalSummary>, usize), StoreError> {
        let dir = if archived {
            self.base_dir.join("archive")
        } else {
            self.base_dir.clone()
        };
        let journals = self.list_dir(&dir)?;
        let summaries: Vec<JournalSummary> = journals.iter().map(JournalSummary::from).collect();
        let (page, total) = pagination.apply(&summaries);
        Ok((page, total))
    }

    async fn delete(&self, name: &str) -> Result<(), StoreError> {
        let (path, _) = self
            .find(name)
            .ok_or_else(|| StoreError::NotFound(name.into()))?;
        fs::remove_file(path)?;
        Ok(())
    }

    async fn exists(&self, name: &str) -> Result<bool, StoreError> {
        Ok(self.find(name).is_some())
    }

    async fn archive(&self, name: &str) -> Result<(), StoreError> {
        let active = self.journal_path(name);
        if !active.exists() {
            if self.archive_path(name).exists() {
                return Err(StoreError::Archived(name.into()));
            }
            return Err(StoreError::NotFound(name.into()));
        }
        let dest = self.archive_path(name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(active, dest)?;
        Ok(())
    }

    async fn unarchive(&self, name: &str) -> Result<(), StoreError> {
        let archived = self.archive_path(name);
        if !archived.exists() {
            if self.journal_path(name).exists() {
                return Ok(());
            }
            return Err(StoreError::NotFound(name.into()));
        }
        let dest = self.journal_path(name);
        fs::rename(archived, dest)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ItemType, item_id};
    use chrono::Utc;

    fn make_store() -> (JsonFileStore, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = JsonFileStore::new(dir.path().to_path_buf());
        (store, dir)
    }

    fn make_item(content: &str) -> JournalItem {
        JournalItem {
            id: item_id(),
            item_type: ItemType::Finding,
            content: content.into(),
            tags: None,
            added_at: Utc::now(),
            meta: None,
        }
    }

    #[tokio::test]
    async fn create_and_load() {
        let (store, _dir) = make_store();
        let journal = JournalFile::new("my-ctx", Some("Test".into()), None);
        store
            .create(&journal.name, journal.title.clone(), journal.meta.clone())
            .await
            .unwrap();
        let (loaded, total) = store.load("my-ctx", &Pagination::default()).await.unwrap();
        assert_eq!(loaded.name, "my-ctx");
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn create_duplicate_errors() {
        let (store, _dir) = make_store();
        store.create("dup", Some("A".into()), None).await.unwrap();
        let result = store.create("dup", Some("B".into()), None).await;
        assert!(matches!(result, Err(StoreError::AlreadyExists(_))));
    }

    #[tokio::test]
    async fn load_not_found() {
        let (store, _dir) = make_store();
        let result = store.load("nonexistent", &Pagination::default()).await;
        assert!(matches!(result, Err(StoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn add_item_and_load() {
        let (store, _dir) = make_store();
        store
            .create("my-ctx", Some("T".into()), None)
            .await
            .unwrap();
        let item = make_item("found a bug");
        store.add_items("my-ctx", vec![item]).await.unwrap();
        let (loaded, total) = store.load("my-ctx", &Pagination::default()).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(loaded.items[0].content, "found a bug");
    }

    #[tokio::test]
    async fn add_item_not_found() {
        let (store, _dir) = make_store();
        let result = store.add_items("nope", vec![make_item("x")]).await;
        assert!(matches!(result, Err(StoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_journals() {
        let (store, _dir) = make_store();
        store.create("alpha", Some("A".into()), None).await.unwrap();
        store.create("beta", Some("B".into()), None).await.unwrap();
        let (summaries, total) = store.list(&Pagination::default(), false).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(summaries[0].name, "alpha");
        assert_eq!(summaries[1].name, "beta");
    }

    #[tokio::test]
    async fn list_pagination() {
        let (store, _dir) = make_store();
        for name in ["a", "b", "c", "d"] {
            store.create(name, Some(name.into()), None).await.unwrap();
        }
        let p = Pagination {
            limit: Some(2),
            offset: Some(1),
        };
        let (page, total) = store.list(&p, false).await.unwrap();
        assert_eq!(total, 4);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].name, "b");
        assert_eq!(page[1].name, "c");
    }

    #[tokio::test]
    async fn delete_journal() {
        let (store, _dir) = make_store();
        store
            .create("to-delete", Some("D".into()), None)
            .await
            .unwrap();
        store.delete("to-delete").await.unwrap();
        assert!(!store.exists("to-delete").await.unwrap());
    }

    #[tokio::test]
    async fn archive_and_unarchive() {
        let (store, _dir) = make_store();
        store
            .create("arch-test", Some("A".into()), None)
            .await
            .unwrap();

        store.archive("arch-test").await.unwrap();

        let (loaded, _) = store
            .load("arch-test", &Pagination::default())
            .await
            .unwrap();
        assert_eq!(loaded.name, "arch-test");
        assert!(matches!(
            store.add_items("arch-test", vec![make_item("x")]).await,
            Err(StoreError::Archived(_))
        ));
        let (archived_list, _) = store.list(&Pagination::default(), true).await.unwrap();
        assert_eq!(archived_list.len(), 1);
        let (active, _) = store.list(&Pagination::default(), false).await.unwrap();
        assert_eq!(active.len(), 0);

        store.unarchive("arch-test").await.unwrap();
        let (active, _) = store.list(&Pagination::default(), false).await.unwrap();
        assert_eq!(active.len(), 1);
    }

    #[tokio::test]
    async fn archive_already_archived_errors() {
        let (store, _dir) = make_store();
        store
            .create("to-archive", Some("A".into()), None)
            .await
            .unwrap();
        store.archive("to-archive").await.unwrap();
        assert!(matches!(
            store.archive("to-archive").await,
            Err(StoreError::Archived(_))
        ));
    }

    #[tokio::test]
    async fn archive_not_found_errors() {
        let (store, _dir) = make_store();
        assert!(matches!(
            store.archive("missing").await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn unarchive_already_active_is_noop() {
        let (store, _dir) = make_store();
        store
            .create("active", Some("A".into()), None)
            .await
            .unwrap();
        store.unarchive("active").await.unwrap();
        // still active
        let (active, _) = store.list(&Pagination::default(), false).await.unwrap();
        assert_eq!(active.len(), 1);
    }

    #[tokio::test]
    async fn unarchive_not_found_errors() {
        let (store, _dir) = make_store();
        assert!(matches!(
            store.unarchive("missing").await,
            Err(StoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn read_journal_migrates_v0() {
        let (store, dir) = make_store();
        // Write a raw schema-0 file (no schema field, has created_at/updated_at).
        let path = dir.path().join("legacy.json");
        let raw = serde_json::json!({
            "_note": "old file",
            "id": "aaaaa-bbbbb-ccccc",
            "name": "legacy",
            "items": [
                {
                    "id": "xxxx-xxxx-xxxx-xxxx",
                    "type": "note",
                    "content": "old note",
                    "added_at": "2026-01-01T00:00:00Z"
                }
            ],
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z"
        });
        std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        let journal = store.read_journal(&path).unwrap();

        // Migration should produce a journal at the current schema version.
        assert_eq!(journal.schema, migrate::CURRENT_SCHEMA);
        assert_eq!(journal.name, "legacy");
        assert_eq!(journal.items.len(), 1);
        assert_eq!(journal.items[0].content, "old note");

        // File on disk is NOT rewritten by read_journal — migration is lazy.
        // The old fields are still present until the next add_items write.
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(
            on_disk.get("schema").is_none(),
            "file not yet healed — schema should still be absent"
        );
    }

    #[tokio::test]
    async fn add_items_heals_v0_journal() {
        let (store, dir) = make_store();
        // Write a raw schema-0 file directly into the store directory.
        let path = dir.path().join("legacy.json");
        let raw = serde_json::json!({
            "id": "aaaaa-bbbbb-ccccc",
            "name": "legacy",
            "items": [],
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z"
        });
        std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        // add_items holds the lock and rewrites the file — this is the heal path.
        store
            .add_items("legacy", vec![make_item("new item")])
            .await
            .unwrap();

        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            on_disk["schema"],
            serde_json::json!(migrate::CURRENT_SCHEMA)
        );
        assert!(
            on_disk.get("created_at").is_none(),
            "created_at should be gone"
        );
        assert!(
            on_disk.get("updated_at").is_none(),
            "updated_at should be gone"
        );
        assert_eq!(on_disk["items"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn read_journal_too_new() {
        let (store, dir) = make_store();
        let path = dir.path().join("future.json");
        let raw = serde_json::json!({
            "schema": 9999,
            "id": "aaaaa-bbbbb-ccccc",
            "name": "future",
            "items": []
        });
        std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        let err = store.read_journal(&path).unwrap_err();
        assert!(
            matches!(
                err,
                StoreError::SchemaTooNew {
                    found: 9999,
                    max: migrate::CURRENT_SCHEMA,
                    origin: crate::store::SchemaOrigin::Storage,
                }
            ),
            "expected SchemaTooNew, got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_propagates_schema_too_new() {
        // A journal with a schema newer than CURRENT_SCHEMA must not be
        // silently omitted from list() — it should propagate SchemaTooNew
        // so the caller knows a newer-version file exists.
        let (store, dir) = make_store();
        // Create a normal journal first so there's something in the directory.
        store
            .create("normal", Some("Normal".into()), None)
            .await
            .unwrap();
        // Drop a future-schema file directly into the journals directory.
        let path = dir.path().join("future.json");
        let raw = serde_json::json!({
            "schema": 9999,
            "id": "aaaaa-bbbbb-ccccc",
            "name": "future",
            "items": []
        });
        std::fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        let err = store.list(&Pagination::default(), false).await.unwrap_err();
        assert!(
            matches!(err, StoreError::SchemaTooNew { found: 9999, .. }),
            "expected SchemaTooNew, got {err:?}"
        );
    }

    #[tokio::test]
    async fn load_paginated_items() {
        let (store, _dir) = make_store();
        store.create("pag", Some("P".into()), None).await.unwrap();
        for i in 0..5 {
            store
                .add_items("pag", vec![make_item(&format!("item-{i}"))])
                .await
                .unwrap();
        }
        let p = Pagination {
            limit: Some(2),
            offset: Some(1),
        };
        let (journal, total) = store.load("pag", &p).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(journal.items.len(), 2);
        assert_eq!(journal.items[0].content, "item-1");
        assert_eq!(journal.items[1].content, "item-2");
    }
}
