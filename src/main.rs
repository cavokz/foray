use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use rmcp::transport::stdio;
use rmcp::ServiceExt;

use hunch::cli::{Cli, Commands};
use hunch::git;
use hunch::server::HunchServer;
use hunch::store::{self, JsonFileStore, Store, StoreError};
use hunch::tree;
use hunch::types::{ContextItem, ItemType};

fn make_store(workspace: &Path) -> Arc<dyn Store> {
    let project = git::detect_project(workspace);
    let base = JsonFileStore::default_base_dir();
    Arc::new(JsonFileStore::new(&base, &project))
}

fn get_project(workspace: &Path) -> String {
    git::detect_project(workspace)
}

fn require_active(store: &dyn Store) -> Result<String, StoreError> {
    store.get_active()?.ok_or_else(|| {
        StoreError::NotFound(
            "No active context. Use 'hunch switch <name>' to create one.".to_string(),
        )
    })
}

fn print_error(e: StoreError) {
    eprintln!("error: {}", e);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { workspace } => {
            let ws = workspace
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap());
            let project = git::detect_project(&ws);
            let base = JsonFileStore::default_base_dir();
            let store: Arc<dyn Store> = Arc::new(JsonFileStore::new(&base, &project));
            let server = HunchServer::new(store, project, ws);

            // Log to stderr so stdout is clean for MCP stdio
            eprintln!("hunch: MCP server starting (stdio)");
            let service = server.serve(stdio()).await?;
            service.waiting().await?;
        }

        Commands::Status { json } => {
            let ws = std::env::current_dir()?;
            let store = make_store(&ws);
            let project = get_project(&ws);
            let active = store.get_active().unwrap_or(None);
            let branch = git::detect_branch(&ws);

            let item_count = active
                .as_ref()
                .and_then(|name| store.load(name).ok())
                .map(|ctx| ctx.items.len());

            if json {
                let resp = serde_json::json!({
                    "project": project,
                    "active_context": active,
                    "item_count": item_count,
                    "git_branch": branch,
                });
                println!("{}", serde_json::to_string_pretty(&resp)?);
            } else {
                println!("Project:  {}", project);
                match &active {
                    Some(name) => println!("Active:   {}", name),
                    None => println!("Active:   (none)"),
                }
                if let Some(count) = item_count {
                    println!("Items:    {}", count);
                }
                if let Some(ref b) = branch {
                    println!("Branch:   {}", b);
                }
            }
        }

        Commands::Show { name, json } => {
            let ws = std::env::current_dir()?;
            let store = make_store(&ws);
            let ctx_name = match name {
                Some(n) => n,
                None => match require_active(&*store) {
                    Ok(n) => n,
                    Err(e) => {
                        print_error(e);
                        std::process::exit(1);
                    }
                },
            };
            match store.load(&ctx_name) {
                Ok(ctx) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&ctx)?);
                    } else {
                        println!("Context: {}", ctx.name);
                        if let Some(ref parent) = ctx.parent {
                            println!("Parent:  {}", parent);
                        }
                        println!("Items:   {}", ctx.items.len());
                        println!();
                        for item in &ctx.items {
                            println!("  [{}] ({}) {}", item.id, item.item_type, item.content);
                            if let Some(ref r) = item.file_ref {
                                println!("         ref: {}", r);
                            }
                            if let Some(ref tags) = item.tags {
                                println!("         tags: {}", tags.join(", "));
                            }
                        }
                    }
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Add {
            content,
            r#type,
            r#ref,
            tags,
        } => {
            let ws = std::env::current_dir()?;
            let store = make_store(&ws);
            let ctx_name = match require_active(&*store) {
                Ok(n) => n,
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            };

            let item_type: ItemType = r#type.parse().unwrap_or_else(|e: String| {
                eprintln!("error: {}", e);
                std::process::exit(1);
            });

            let parsed_tags = tags.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });

            let id = uuid::Uuid::new_v4().to_string()[..8].to_string();

            let item = ContextItem {
                id: id.clone(),
                item_type: item_type.clone(),
                content: content.clone(),
                file_ref: r#ref,
                tags: parsed_tags,
                added_at: chrono::Utc::now(),
            };

            match store.add_item(&ctx_name, item) {
                Ok(()) => println!("Added {} [{}] to {}", item_type, id, ctx_name),
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Fork { new_name, from } => {
            let ws = std::env::current_dir()?;
            let store = make_store(&ws);
            let source = match from {
                Some(n) => n,
                None => match require_active(&*store) {
                    Ok(n) => n,
                    Err(e) => {
                        print_error(e);
                        std::process::exit(1);
                    }
                },
            };

            match store::fork_context(&*store, &source, &new_name) {
                Ok(forked) => {
                    println!(
                        "Forked '{}' -> '{}' ({} items copied)",
                        source,
                        forked.name,
                        forked.items.len()
                    );
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Switch { name } => {
            let ws = std::env::current_dir()?;
            let store = make_store(&ws);
            let project = get_project(&ws);
            match store::switch_context(&*store, &name, &project) {
                Ok((ctx, created)) => {
                    if created {
                        println!("Created and switched to '{}'", ctx.name);
                    } else {
                        println!("Switched to '{}' ({} items)", ctx.name, ctx.items.len());
                    }
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::List { json } => {
            let ws = std::env::current_dir()?;
            let store = make_store(&ws);
            match store.list() {
                Ok(summaries) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&summaries)?);
                    } else {
                        let tree_str = tree::build_tree(&summaries);
                        println!("{}", tree_str);
                    }
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Remove { item_id } => {
            let ws = std::env::current_dir()?;
            let store = make_store(&ws);
            let ctx_name = match require_active(&*store) {
                Ok(n) => n,
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            };

            match store.remove_item(&ctx_name, &item_id) {
                Ok(true) => println!("Removed item '{}' from '{}'", item_id, ctx_name),
                Ok(false) => {
                    eprintln!("error: item '{}' not found in '{}'", item_id, ctx_name);
                    std::process::exit(1);
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
