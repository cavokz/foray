use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "hunch",
    about = "Investigation journals — start with a hunch, fork it when it branches, keep the trail.",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start MCP stdio server
    Serve {
        /// Workspace path (defaults to current directory)
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Show project status
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show a context with all its items
    Show {
        /// Context name (defaults to active)
        name: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Add an item to the active context
    Add {
        /// Item content
        content: String,
        /// Item type: finding, decision, snippet, note
        #[arg(long, short = 't', default_value = "note")]
        r#type: String,
        /// File reference (path, URL, ticket link)
        #[arg(long, short = 'r')]
        r#ref: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
    },
    /// Fork a context (snapshot copy)
    Fork {
        /// Name for the new forked context
        new_name: String,
        /// Source context (defaults to active)
        #[arg(long)]
        from: Option<String>,
    },
    /// Switch to a context (creates if new)
    Switch {
        /// Context name
        name: String,
    },
    /// List all contexts with fork tree
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Remove an item by ID from the active context
    Remove {
        /// Item ID
        item_id: String,
    },
}
