use crate::store::{JsonFileStore, Store, StoreError};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

// ── Config file types ───────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    stores: BTreeMap<String, RawStoreConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, tag = "type")]
enum RawStoreConfig {
    #[serde(rename = "json_file")]
    JsonFile {
        /// Absolute path to the journals directory.
        path: PathBuf,
        /// Human-readable description — required; helps the model suggest the right store.
        description: String,
    },
}

// ── StoreRegistry ───────────────────────────────────────────────────

/// A named collection of stores, ordered by store name (BTreeMap key order) for a stable, deterministic nuance.
#[derive(Clone)]
pub struct StoreRegistry {
    stores: Vec<StoreEntry>,
    /// Stable fingerprint derived from the config (sorted store names+paths).
    pub nuance: String,
}

#[derive(Clone)]
pub struct StoreEntry {
    pub name: String,
    pub description: String,
    pub store: Arc<dyn Store>,
}

impl StoreRegistry {
    /// Build a registry from `~/.foray/config.toml`.
    /// Falls back to a single implicit `local` store if the file is absent or has no stores.
    pub fn load() -> Result<Self, StoreError> {
        Self::load_from(&config_path()?)
    }

    fn load_from(config_path: &std::path::Path) -> Result<Self, StoreError> {
        let raw = if config_path.exists() {
            let text = std::fs::read_to_string(config_path).map_err(StoreError::Io)?;
            toml::from_str::<RawConfig>(&text).map_err(|e| {
                StoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("config parse error: {e}"),
                ))
            })?
        } else {
            RawConfig::default()
        };

        if raw.stores.is_empty() {
            return Self::implicit_local();
        }

        let mut stores = Vec::with_capacity(raw.stores.len());
        let mut fingerprints = Vec::with_capacity(raw.stores.len());
        // BTreeMap iteration is sorted by key → deterministic nuance
        for (name, entry) in raw.stores {
            let (store, description): (Arc<dyn Store>, String) = match entry {
                RawStoreConfig::JsonFile { path, description } => {
                    if !path.is_absolute() {
                        return Err(StoreError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!(
                                "store '{name}': path must be absolute, got '{}'",
                                path.display()
                            ),
                        )));
                    }
                    fingerprints.push(format!("{}={}", name, path.display()));
                    (Arc::new(JsonFileStore::new(path)), description)
                }
            };
            stores.push(StoreEntry {
                name,
                description,
                store,
            });
        }

        let nuance = compute_nuance(&fingerprints);
        Ok(Self { stores, nuance })
    }

    /// Single implicit `local` `JsonFileStore` at `~/.foray/journals/`.
    pub fn implicit_local() -> Result<Self, StoreError> {
        let base_dir = JsonFileStore::default_dir()?;
        let path_str = base_dir.display().to_string();
        let store: Arc<dyn Store> = Arc::new(JsonFileStore::new(base_dir));
        let nuance = compute_nuance(&[format!("local={path_str}")]);
        Ok(Self {
            stores: vec![StoreEntry {
                name: "local".to_string(),
                description: "Default local journal store".to_string(),
                store,
            }],
            nuance,
        })
    }

    /// Construct a single-store registry backed by `base_dir`.
    /// For use in tests — avoids depending on the user's home directory.
    #[cfg(test)]
    pub fn for_test(base_dir: std::path::PathBuf) -> Self {
        let path_str = base_dir.display().to_string();
        let store: Arc<dyn Store> = Arc::new(JsonFileStore::new(base_dir));
        let nuance = compute_nuance(&[format!("local={path_str}")]);
        Self {
            stores: vec![StoreEntry {
                name: "local".to_string(),
                description: "Test store".to_string(),
                store,
            }],
            nuance,
        }
    }

    /// Look up a store by name. Returns `None` if not found.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Store>> {
        self.stores
            .iter()
            .find(|e| e.name == name)
            .map(|e| &e.store)
    }

    /// The default store (first in registry).
    pub fn default_store(&self) -> &Arc<dyn Store> {
        &self.stores[0].store
    }

    /// Default store name.
    pub fn default_store_name(&self) -> &str {
        &self.stores[0].name
    }

    /// All store entries (name + description), for listing.
    pub fn entries(&self) -> &[StoreEntry] {
        &self.stores
    }

    /// All store names joined, for hint messages.
    pub fn names_hint(&self) -> String {
        self.stores
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn config_path() -> Result<PathBuf, StoreError> {
    Ok(dirs::home_dir()
        .ok_or_else(|| {
            StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot determine home directory",
            ))
        })?
        .join(".foray")
        .join("config.toml"))
}

