use crate::store::{Store, fork_journal};
use crate::tree::{build_tree, extract_fork_infos};
use crate::types::{ItemType, JournalFile, JournalItem, Pagination, item_id, validate_name};
use chrono::Utc;
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "foray",
    version,
    about = "Persistent, forkable investigation journals"
)]
pub struct Cli {
    /// Override journal name (skips env + .forayrc resolution)
    #[arg(long, global = true)]
    pub journal: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start MCP stdio server
    Serve,
    /// Show a journal with all items
    Show {
        /// Journal name (optional if resolvable)
        name: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Follow: watch for new items in real time
        #[arg(short, long)]
        follow: bool,
        /// Maximum number of items
        #[arg(long)]
        limit: Option<usize>,
        /// Skip N items
        #[arg(long)]
        offset: Option<usize>,
    },
    /// Add an item to the current journal
    Add {
        /// Item content
        content: String,
        /// Item type: finding, decision, snippet, note
        #[arg(long, name = "type", default_value = "note")]
        item_type: String,
        /// File reference (path, URL, etc.)
        #[arg(long, name = "ref")]
        file_ref: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// Metadata key=value pairs
        #[arg(long = "meta", value_name = "KEY=VALUE")]
        meta: Vec<String>,
    },
    /// Create or fork a journal, set it as active in .forayrc
    Open {
        /// Journal name
        name: String,
        /// Title (required for new/fork)
        #[arg(long)]
        title: Option<String>,
        /// Fork from another journal. Without value: fork from active journal.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        fork: Option<String>,
        /// Metadata key=value pairs
        #[arg(long = "meta", value_name = "KEY=VALUE")]
        meta: Vec<String>,
    },
    /// List journals
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Show fork lineage tree
        #[arg(long)]
        tree: bool,
        /// Show archived journals
        #[arg(long)]
        archived: bool,
        /// Maximum number of journals
        #[arg(long)]
        limit: Option<usize>,
        /// Skip N journals
        #[arg(long)]
        offset: Option<usize>,
    },
    /// Archive a journal
    Archive {
        /// Journal name
        name: String,
    },
    /// Unarchive a journal
    Unarchive {
        /// Journal name
        name: String,
    },
    /// Export journal JSON to stdout or file
    Export {
        /// Journal name
        name: String,
        /// Output file (default: stdout)
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Import journal JSON from stdin or file
    Import {
        /// Input file (default: stdin)
        #[arg(long)]
        file: Option<PathBuf>,
    },
}

/// Resolve journal name from CLI flag, env, or .forayrc walk-up.
pub fn resolve_journal(
    cli_flag: Option<&str>,
    explicit_name: Option<&str>,
) -> anyhow::Result<String> {
    let name = if let Some(name) = explicit_name {
        name.to_string()
    } else if let Some(name) = cli_flag {
        name.to_string()
    } else if let Ok(name) = std::env::var("FORAY_JOURNAL")
        && !name.is_empty()
    {
        name
    } else if let Some(name) = find_forayrc(&std::env::current_dir()?) {
        name
    } else {
        anyhow::bail!(
            "no journal specified. Use --journal <name>, set FORAY_JOURNAL, \
             or run `foray open <name>` to create a .forayrc"
        )
    };
    validate_name(&name).map_err(|e| anyhow::anyhow!(e))?;
    Ok(name)
}

/// Walk up from `start_dir` looking for `.forayrc` with `current-journal`.
pub fn find_forayrc(start_dir: &std::path::Path) -> Option<String> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let rc_path = dir.join(".forayrc");
        if rc_path.is_file()
            && let Ok(contents) = std::fs::read_to_string(&rc_path)
            && let Ok(table) = contents.parse::<toml::Table>()
        {
            if let Some(name) = table.get("current-journal").and_then(|v| v.as_str()) {
                return Some(name.to_string());
            }
            if table.get("root").and_then(|v| v.as_bool()) == Some(true) {
                return None;
            }
        }
        if !dir.pop() {
            break;
        }
    }
    None
}

/// Write or update `.forayrc` in the current directory.
pub fn write_forayrc(name: &str) -> anyhow::Result<()> {
    let rc_path = std::env::current_dir()?.join(".forayrc");
    let mut table = if rc_path.is_file() {
        let contents = std::fs::read_to_string(&rc_path)?;
        contents.parse::<toml::Table>().unwrap_or_default()
    } else {
        toml::Table::new()
    };
    table.insert("current-journal".into(), toml::Value::String(name.into()));
    std::fs::write(&rc_path, toml::to_string_pretty(&table)?)?;
    Ok(())
}

