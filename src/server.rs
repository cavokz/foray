use crate::store::{JournalStore, StoreError, fork_journal};
use crate::types::{ItemType, JournalFile, JournalItem, Pagination, item_id, validate_name};
use chrono::Utc;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ErrorData, GetPromptRequestParams, GetPromptResult, Implementation,
    InitializeResult, ListPromptsResult, PaginatedRequestParams, PromptMessage, PromptMessageRole,
    ServerCapabilities,
};
use rmcp::schemars;
use rmcp::schemars::JsonSchema;
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{prompt, prompt_router, tool, tool_router};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

fn deserialize_tags<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        Vec(Vec<String>),
        String(String),
    }
    Option::<StringOrVec>::deserialize(deserializer).map(|opt| {
        opt.map(|v| match v {
            StringOrVec::Vec(vec) => vec,
            StringOrVec::String(s) => s.split(',').map(|t| t.trim().to_string()).collect(),
        })
    })
}

// ── Tool parameter types ────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenJournalParams {
    /// Journal name ([a-z0-9_-], max 64 chars)
    pub name: String,
    /// Title for new journals (required when creating or forking, ignored when reopening)
    #[serde(default)]
    pub title: Option<String>,
    /// Source journal name to fork from
    #[serde(default)]
    pub fork: Option<String>,
    /// Journal-level metadata (free-form key-value pairs)
    #[serde(default)]
    pub meta: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SyncItemInput {
    /// Content of the item
    pub content: String,
    /// Type of item: finding, decision, snippet, note (default: note)
    #[serde(default)]
    pub item_type: Option<String>,
    /// File reference (path, URL, ticket link, etc.)
    #[serde(default)]
    pub file_ref: Option<String>,
    /// Tags for categorization (array or comma-separated string)
    #[serde(default, deserialize_with = "deserialize_tags")]
    pub tags: Option<Vec<String>>,
    /// Item-level metadata (free-form key-value pairs)
    #[serde(default)]
    pub meta: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SyncJournalParams {
    /// Journal name
    pub name: String,
    /// Position from previous sync response — return only items after this position (omit for full read)
    #[serde(default)]
    pub cursor: Option<usize>,
    /// Maximum number of items to return (does not affect additions — all items are always added)
    #[serde(default)]
    pub limit: Option<usize>,
    /// Items to add to the journal
    #[serde(default)]
    pub items: Option<Vec<SyncItemInput>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListJournalsParams {
    /// Maximum number of journals to return
    #[serde(default)]
    pub limit: Option<usize>,
    /// Number of journals to skip
    #[serde(default)]
    pub offset: Option<usize>,
}

// ── Tool response types ─────────────────────────────────────────────

#[derive(Serialize)]
struct OpenJournalResponse {
    name: String,
    title: Option<String>,
    item_count: usize,
    created: bool,
}

#[derive(Serialize)]
struct SyncJournalResponse {
    name: String,
    title: Option<String>,
    items: Vec<serde_json::Value>,
    added_ids: Vec<String>,
    cursor: usize,
    total: usize,
}

#[derive(Serialize)]
struct ListJournalsResponse {
    journals: Vec<serde_json::Value>,
    total: usize,
    limit: Option<usize>,
    offset: Option<usize>,
}

// ── Prompt parameter types ──────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartInvestigationParams {
    /// Name for the new journal
    pub name: String,
    /// Title describing the journal
    pub title: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResumeInvestigationParams {
    /// Name of the journal to resume
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SummarizeParams {
    /// Name of the journal to summarize
    pub name: String,
}

// ── Server ──────────────────────────────────────────────────────────

const MAX_LIMIT: usize = 500;
const MAX_CONTENT: usize = 64 * 1024;
const MAX_TITLE: usize = 512;
const MAX_TAGS: usize = 20;
const MAX_TAG_LEN: usize = 64;
const MAX_META: usize = 8 * 1024;

fn validate_meta(meta: &Option<HashMap<String, serde_json::Value>>) -> Result<(), ErrorData> {
    if let Some(m) = meta {
        let size = serde_json::to_string(m).unwrap_or_default().len();
        if size > MAX_META {
            return Err(ErrorData::invalid_params(
                format!("meta exceeds {MAX_META} byte limit ({size} bytes)"),
                None,
            ));
        }
    }
    Ok(())
}

fn validate_tags(tags: &Option<Vec<String>>) -> Result<(), ErrorData> {
    if let Some(t) = tags {
        if t.len() > MAX_TAGS {
            return Err(ErrorData::invalid_params(
                format!("too many tags ({}, max {MAX_TAGS})", t.len()),
                None,
            ));
        }
        for tag in t {
            if tag.len() > MAX_TAG_LEN {
                return Err(ErrorData::invalid_params(
                    format!("tag exceeds {MAX_TAG_LEN} char limit ({} chars)", tag.len()),
                    None,
                ));
            }
        }
    }
    Ok(())
}

const SERVER_INSTRUCTIONS: &str = "\
You have access to foray, a persistent journal system for capturing findings, decisions, \
and context across sessions. \
Use `list_journals` to see existing journals, `open_journal` to create or resume one, \
and `sync_journal` to read and write items.\n\n\
For the best experience, install the foray companion skill. \
Download it from the foray GitHub repository and save it to your project's skills directory. \
The skill teaches you when and how to use journal tools effectively — including when to fork, \
how to handle corrections, and how to anchor findings to source code.";

#[derive(Clone)]
pub struct ForayServer {
    store: Arc<dyn JournalStore>,
}

impl ForayServer {
    pub fn new(store: Arc<dyn JournalStore>) -> Self {
        Self { store }
    }

    fn store_err(e: StoreError) -> ErrorData {
        ErrorData::internal_error(e.to_string(), None)
    }

    fn parse_item_type(s: &str) -> Result<ItemType, ErrorData> {
        match s {
            "finding" => Ok(ItemType::Finding),
            "decision" => Ok(ItemType::Decision),
            "snippet" => Ok(ItemType::Snippet),
            "note" => Ok(ItemType::Note),
            other => Err(ErrorData::invalid_params(
                format!(
                    "unknown item type: {other}. Valid types: finding, decision, snippet, note"
                ),
                None,
            )),
        }
    }
}

#[tool_router]
impl ForayServer {
    #[tool(
        name = "open_journal",
        description = "Create, fork, or reopen a journal. title is required when creating or forking (error if missing), ignored when reopening. fork specifies source journal name. Idempotent if journal exists without fork."
    )]
    async fn open_journal(
        &self,
        Parameters(args): Parameters<OpenJournalParams>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_name(&args.name).map_err(|e| ErrorData::invalid_params(e, None))?;

        let exists = self.store.exists(&args.name).map_err(Self::store_err)?;

        match (exists, &args.fork) {
            (false, None) => {
                let title = args.title.ok_or_else(|| {
                    ErrorData::invalid_params("title is required when creating a new journal", None)
                })?;
                if title.len() > MAX_TITLE {
                    return Err(ErrorData::invalid_params(
                        format!(
                            "title exceeds {MAX_TITLE} char limit ({} chars)",
                            title.len()
                        ),
                        None,
                    ));
                }
                validate_meta(&args.meta)?;
                let journal = JournalFile::new(&args.name, Some(title), args.meta);
                self.store.create(journal).map_err(Self::store_err)?;
                let p = Pagination::default();
                let (j, _) = self.store.load(&args.name, &p).map_err(Self::store_err)?;
                let resp = OpenJournalResponse {
                    name: j.name,
                    title: j.title,
                    item_count: j.items.len(),
                    created: true,
                };
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string(&resp).unwrap(),
                )]))
            }
            (false, Some(source)) => {
                let title = args.title.ok_or_else(|| {
                    ErrorData::invalid_params("title is required when forking a journal", None)
                })?;
                if title.len() > MAX_TITLE {
                    return Err(ErrorData::invalid_params(
                        format!(
                            "title exceeds {MAX_TITLE} char limit ({} chars)",
                            title.len()
                        ),
                        None,
                    ));
                }
                validate_meta(&args.meta)?;
                let forked =
                    fork_journal(self.store.as_ref(), source, &args.name, title, args.meta)
                        .map_err(Self::store_err)?;
                let resp = OpenJournalResponse {
                    name: forked.name,
                    title: forked.title,
                    item_count: forked.items.len(),
                    created: true,
                };
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string(&resp).unwrap(),
                )]))
            }
            (true, None) => {
                let p = Pagination::default();
                let (j, total) = self.store.load(&args.name, &p).map_err(Self::store_err)?;
                let resp = OpenJournalResponse {
                    name: j.name,
                    title: j.title,
                    item_count: total,
                    created: false,
                };
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string(&resp).unwrap(),
                )]))
            }
            (true, Some(source)) if *source == args.name => {
                let p = Pagination::default();
                let (j, total) = self.store.load(&args.name, &p).map_err(Self::store_err)?;
                let resp = OpenJournalResponse {
                    name: j.name,
                    title: j.title,
                    item_count: total,
                    created: false,
                };
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string(&resp).unwrap(),
                )]))
            }
            (true, Some(_)) => Err(ErrorData::invalid_params(
                format!("journal already exists: {}", args.name),
                None,
            )),
        }
    }

    #[tool(
        name = "sync_journal",
        description = "Read and write journal items in one call. Returns items since your last cursor position. Pass items to add them. Pass cursor from the previous response to get only new items — omit cursor for a full read. Response includes cursor for the next call and added_ids for items you added."
    )]
    async fn sync_journal(
        &self,
        Parameters(args): Parameters<SyncJournalParams>,
    ) -> Result<CallToolResult, ErrorData> {
        validate_name(&args.name).map_err(|e| ErrorData::invalid_params(e, None))?;

        // Add items if provided
        let mut added_ids = Vec::new();
        if let Some(inputs) = args.items {
            let mut items_to_add = Vec::new();
            for input in inputs {
                if input.content.len() > MAX_CONTENT {
                    return Err(ErrorData::invalid_params(
                        format!(
                            "content exceeds {} byte limit ({} bytes)",
                            MAX_CONTENT,
                            input.content.len()
                        ),
                        None,
                    ));
                }
                validate_tags(&input.tags)?;
                validate_meta(&input.meta)?;
                let item_type = match &input.item_type {
                    Some(t) => Self::parse_item_type(t)?,
                    None => ItemType::Note,
                };
                let id = item_id();
                let item = JournalItem {
                    id: id.clone(),
                    item_type,
                    content: input.content,
                    file_ref: input.file_ref,
                    tags: input.tags,
                    added_at: Utc::now(),
                    meta: input.meta,
                };
                items_to_add.push(item);
                added_ids.push(id);
            }
            self.store
                .add_items(&args.name, items_to_add)
                .map_err(Self::store_err)?;
        }

        // Load journal and apply cursor
        let all = Pagination::default();
        let (journal, total) = self.store.load(&args.name, &all).map_err(Self::store_err)?;

        let after = args.cursor.unwrap_or(0);
        let items_slice = if after < journal.items.len() {
            &journal.items[after..]
        } else {
            &[]
        };

        let limit = args.limit.unwrap_or(items_slice.len()).min(MAX_LIMIT);
        let items: Vec<serde_json::Value> = items_slice
            .iter()
            .take(limit)
            .map(|i| serde_json::to_value(i).unwrap())
            .collect();

        let cursor = after + items.len();

        let resp = SyncJournalResponse {
            name: journal.name,
            title: journal.title,
            items,
            added_ids,
            cursor,
            total,
        };
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }

    #[tool(
        name = "list_journals",
        description = "List active journals. Paginated: defaults to first 500."
    )]
    async fn list_journals(
        &self,
        Parameters(args): Parameters<ListJournalsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let pagination = Pagination {
            limit: Some(args.limit.unwrap_or(MAX_LIMIT).min(MAX_LIMIT)),
            offset: args.offset,
        };
        let (summaries, total) = self
            .store
            .list(&pagination, false)
            .map_err(Self::store_err)?;

        let journals: Vec<serde_json::Value> = summaries
            .iter()
            .map(|s| serde_json::to_value(s).unwrap())
            .collect();

        let resp = ListJournalsResponse {
            journals,
            total,
            limit: args.limit,
            offset: args.offset,
        };
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }
}

