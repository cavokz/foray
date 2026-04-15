use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::types::{validate_name, ContextFile, ContextItem, ContextSummary};

#[derive(Debug)]
pub enum StoreError {
    NotFound(String),
    AlreadyExists(String),
    InvalidName(String),
    Io(std::io::Error),
    Parse(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotFound(name) => write!(f, "context not found: '{}'", name),
            StoreError::AlreadyExists(name) => write!(f, "context already exists: '{}'", name),
            StoreError::InvalidName(msg) => write!(f, "{}", msg),
            StoreError::Io(err) => write!(f, "I/O error: {}", err),
            StoreError::Parse(msg) => write!(f, "parse error: {}", msg),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::Io(err)
    }
}

pub trait Store: Send + Sync {
    fn load(&self, name: &str) -> Result<ContextFile, StoreError>;
    fn save(&self, ctx: &ContextFile) -> Result<(), StoreError>;
    fn add_item(&self, name: &str, item: ContextItem) -> Result<(), StoreError>;
    fn remove_item(&self, name: &str, item_id: &str) -> Result<bool, StoreError>;
    fn list(&self) -> Result<Vec<ContextSummary>, StoreError>;
    fn delete(&self, name: &str) -> Result<(), StoreError>;
    fn get_active(&self) -> Result<Option<String>, StoreError>;
    fn set_active(&self, name: &str) -> Result<(), StoreError>;
    fn exists(&self, name: &str) -> Result<bool, StoreError>;
}

pub struct JsonFileStore {
    dir: PathBuf,
}

impl JsonFileStore {
    pub fn new(base_dir: &Path) -> Self {
        Self {
            dir: base_dir.to_path_buf(),
        }
    }

    pub fn default_base_dir() -> PathBuf {
        dirs::home_dir()
            .expect("could not determine home directory")
            .join(".hunch")
    }

    fn ensure_dir(&self) -> Result<(), StoreError> {
        fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    fn context_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{}.json", name))
    }

    fn active_path(&self) -> PathBuf {
        self.dir.join(".active")
    }

    /// Atomic write: write to temp file, fsync, rename.
    fn atomic_write(&self, path: &Path, data: &[u8]) -> Result<(), StoreError> {
        self.ensure_dir()?;
        let tmp_path = self.dir.join(format!(".tmp_{}", uuid::Uuid::new_v4()));
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(data)?;
        file.sync_all()?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

impl Store for JsonFileStore {
    fn load(&self, name: &str) -> Result<ContextFile, StoreError> {
        let path = self.context_path(name);
        if !path.exists() {
            return Err(StoreError::NotFound(name.to_string()));
        }
        let data = fs::read_to_string(&path)?;
        let ctx: ContextFile = serde_json::from_str(&data)
            .map_err(|e| StoreError::Parse(format!("{}: {}", name, e)))?;
        Ok(ctx)
    }

    fn save(&self, ctx: &ContextFile) -> Result<(), StoreError> {
        let data =
            serde_json::to_string_pretty(ctx).map_err(|e| StoreError::Parse(e.to_string()))?;
        let path = self.context_path(&ctx.name);
        self.atomic_write(&path, data.as_bytes())
    }

    fn add_item(&self, name: &str, item: ContextItem) -> Result<(), StoreError> {
        let mut ctx = self.load(name)?;
        ctx.items.push(item);
        ctx.updated_at = Utc::now();
        let data =
            serde_json::to_string_pretty(&ctx).map_err(|e| StoreError::Parse(e.to_string()))?;
        let path = self.context_path(name);
        self.atomic_write(&path, data.as_bytes())
    }

    fn remove_item(&self, name: &str, item_id: &str) -> Result<bool, StoreError> {
        let mut ctx = self.load(name)?;
        let before = ctx.items.len();
        ctx.items.retain(|i| i.id != item_id);
        if ctx.items.len() == before {
            return Ok(false);
        }
        ctx.updated_at = Utc::now();
        let data =
            serde_json::to_string_pretty(&ctx).map_err(|e| StoreError::Parse(e.to_string()))?;
        let path = self.context_path(name);
        self.atomic_write(&path, data.as_bytes())?;
        Ok(true)
    }

    fn list(&self) -> Result<Vec<ContextSummary>, StoreError> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let active = self.get_active()?;
        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "json" {
                    if let Some(stem) = path.file_stem() {
                        let name = stem.to_string_lossy().to_string();
                        // Skip temp files
                        if name.starts_with(".tmp_") {
                            continue;
                        }
                        match self.load(&name) {
                            Ok(ctx) => {
                                summaries.push(ContextSummary {
                                    name: ctx.name.clone(),
                                    item_count: ctx.items.len(),
                                    active: active.as_deref() == Some(&ctx.name),
                                    parent: ctx.parent.clone(),
                                });
                            }
                            Err(_) => continue, // skip unparseable files
                        }
                    }
                }
            }
        }
        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
    }

    fn delete(&self, name: &str) -> Result<(), StoreError> {
        let path = self.context_path(name);
        if !path.exists() {
            return Err(StoreError::NotFound(name.to_string()));
        }
        fs::remove_file(&path)?;
        // If this was the active context, clear .active
        if let Ok(Some(active)) = self.get_active() {
            if active == name {
                let _ = fs::remove_file(self.active_path());
            }
        }
        Ok(())
    }

    fn get_active(&self) -> Result<Option<String>, StoreError> {
        let path = self.active_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        let name = content.trim().to_string();
        if name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    fn set_active(&self, name: &str) -> Result<(), StoreError> {
        self.ensure_dir()?;
        fs::write(self.active_path(), name)?;
        Ok(())
    }

    fn exists(&self, name: &str) -> Result<bool, StoreError> {
        Ok(self.context_path(name).exists())
    }
}