/// Parse `--meta KEY=VALUE` pairs into a HashMap.
pub fn parse_meta(pairs: &[String]) -> Option<HashMap<String, serde_json::Value>> {
    if pairs.is_empty() {
        return None;
    }
    let map: HashMap<String, serde_json::Value> = pairs
        .iter()
        .filter_map(|s| {
            let (k, v) = s.split_once('=')?;
            Some((k.to_string(), serde_json::Value::String(v.to_string())))
        })
        .collect();
    if map.is_empty() { None } else { Some(map) }
}

fn parse_item_type(s: &str) -> anyhow::Result<ItemType> {
    match s {
        "finding" => Ok(ItemType::Finding),
        "decision" => Ok(ItemType::Decision),
        "snippet" => Ok(ItemType::Snippet),
        "note" => Ok(ItemType::Note),
        other => {
            anyhow::bail!("unknown item type: {other}. Valid: finding, decision, snippet, note")
        }
    }
}

fn print_item(item: &JournalItem) {
    let type_str = format!("{:?}", item.item_type).to_lowercase();
    println!(
        "[{}] ({}) {}",
        item.added_at.format("%Y-%m-%d %H:%M"),
        type_str,
        item.content
    );
    if let Some(r) = &item.file_ref {
        println!("  ref: {r}");
    }
    if let Some(tags) = &item.tags {
        println!("  tags: {}", tags.join(", "));
    }
}

