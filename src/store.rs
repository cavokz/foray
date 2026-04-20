use crate::types::{JournalFile, JournalItem, JournalSummary, Pagination};
use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("journal not found: {0}")]
    NotFound(String),
    #[error("journal already exists: {0}")]
    AlreadyExists(String),
    #[error("journal is archived: {0}")]
    Archived(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Backend-agnostic journal storage.
#[async_trait]
pub trait Store: Send + Sync {
    async fn load(
        &self,
        name: &str,
        pagination: &Pagination,
    ) -> Result<(JournalFile, usize), StoreError>;
    async fn create(&self, journal: JournalFile) -> Result<(), StoreError>;
    async fn add_items(&self, name: &str, items: Vec<JournalItem>) -> Result<usize, StoreError>;
    async fn list(
        &self,
        pagination: &Pagination,
        archived: bool,
    ) -> Result<(Vec<JournalSummary>, usize), StoreError>;
    async fn delete(&self, name: &str) -> Result<(), StoreError>;
    async fn exists(&self, name: &str) -> Result<bool, StoreError>;
    async fn archive(&self, name: &str) -> Result<String, StoreError>;
    async fn unarchive(&self, name: &str) -> Result<String, StoreError>;
}