/// Fork (snapshot-copy) a context under a new name.
pub fn fork_context(
    store: &dyn Store,
    source_name: &str,
    new_name: &str,
) -> Result<ContextFile, StoreError> {
    validate_name(new_name).map_err(StoreError::InvalidName)?;

    if store.exists(new_name)? {
        return Err(StoreError::AlreadyExists(new_name.to_string()));
    }

    let source = store.load(source_name)?;
    let now = Utc::now();

    let mut forked = ContextFile {
        _note: Some("Edit this file freely. Each file is self-contained.".to_string()),
        name: new_name.to_string(),
        project: source.project.clone(),
        parent: Some(source_name.to_string()),
        items: source.items.clone(),
        created_at: now,
        updated_at: now,
    };

    // Update item IDs aren't changed — they're copied as-is (snapshot)
    let _ = &mut forked; // suppress unused_mut if we add logic later

    store.save(&forked)?;
    store.set_active(new_name)?;
    Ok(forked)
}

/// Switch to a context (create empty if it doesn't exist).
/// Returns (context_file, was_created).
pub fn switch_context(
    store: &dyn Store,
    name: &str,
    project: &str,
) -> Result<(ContextFile, bool), StoreError> {
    validate_name(name).map_err(StoreError::InvalidName)?;

    let created = if !store.exists(name)? {
        let ctx = ContextFile::new(name.to_string(), project.to_string());
        store.save(&ctx)?;
        true
    } else {
        false
    };

    store.set_active(name)?;
    let ctx = store.load(name)?;
    Ok((ctx, created))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ItemType;
    use tempfile::TempDir;

    fn test_store() -> (TempDir, JsonFileStore) {
        let tmp = TempDir::new().unwrap();
        let store = JsonFileStore::new(tmp.path());
        (tmp, store)
    }

    fn make_item(content: &str) -> ContextItem {
        ContextItem {
            id: uuid::Uuid::new_v4().to_string()[..8].to_string(),
            item_type: ItemType::Finding,
            content: content.to_string(),
            file_ref: None,
            tags: None,
            added_at: Utc::now(),
        }
    }

    #[test]
    fn test_create_and_load() {
        let (_tmp, store) = test_store();
        let ctx = ContextFile::new("my-ctx".to_string(), "test-project".to_string());
        store.save(&ctx).unwrap();
        let loaded = store.load("my-ctx").unwrap();
        assert_eq!(loaded.name, "my-ctx");
        assert_eq!(loaded.project, "test-project");
        assert!(loaded.items.is_empty());
    }

    #[test]
    fn test_load_not_found() {
        let (_tmp, store) = test_store();
        let result = store.load("nonexistent");
        assert!(matches!(result, Err(StoreError::NotFound(_))));
    }

    #[test]
    fn test_add_item() {
        let (_tmp, store) = test_store();
        let ctx = ContextFile::new("my-ctx".to_string(), "test-project".to_string());
        store.save(&ctx).unwrap();

        let item = make_item("found a bug");
        let item_id = item.id.clone();
        store.add_item("my-ctx", item).unwrap();

        let loaded = store.load("my-ctx").unwrap();
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].id, item_id);
        assert_eq!(loaded.items[0].content, "found a bug");
    }

    #[test]
    fn test_remove_item() {
        let (_tmp, store) = test_store();
        let ctx = ContextFile::new("my-ctx".to_string(), "test-project".to_string());
        store.save(&ctx).unwrap();

        let item = make_item("to remove");
        let item_id = item.id.clone();
        store.add_item("my-ctx", item).unwrap();

        assert!(store.remove_item("my-ctx", &item_id).unwrap());
        let loaded = store.load("my-ctx").unwrap();
        assert!(loaded.items.is_empty());

        // Removing again returns false
        assert!(!store.remove_item("my-ctx", &item_id).unwrap());
    }

    #[test]
    fn test_list() {
        let (_tmp, store) = test_store();

        let ctx1 = ContextFile::new("alpha".to_string(), "test-project".to_string());
        store.save(&ctx1).unwrap();

        let ctx2 = ContextFile::new("beta".to_string(), "test-project".to_string());
        store.save(&ctx2).unwrap();

        store.set_active("alpha").unwrap();

        let summaries = store.list().unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].name, "alpha");
        assert!(summaries[0].active);
        assert_eq!(summaries[1].name, "beta");
        assert!(!summaries[1].active);
    }

    #[test]
    fn test_active_context() {
        let (_tmp, store) = test_store();

        assert_eq!(store.get_active().unwrap(), None);

        store.ensure_dir().unwrap();
        store.set_active("my-ctx").unwrap();
        assert_eq!(store.get_active().unwrap(), Some("my-ctx".to_string()));
    }

    #[test]
    fn test_delete() {
        let (_tmp, store) = test_store();
        let ctx = ContextFile::new("to-delete".to_string(), "test-project".to_string());
        store.save(&ctx).unwrap();
        store.set_active("to-delete").unwrap();

        store.delete("to-delete").unwrap();
        assert!(!store.exists("to-delete").unwrap());
        assert_eq!(store.get_active().unwrap(), None);
    }

    #[test]
    fn test_fork_context() {
        let (_tmp, store) = test_store();

        let mut ctx = ContextFile::new("parent".to_string(), "test-project".to_string());
        ctx.items.push(make_item("finding one"));
        ctx.items.push(make_item("finding two"));
        store.save(&ctx).unwrap();
        store.set_active("parent").unwrap();

        let forked = fork_context(&store, "parent", "child").unwrap();
        assert_eq!(forked.name, "child");
        assert_eq!(forked.parent, Some("parent".to_string()));
        assert_eq!(forked.items.len(), 2);

        // Active switched to child
        assert_eq!(store.get_active().unwrap(), Some("child".to_string()));

        // Parent unchanged
        let parent = store.load("parent").unwrap();
        assert_eq!(parent.items.len(), 2);
    }

    #[test]
    fn test_fork_already_exists() {
        let (_tmp, store) = test_store();
        let ctx = ContextFile::new("existing".to_string(), "test-project".to_string());
        store.save(&ctx).unwrap();

        let result = fork_context(&store, "existing", "existing");
        assert!(matches!(result, Err(StoreError::AlreadyExists(_))));
    }

    #[test]
    fn test_switch_creates_new() {
        let (_tmp, store) = test_store();

        let (ctx, created) = switch_context(&store, "new-ctx", "test-project").unwrap();
        assert!(created);
        assert_eq!(ctx.name, "new-ctx");
        assert_eq!(store.get_active().unwrap(), Some("new-ctx".to_string()));
    }

    #[test]
    fn test_switch_existing() {
        let (_tmp, store) = test_store();
        let ctx = ContextFile::new("existing".to_string(), "test-project".to_string());
        store.save(&ctx).unwrap();

        let (ctx, created) = switch_context(&store, "existing", "test-project").unwrap();
        assert!(!created);
        assert_eq!(ctx.name, "existing");
    }

    #[test]
    fn test_invalid_name() {
        let (_tmp, store) = test_store();
        let result = switch_context(&store, "Bad Name!", "test-project");
        assert!(matches!(result, Err(StoreError::InvalidName(_))));
    }

    #[test]
    fn test_snapshot_isolation() {
        // After forking, adding to parent doesn't affect child
        let (_tmp, store) = test_store();

        let mut ctx = ContextFile::new("parent".to_string(), "test-project".to_string());
        ctx.items.push(make_item("original"));
        store.save(&ctx).unwrap();

        fork_context(&store, "parent", "child").unwrap();

        // Add to parent after fork
        store
            .add_item("parent", make_item("added-after-fork"))
            .unwrap();

        let parent = store.load("parent").unwrap();
        assert_eq!(parent.items.len(), 2);

        let child = store.load("child").unwrap();
        assert_eq!(child.items.len(), 1); // Only the original
    }
}