fn compute_nuance(fingerprints: &[String]) -> String {
    // Sort internally so callers don't need to guarantee order.
    // Using a simple FNV-1a 64-bit hash — no crypto needed, just stability.
    let mut sorted = fingerprints.to_vec();
    sorted.sort_unstable();
    let mut hash: u64 = 0xcbf29ce484222325;
    for s in &sorted {
        for byte in s.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        // separator between entries
        hash ^= b'|' as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nuance_is_stable() {
        let a = compute_nuance(&["local=/home/user/.foray/journals".to_string()]);
        let b = compute_nuance(&["local=/home/user/.foray/journals".to_string()]);
        assert_eq!(a, b);
    }

    #[test]
    fn nuance_differs_on_change() {
        let a = compute_nuance(&["local=/home/user/.foray/journals".to_string()]);
        let b = compute_nuance(&["work=/home/user/.foray/work".to_string()]);
        assert_ne!(a, b);
    }

    #[test]
    fn implicit_local_builds() {
        // Just verify it doesn't panic; actual path depends on env.
        let _ = StoreRegistry::implicit_local();
    }

    #[test]
    fn load_from_rejects_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            "[stores.work]\ntype = \"json_file\"\npath = \"relative/path\"\ndescription = \"Work\"\n",
        )
        .unwrap();
        let err = StoreRegistry::load_from(&config_path).err().unwrap();
        let msg = err.to_string();
        assert!(msg.contains("path must be absolute"), "error was: {msg}");
    }

    #[test]
    fn load_from_rejects_missing_description() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let path_str = dir.path().join("journals").display().to_string();
        std::fs::write(
            &config_path,
            format!("[stores.work]\ntype = \"json_file\"\npath = '{path_str}'\n"),
        )
        .unwrap();
        let err = StoreRegistry::load_from(&config_path).err().unwrap();
        assert!(err.to_string().contains("description"), "error was: {err}");
    }

    #[test]
    fn load_from_accepts_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let journals_dir = dir.path().join("journals");
        let config_path = dir.path().join("config.toml");
        // Use TOML literal strings (single quotes) so backslashes in Windows
        // paths are not interpreted as TOML escape sequences.
        let path_str = journals_dir.display().to_string();
        std::fs::write(
            &config_path,
            format!("[stores.work]\ntype = \"json_file\"\npath = '{path_str}'\ndescription = \"Work journals\"\n"),
        )
        .unwrap();
        let registry = StoreRegistry::load_from(&config_path).unwrap();
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].name, "work");
        assert_eq!(registry.entries()[0].description, "Work journals");
    }

    #[test]
    fn load_from_absent_config_falls_back_to_implicit_local() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml"); // does not exist
        let registry = StoreRegistry::load_from(&config_path).unwrap();
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].name, "local");
    }

    #[test]
    fn load_from_empty_stores_falls_back_to_implicit_local() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "# no stores section\n").unwrap();
        let registry = StoreRegistry::load_from(&config_path).unwrap();
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].name, "local");
    }

    #[test]
    fn load_from_multi_store_config() {
        let dir = tempfile::tempdir().unwrap();
        let a_path = dir.path().join("a").display().to_string();
        let b_path = dir.path().join("b").display().to_string();
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                "[stores.alpha]\ntype = \"json_file\"\npath = '{a_path}'\ndescription = \"Alpha\"\n\
                 [stores.beta]\ntype = \"json_file\"\npath = '{b_path}'\ndescription = \"Beta\"\n"
            ),
        )
        .unwrap();
        let registry = StoreRegistry::load_from(&config_path).unwrap();
        assert_eq!(registry.entries().len(), 2);
        // BTreeMap order: alpha < beta
        assert_eq!(registry.entries()[0].name, "alpha");
        assert_eq!(registry.entries()[1].name, "beta");
    }

    #[test]
    fn get_returns_none_for_unknown_name() {
        let registry = StoreRegistry::implicit_local().unwrap();
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn get_returns_entry_for_known_name() {
        let registry = StoreRegistry::implicit_local().unwrap();
        assert!(registry.get("local").is_some());
    }
}
