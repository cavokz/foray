use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::schemars::JsonSchema;
use rmcp::{tool, tool_router, ErrorData as McpError};
use serde::{Deserialize, Serialize};

use crate::git;
use crate::store::{self, Store, StoreError};
use crate::tree;
use crate::types::{ContextItem, ItemType};

// --- Parameter structs ---

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetContextParams {
    /// Context name. Defaults to active context if omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddItemParams {
    /// The content of the item (finding, decision, snippet, or note).
    pub content: String,
    /// Item type: finding, decision, snippet, or note. Defaults to "note".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    /// File path, URL, ticket link, or other reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_ref: Option<String>,
    /// Comma-separated tags.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ForkContextParams {
    /// Name for the new forked context.
    pub new_name: String,
    /// Source context to fork from. Defaults to active context if omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SwitchContextParams {
    /// Context name to switch to. Creates a new empty context if it doesn't exist.
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveItemParams {
    /// ID of the item to remove.
    pub item_id: String,
}

// --- Response structs ---

#[derive(Debug, Serialize)]
struct GetContextResponse {
    name: String,
    parent: Option<String>,
    items: Vec<ContextItem>,
    item_count: usize,
}

#[derive(Debug, Serialize)]
struct AddItemResponse {
    id: String,
    context: String,
    item_count: usize,
}

#[derive(Debug, Serialize)]
struct ForkContextResponse {
    name: String,
    parent: Option<String>,
    item_count: usize,
}

#[derive(Debug, Serialize)]
struct SwitchContextResponse {
    name: String,
    item_count: usize,
    created: bool,
}

#[derive(Debug, Serialize)]
struct ListContextsResponse {
    contexts: Vec<crate::types::ContextSummary>,
    tree: String,
}

#[derive(Debug, Serialize)]
struct RemoveItemResponse {
    removed: bool,
    id: String,
    context: String,
}

#[derive(Debug, Serialize)]
struct GetStatusResponse {
    project: String,
    active_context: Option<String>,
    item_count: Option<usize>,
    git_branch: Option<String>,
}

// --- Server ---

#[derive(Clone)]
pub struct HunchServer {
    store: Arc<dyn Store>,
    project: String,
    workspace: PathBuf,
}

impl HunchServer {
    pub fn new(store: Arc<dyn Store>, project: String, workspace: PathBuf) -> Self {
        Self {
            store,
            project,
            workspace,
        }
    }

    fn resolve_active(&self) -> Result<String, McpError> {
        self.store
            .get_active()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?
            .ok_or_else(|| {
                McpError::invalid_params(
                    "No active context. Use switch_context to create one.",
                    None,
                )
            })
    }

    fn store_err_to_mcp(e: StoreError) -> McpError {
        match e {
            StoreError::NotFound(_) => McpError::invalid_params(e.to_string(), None),
            StoreError::AlreadyExists(_) => McpError::invalid_params(e.to_string(), None),
            StoreError::InvalidName(_) => McpError::invalid_params(e.to_string(), None),
            StoreError::Io(_) | StoreError::Parse(_) => {
                McpError::internal_error(e.to_string(), None)
            }
        }
    }
}

#[tool_router(server_handler)]
impl HunchServer {
    #[tool(description = "Read all items in a context. Defaults to the active context.")]
    async fn get_context(
        &self,
        Parameters(params): Parameters<GetContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let name = match params.name {
            Some(n) => n,
            None => self.resolve_active()?,
        };

        let ctx = self.store.load(&name).map_err(Self::store_err_to_mcp)?;

        let resp = GetContextResponse {
            name: ctx.name,
            parent: ctx.parent,
            item_count: ctx.items.len(),
            items: ctx.items,
        };

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }

    #[tool(description = "Add a finding, decision, snippet, or note to the active context.")]
    async fn add_item(
        &self,
        Parameters(params): Parameters<AddItemParams>,
    ) -> Result<CallToolResult, McpError> {
        let name = self.resolve_active()?;

        let item_type: ItemType = params
            .item_type
            .as_deref()
            .unwrap_or("note")
            .parse()
            .map_err(|e: String| McpError::invalid_params(e, None))?;

        let tags = params.tags.map(|t| {
            t.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        });

        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();

        let item = ContextItem {
            id: id.clone(),
            item_type,
            content: params.content,
            file_ref: params.file_ref,
            tags,
            added_at: chrono::Utc::now(),
        };

        self.store
            .add_item(&name, item)
            .map_err(Self::store_err_to_mcp)?;

        let ctx = self.store.load(&name).map_err(Self::store_err_to_mcp)?;

        let resp = AddItemResponse {
            id,
            context: name,
            item_count: ctx.items.len(),
        };

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }

    #[tool(description = "Fork (snapshot-copy) a context under a new name. Switches to the fork.")]
    async fn fork_context(
        &self,
        Parameters(params): Parameters<ForkContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let source = match params.from {
            Some(n) => n,
            None => self.resolve_active()?,
        };

        let forked = store::fork_context(&*self.store, &source, &params.new_name)
            .map_err(Self::store_err_to_mcp)?;

        let resp = ForkContextResponse {
            name: forked.name,
            parent: forked.parent,
            item_count: forked.items.len(),
        };

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }

    #[tool(description = "Switch to a context (creates a new empty one if it doesn't exist).")]
    async fn switch_context(
        &self,
        Parameters(params): Parameters<SwitchContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let (ctx, created) = store::switch_context(&*self.store, &params.name, &self.project)
            .map_err(Self::store_err_to_mcp)?;

        let resp = SwitchContextResponse {
            name: ctx.name,
            item_count: ctx.items.len(),
            created,
        };

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }

    #[tool(description = "List all contexts in the project with fork lineage tree.")]
    async fn list_contexts(&self) -> Result<CallToolResult, McpError> {
        let summaries = self.store.list().map_err(Self::store_err_to_mcp)?;
        let tree_str = tree::build_tree(&summaries);

        let resp = ListContextsResponse {
            contexts: summaries,
            tree: tree_str,
        };

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }

    #[tool(description = "Remove an item by ID from the active context.")]
    async fn remove_item(
        &self,
        Parameters(params): Parameters<RemoveItemParams>,
    ) -> Result<CallToolResult, McpError> {
        let name = self.resolve_active()?;

        let removed = self
            .store
            .remove_item(&name, &params.item_id)
            .map_err(Self::store_err_to_mcp)?;

        if !removed {
            return Err(McpError::invalid_params(
                format!("item '{}' not found in context '{}'", params.item_id, name),
                None,
            ));
        }

        let resp = RemoveItemResponse {
            removed: true,
            id: params.item_id,
            context: name,
        };

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }

    #[tool(description = "Get project status: project name, active context, git branch.")]
    async fn get_status(&self) -> Result<CallToolResult, McpError> {
        let active = self.store.get_active().map_err(Self::store_err_to_mcp)?;

        let item_count = if let Some(ref name) = active {
            self.store.load(name).ok().map(|ctx| ctx.items.len())
        } else {
            None
        };

        let branch = git::detect_branch(&self.workspace);

        let resp = GetStatusResponse {
            project: self.project.clone(),
            active_context: active,
            item_count,
            git_branch: branch,
        };

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&resp).unwrap(),
        )]))
    }
}