/// Execute a CLI command against the store.
pub fn run(cli: &Cli, store: &dyn Store) -> anyhow::Result<()> {
    match &cli.command {
        Commands::Serve => {
            unreachable!("serve is handled in main")
        }
        Commands::Show {
            name,
            json,
            follow,
            limit,
            offset,
        } => {
            let journal_name = resolve_journal(cli.journal.as_deref(), name.as_deref())?;
            let pagination = Pagination {
                limit: *limit,
                offset: *offset,
            };
            let (journal, total) = store.load(&journal_name, &pagination)?;
            if *json {
                for item in &journal.items {
                    println!("{}", serde_json::to_string(item)?);
                }
            } else {
                println!("Journal: {}", journal.name);
                if let Some(title) = &journal.title {
                    println!("Title:   {title}");
                }
                println!("Items:   {} / {total}", journal.items.len());
                println!();
                for item in &journal.items {
                    print_item(item);
                }
            }
            if *follow {
                let mut seen = total;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    let all = Pagination::default();
                    let (journal, new_total) = store.load(&journal_name, &all)?;
                    if new_total > seen {
                        for item in &journal.items[seen..] {
                            if *json {
                                println!("{}", serde_json::to_string(item).unwrap());
                            } else {
                                print_item(item);
                            }
                        }
                        seen = new_total;
                    }
                }
            }
        }
        Commands::Add {
            content,
            item_type,
            file_ref,
            tags,
            meta,
        } => {
            let journal_name = resolve_journal(cli.journal.as_deref(), None)?;
            let it = parse_item_type(item_type)?;
            let parsed_tags = tags.as_ref().map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>()
            });
            let item = JournalItem {
                id: item_id(),
                item_type: it,
                content: content.clone(),
                file_ref: file_ref.clone(),
                tags: parsed_tags,
                added_at: Utc::now(),
                meta: parse_meta(meta),
            };
            let count = store.add_items(&journal_name, vec![item])?;
            println!("Added to {journal_name} ({count} items)");
        }
        Commands::Open {
            name,
            title,
            fork,
            meta,
        } => {
            validate_name(name).map_err(|e| anyhow::anyhow!(e))?;
            let meta = parse_meta(meta);
            let exists = store.exists(name)?;

            match (exists, fork) {
                (false, None) => {
                    let title = title.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("--title is required when creating a new journal")
                    })?;
                    let journal = JournalFile::new(name, Some(title.clone()), meta);
                    store.create(journal)?;
                    println!("Created journal: {name}");
                }
                (false, Some(source)) => {
                    let title = title.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("--title is required when forking a journal")
                    })?;
                    let source_name = if source.is_empty() {
                        resolve_journal(cli.journal.as_deref(), None)?
                    } else {
                        validate_name(source).map_err(|e| anyhow::anyhow!(e))?;
                        source.clone()
                    };
                    let forked = fork_journal(store, &source_name, name, title.clone(), meta)?;
                    println!(
                        "Forked {} → {} ({} items)",
                        source_name,
                        name,
                        forked.items.len()
                    );
                }
                (true, None) => {
                    println!("Journal already exists: {name}");
                }
                (true, Some(source)) if source == name || source.is_empty() => {
                    println!("Journal already exists: {name}");
                }
                (true, Some(_)) => {
                    anyhow::bail!("journal already exists: {name}");
                }
            }
            write_forayrc(name)?;
            println!("Set active journal in .forayrc");
        }
        Commands::List {
            json,
            tree,
            archived,
            limit,
            offset,
        } => {
            let pagination = Pagination {
                limit: *limit,
                offset: *offset,
            };
            let (summaries, total) = store.list(&pagination, *archived)?;

            if *tree {
                let all_p = Pagination::default();
                let (all_summaries, _) = store.list(&all_p, false)?;
                let mut fork_data = Vec::new();
                for s in &all_summaries {
                    if let Ok((j, _)) = store.load(&s.name, &all_p) {
                        let infos = extract_fork_infos(&j.items);
                        if !infos.is_empty() {
                            fork_data.push((s.name.clone(), infos));
                        }
                    }
                }
                print!("{}", build_tree(&all_summaries, &fork_data));
            } else if *json {
                println!("{}", serde_json::to_string_pretty(&summaries)?);
            } else {
                let label = if *archived { "archived" } else { "active" };
                println!("{} journal(s) ({label}):", total);
                for s in &summaries {
                    let title = s.title.as_deref().unwrap_or("");
                    println!("  {} ({} items) {}", s.name, s.item_count, title);
                }
            }
        }
        Commands::Archive { name } => {
            store.archive(name)?;
            println!("Archived: {name}");
        }
        Commands::Unarchive { name } => {
            store.unarchive(name)?;
            println!("Unarchived: {name}");
        }
        Commands::Export { name, file } => {
            let (journal, _) = store.load(name, &Pagination::default())?;
            let data = serde_json::to_string_pretty(&journal)?;
            match file {
                Some(path) => std::fs::write(path, format!("{data}\n"))?,
                None => println!("{data}"),
            }
        }
        Commands::Import { file } => {
            let data = match file {
                Some(path) => std::fs::read_to_string(path)?,
                None => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                }
            };
            let journal: JournalFile = serde_json::from_str(&data)?;
            validate_name(&journal.name).map_err(|e| anyhow::anyhow!(e))?;
            store.create(journal)?;
            println!("Imported successfully");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_forayrc() {
        let dir = tempfile::TempDir::new().unwrap();
        let rc_path = dir.path().join(".forayrc");
        std::fs::write(&rc_path, "current-journal = \"test-journal\"\n").unwrap();
        assert_eq!(find_forayrc(dir.path()), Some("test-journal".into()));
    }

    #[test]
    fn test_find_forayrc_root_stops_walk() {
        let dir = tempfile::TempDir::new().unwrap();
        let rc_path = dir.path().join(".forayrc");
        std::fs::write(&rc_path, "root = true\n").unwrap();
        let child = dir.path().join("sub");
        std::fs::create_dir(&child).unwrap();
        assert_eq!(find_forayrc(&child), None);
    }

    #[test]
    fn test_parse_meta() {
        let pairs = vec!["key1=value1".into(), "key2=value2".into()];
        let meta = parse_meta(&pairs).unwrap();
        assert_eq!(meta.get("key1").unwrap(), "value1");
        assert_eq!(meta.get("key2").unwrap(), "value2");
        assert!(parse_meta(&[]).is_none());
    }

    #[test]
    fn test_parse_item_type() {
        assert_eq!(parse_item_type("finding").unwrap(), ItemType::Finding);
        assert_eq!(parse_item_type("decision").unwrap(), ItemType::Decision);
        assert_eq!(parse_item_type("snippet").unwrap(), ItemType::Snippet);
        assert_eq!(parse_item_type("note").unwrap(), ItemType::Note);
        assert!(parse_item_type("invalid").is_err());
    }
}
