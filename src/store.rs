use crate::types::{JournalFile, JournalItem, JournalSummary, Pagination};
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
pub trait Store: Send + Sync {
    fn load(&self, name: &str, pagination: &Pagination)
    -> Result<(JournalFile, usize), StoreError>;
    fn create(&self, journal: JournalFile) -> Result<(), StoreError>;
    fn add_items(&self, name: &str, items: Vec<JournalItem>) -> Result<usize, StoreError>;
    fn list(
        &self,
        pagination: &Pagination,
        archived: bool,
    ) -> Result<(Vec<JournalSummary>, usize), StoreError>;
    fn delete(&self, name: &str) -> Result<(), StoreError>;
    fn exists(&self, name: &str) -> Result<bool, StoreError>;
    fn archive(&self, name: &str) -> Result<(), StoreError>;
    fn unarchive(&self, name: &str) -> Result<(), StoreError>;
}
