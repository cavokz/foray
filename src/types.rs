use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ItemType {
    Finding,
    Decision,
    Snippet,
    Note,
}

impl std::fmt::Display for ItemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ItemType::Finding => write!(f, "finding"),
            ItemType::Decision => write!(f, "decision"),
            ItemType::Snippet => write!(f, "snippet"),
            ItemType::Note => write!(f, "note"),
        }
    }
}

impl std::str::FromStr for ItemType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "finding" => Ok(ItemType::Finding),
            "decision" => Ok(ItemType::Decision),
            "snippet" => Ok(ItemType::Snippet),
            "note" => Ok(ItemType::Note),
            other => Err(format!(
                "unknown item type: '{}'. Use finding, decision, snippet, or note",
                other
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: ItemType,
    pub content: String,
    #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
    pub file_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub added_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _note: Option<String>,
    pub name: String,
    pub project: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub items: Vec<ContextItem>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ContextFile {
    pub fn new(name: String, project: String) -> Self {
        let now = Utc::now();
        Self {
            _note: Some("Edit this file freely. Each file is self-contained.".to_string()),
            name,
            project,
            parent: None,
            items: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSummary {
    pub name: String,
    pub item_count: usize,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// Validates a context name: must be [a-z0-9_-] only, non-empty, max 64 chars.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("context name cannot be empty".to_string());
    }
    if name.len() > 64 {
        return Err("context name too long (max 64 characters)".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(format!(
            "invalid context name '{}': only lowercase letters, digits, hyphens, and underscores allowed",
            name
        ));
    }
    Ok(())
}
