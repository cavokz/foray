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
        /// Optional human-readable description.
        #[serde(default)]
        description: Option<String>,
    },
}

// ── StoreRegistry ───────────────────────────────────────────────────

/// A named collection of stores, keyed by store name.
/// Preserves insertion order (BTreeMap gives sorted order = stable nuance).
#[derive(Clone)]
pub struct StoreRegistry {
    stores: Vec<StoreEntry>,
    /// Stable fingerprint derived from the config (sorted store names+paths).
    pub nuance: String,
}

#[derive(Clone)]
pub struct StoreEntry {
    pub name: String,
    pub description: Option<String>,
    pub store: Arc<dyn Store>,
}

impl StoreRegistry {
    /// Build a registry from `~/.foray/config.toml`.
    /// Falls back to a single implicit `local` store if the file is absent or has no stores.
    pub fn load() -> Result<Self, StoreError> {
        let config_path = config_path()?;
        let raw = if config_path.exists() {
            let text = std::fs::read_to_string(&config_path).map_err(StoreError::Io)?;
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
            let (store, description): (Arc<dyn Store>, Option<String>) = match entry {
                RawStoreConfig::JsonFile { path, description } => {
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
                description: None,
                store,
            }],
            nuance,
        })
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
    // Deterministic hash of sorted fingerprint strings.
    // Using a simple FNV-1a 64-bit hash — no crypto needed, just stability.
    let mut hash: u64 = 0xcbf29ce484222325;
    for s in fingerprints {
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
}
