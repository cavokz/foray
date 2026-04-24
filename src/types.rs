use crate::migrate;
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Allowed item types in a journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    Finding,
    Decision,
    Snippet,
    Note,
}

/// A single entry inside a journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub content: String,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub file_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub added_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, serde_json::Value>>,
}

/// The top-level journal file stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _note: Option<String>,
    /// Schema version. Always call [`crate::migrate::migrate`] before deserializing —
    /// migration guarantees this field is present and at the current version.
    pub schema: u32,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub items: Vec<JournalItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, serde_json::Value>>,
}

impl JournalFile {
    pub fn new(
        name: &str,
        title: Option<String>,
        meta: Option<HashMap<String, serde_json::Value>>,
    ) -> Self {
        Self {
            _note: Some("Edit this file freely. Each file is self-contained.".into()),
            schema: migrate::CURRENT_SCHEMA,
            id: journal_id(),
            name: name.to_string(),
            title,
            items: Vec::new(),
            meta,
        }
    }
}

/// Summary returned by `list_journals`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalSummary {
    pub name: String,
    pub title: Option<String>,
    pub item_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<HashMap<String, serde_json::Value>>,
}

impl From<&JournalFile> for JournalSummary {
    fn from(j: &JournalFile) -> Self {
        Self {
            name: j.name.clone(),
            title: j.title.clone(),
            item_count: j.items.len(),
            meta: j.meta.clone(),
        }
    }
}

/// Pagination parameters for list/get operations.
#[derive(Debug, Clone, Default)]
pub struct Pagination {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl Pagination {
    /// Apply pagination to a slice, returning the page and total count.
    pub fn apply<T: Clone>(&self, items: &[T]) -> (Vec<T>, usize) {
        let total = items.len();
        let offset = self.offset.unwrap_or(0).min(total);
        let remaining = &items[offset..];
        let page = match self.limit {
            Some(limit) => &remaining[..limit.min(remaining.len())],
            None => remaining,
        };
        (page.to_vec(), total)
    }
}

const CONSONANTS: &[u8] = b"bcdfghjklmnpqrstvwxyz";

fn random_consonants(n: usize) -> String {
    let mut rng = rand::rng();
    (0..n)
        .map(|_| CONSONANTS[rng.random_range(0..CONSONANTS.len())] as char)
        .collect()
}

/// Generate a journal ID in `xxxxx-xxxxx-xxxxx` format (15 consonants, 3 groups of 5).
pub fn journal_id() -> String {
    let c = random_consonants(15);
    format!("{}-{}-{}", &c[..5], &c[5..10], &c[10..15])
}

/// Generate an item ID in `xxxx-xxxx-xxxx-xxxx` format (16 consonants, 4 groups of 4).
pub fn item_id() -> String {
    let c = random_consonants(16);
    format!("{}-{}-{}-{}", &c[..4], &c[4..8], &c[8..12], &c[12..16])
}

/// Validate a journal name: `[a-z0-9_-]`, non-empty, max 64 chars.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("journal name cannot be empty".into());
    }
    if name.len() > 64 {
        return Err("journal name cannot exceed 64 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("journal name may only contain [a-z0-9_-]".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_names() {
        assert!(validate_name("auth-triage").is_ok());
        assert!(validate_name("perf_deep_dive").is_ok());
        assert!(validate_name("a").is_ok());
        assert!(validate_name("abc-123_def").is_ok());
    }

    #[test]
    fn invalid_names() {
        assert!(validate_name("").is_err());
        assert!(validate_name("Auth").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("has.dot").is_err());
        assert!(validate_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn pagination_apply() {
        let items: Vec<i32> = (0..10).collect();

        let p = Pagination {
            limit: Some(3),
            offset: Some(2),
        };
        let (page, total) = p.apply(&items);
        assert_eq!(total, 10);
        assert_eq!(page, vec![2, 3, 4]);

        let p = Pagination::default();
        let (page, total) = p.apply(&items);
        assert_eq!(total, 10);
        assert_eq!(page.len(), 10);

        let p = Pagination {
            limit: Some(5),
            offset: Some(8),
        };
        let (page, total) = p.apply(&items);
        assert_eq!(total, 10);
        assert_eq!(page, vec![8, 9]);
    }

    #[test]
    fn journal_file_new() {
        let j = JournalFile::new("test-journal", Some("Test Title".into()), None);
        assert_eq!(j.name, "test-journal");
        assert_eq!(j.title.as_deref(), Some("Test Title"));
        assert!(j.items.is_empty());
        assert_eq!(j.id.len(), 17);
        assert_eq!(&j.id[5..6], "-");
        assert_eq!(&j.id[11..12], "-");
        assert!(
            j.id.chars()
                .all(|c| c == '-' || CONSONANTS.contains(&(c as u8)))
        );
    }

    #[test]
    fn item_id_format() {
        let id = item_id();
        assert_eq!(id.len(), 19);
        assert_eq!(&id[4..5], "-");
        assert_eq!(&id[9..10], "-");
        assert_eq!(&id[14..15], "-");
        assert!(
            id.chars()
                .all(|c| c == '-' || CONSONANTS.contains(&(c as u8)))
        );
    }
}