#[prompt_router]
impl ForayServer {
    #[prompt(
        name = "start_journal",
        description = "List existing journals, create a new one, and begin recording items."
    )]
    async fn start_journal(
        &self,
        Parameters(args): Parameters<StartInvestigationParams>,
    ) -> Result<GetPromptResult, ErrorData> {
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            format!(
                "I want to start a new journal. \
                First, check if there are existing journals with `list_journals`. \
                Then create a new journal named \"{}\" with title \"{}\" using \
                `open_journal`. Record items as you work with `sync_journal`.",
                args.name, args.title
            ),
        )])
        .with_description("Start a new journal"))
    }

    #[prompt(
        name = "resume_journal",
        description = "Load the journal, summarize recent items, continue where you left off."
    )]
    async fn resume_journal(
        &self,
        Parameters(args): Parameters<ResumeInvestigationParams>,
    ) -> Result<GetPromptResult, ErrorData> {
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            format!(
                "I want to resume work on a journal. \
                Load journal \"{}\" with `sync_journal` and summarize what has been \
                recorded so far. Then continue, recording new items with `sync_journal`.",
                args.name
            ),
        )])
        .with_description("Resume an existing journal"))
    }

    #[prompt(
        name = "summarize",
        description = "Read all items in the journal and produce a synthesis."
    )]
    async fn summarize(
        &self,
        Parameters(args): Parameters<SummarizeParams>,
    ) -> Result<GetPromptResult, ErrorData> {
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(
            PromptMessageRole::User,
            format!(
                "Read all items from journal \"{}\" using `sync_journal` and produce \
                a synthesis. Group findings by theme, highlight key decisions, note \
                any open questions, and identify potential next steps.",
                args.name
            ),
        )])
        .with_description("Summarize a journal"))
    }
}

#[rmcp::tool_handler(router = "Self::tool_router()")]
#[rmcp::prompt_handler(router = "Self::prompt_router()")]
impl rmcp::ServerHandler for ForayServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_instructions(SERVER_INSTRUCTIONS.to_string())
        .with_server_info(Implementation::new("foray", env!("CARGO_PKG_VERSION")))
    }
}
