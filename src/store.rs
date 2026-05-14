use crate::types::{JournalFile, JournalItem, JournalSummary, Pagination};
use async_trait::async_trait;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum StoreError {
    #[error("journal not found: {0}")]
    NotFound(String),
    #[error("journal already exists: {0}")]
    AlreadyExists(String),
    #[error("journal is archived: {0}")]
    Archived(String),
    #[error("journal schema {found} is too new (max supported: {max})")]
    SchemaTooNew {
        found: u32,
        max: u32,
        origin: SchemaOrigin,
    },
    #[error("wire protocol {found} is too new (max supported: {max})")]
    ProtocolTooNew { found: u32, max: u32 },
    #[error("operation not supported on remote stores: {0}")]
    Unsupported(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Where a schema-too-new condition was detected.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SchemaOrigin {
    /// Detected reading a storage file (server binary is older than the file).
    Storage,
    /// Detected reading a wire response (client binary is older than the server).
    Wire,
}

/// Backend-agnostic journal storage.
#[async_trait]
pub(crate) trait Store: Send + Sync {
    async fn load(
        &self,
        name: &str,
        pagination: &Pagination,
    ) -> Result<(JournalFile, usize), StoreError>;
    async fn create(
        &self,
        name: &str,
        title: String,
        meta: Option<HashMap<String, serde_json::Value>>,
    ) -> Result<(), StoreError>;
    async fn add_items(&self, name: &str, items: Vec<JournalItem>) -> Result<usize, StoreError>;
    async fn list(&self, archived: bool) -> Result<(Vec<JournalSummary>, usize), StoreError>;
    async fn delete(&self, name: &str, archived: bool) -> Result<(), StoreError>;
    async fn archive(&self, name: &str) -> Result<(), StoreError>;
    async fn unarchive(&self, name: &str) -> Result<(), StoreError>;
}
