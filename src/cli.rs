use crate::config::StoreRegistry;
use crate::migrate::{self, MigrateResult};
use crate::store::Store;
use crate::types::{ItemType, JournalFile, JournalItem, Pagination, item_id, validate_name};
use chrono::Utc;
use clap::{Parser, Subcommand};
use clap_complete::Shell;
#[cfg(feature = "dynamic-completion")]
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};
use std::collections::HashMap;
#[cfg(feature = "dynamic-completion")]
use std::ffi::OsStr;
use std::io::Read;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "foray", version, about = "Persistent investigation journals")]
pub struct Cli {
    /// Override journal name (skips env + .forayrc resolution)
    #[arg(long, global = true)]
    #[cfg_attr(feature = "dynamic-completion", arg(add = ArgValueCompleter::new(complete_journal_names)))]
    pub journal: Option<String>,

    /// Override store name (skips env + .forayrc resolution)
    #[arg(long, global = true)]
    #[cfg_attr(feature = "dynamic-completion", arg(add = ArgValueCompleter::new(complete_store_names)))]
    pub store: Option<String>,

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
        #[arg()]
        #[cfg_attr(feature = "dynamic-completion", arg(add = ArgValueCompleter::new(complete_journal_names)))]
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
    /// Create or open a journal, set it as active in .forayrc
    Open {
        /// Journal name
        name: String,
        /// Title (required for new journals)
        #[arg(long)]
        title: Option<String>,
        /// Metadata key=value pairs
        #[arg(long = "meta", value_name = "KEY=VALUE")]
        meta: Vec<String>,
    },
    /// List journals
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
        /// Show archived journals
        #[arg(long)]
        archived: bool,
        /// Maximum number of journals
        #[arg(long)]
        limit: Option<usize>,
        /// Skip N journals
        #[arg(long)]
        offset: Option<usize>,
        /// Output bare journal names for shell completion (one per line)
        #[arg(long, conflicts_with_all = ["json", "limit", "offset"])]
        completion: bool,
    },
    /// Archive a journal
    Archive {
        /// Journal name
        #[arg()]
        #[cfg_attr(feature = "dynamic-completion", arg(add = ArgValueCompleter::new(complete_journal_names)))]
        name: String,
    },
    /// Unarchive a journal
    Unarchive {
        /// Journal name
        #[arg()]
        #[cfg_attr(feature = "dynamic-completion", arg(add = ArgValueCompleter::new(complete_archived_journal_names)))]
        name: String,
    },
    /// Export journal JSON to stdout or file
    Export {
        /// Journal name
        #[arg()]
        #[cfg_attr(feature = "dynamic-completion", arg(add = ArgValueCompleter::new(complete_journal_names)))]
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
    /// Generate shell completion script
    #[cfg_attr(
        not(feature = "dynamic-completion"),
        command(after_help = "\
ACTIVATION:
  bash:       eval \"$(COMPLETE=bash foray)\"
              # or persist: COMPLETE=bash foray >> ~/.bash_completion

  zsh:        Add to ~/.zshrc, AFTER compinit:
                eval \"$(COMPLETE=zsh foray)\"
              # Example ~/.zshrc order:
              #   autoload -Uz compinit && compinit
              #   eval \"$(COMPLETE=zsh foray)\"

  fish:       COMPLETE=fish foray | source
              # or persist: COMPLETE=fish foray > ~/.config/fish/completions/foray.fish

  powershell: & { $env:COMPLETE='powershell'; foray } | Invoke-Expression
              # or append the output to $PROFILE

  elvish:     eval (COMPLETE=elvish foray | slurp)
              # or persist: COMPLETE=elvish foray > ~/.config/elvish/lib/foray-complete.elv
              #   then add to rc.elv: use foray-complete

NOTE: this binary completes subcommands and flags only.
For store and journal name completion, rebuild with:
  cargo build --features dynamic-completion
")
    )]
    #[cfg_attr(
        feature = "dynamic-completion",
        command(after_help = "\
ACTIVATION (completes subcommands, flags, store names and journal names):
  bash:       eval \"$(COMPLETE=bash foray)\"
              # or persist: COMPLETE=bash foray >> ~/.bash_completion

  zsh:        Add to ~/.zshrc, AFTER compinit:
                eval \"$(COMPLETE=zsh foray)\"
              # Example ~/.zshrc order:
              #   autoload -Uz compinit && compinit
              #   eval \"$(COMPLETE=zsh foray)\"

  fish:       COMPLETE=fish foray | source
              # or persist: COMPLETE=fish foray > ~/.config/fish/completions/foray.fish

  powershell: & { $env:COMPLETE='powershell'; foray } | Invoke-Expression
              # or append the output to $PROFILE

  elvish:     eval (COMPLETE=elvish foray | slurp)
              # or persist: COMPLETE=elvish foray > ~/.config/elvish/lib/foray-complete.elv
              #   then add to rc.elv: use foray-complete
")
    )]
    Completions {
        /// Shell to generate completions for
        shell: Shell,
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

/// Resolve which store to use: CLI flag > FORAY_STORE env > .forayrc current-store >
/// implicit default (only when exactly one store is configured) > error.
pub fn resolve_store<'a>(
    registry: &'a StoreRegistry,
    cli_flag: Option<&str>,
) -> anyhow::Result<&'a dyn Store> {
    let name: Option<String> = if let Some(n) = cli_flag {
        Some(n.to_string())
    } else if let Ok(n) = std::env::var("FORAY_STORE")
        && !n.is_empty()
    {
        Some(n)
    } else {
        find_store_in_forayrc(&std::env::current_dir()?)
    };

    match name {
        None => {
            if registry.entries().len() == 1 {
                Ok(registry.default_store().as_ref())
            } else {
                Err(anyhow::anyhow!(
                    "no store specified. Use --store <name>, set FORAY_STORE, or add current-store to .forayrc (available: {})",
                    registry.names_hint()
                ))
            }
        }
        Some(n) => registry.get(&n).map(|s| s.as_ref()).ok_or_else(|| {
            anyhow::anyhow!(
                "store '{n}' not found. Available: {}",
                registry.names_hint()
            )
        }),
    }
}

/// Walk up from `start_dir` looking for `.forayrc` with `current-store`.
pub fn find_store_in_forayrc(start_dir: &std::path::Path) -> Option<String> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let rc_path = dir.join(".forayrc");
        if rc_path.is_file()
            && let Ok(contents) = std::fs::read_to_string(&rc_path)
            && let Ok(table) = contents.parse::<toml::Table>()
        {
            if let Some(name) = table.get("current-store").and_then(|v| v.as_str()) {
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
pub fn write_forayrc(name: &str, store: Option<&str>) -> anyhow::Result<()> {
    let rc_path = std::env::current_dir()?.join(".forayrc");
    let mut table = if rc_path.is_file() {
        let contents = std::fs::read_to_string(&rc_path)?;
        contents.parse::<toml::Table>().unwrap_or_default()
    } else {
        toml::Table::new()
    };
    table.insert("current-journal".into(), toml::Value::String(name.into()));
    if let Some(s) = store {
        table.insert("current-store".into(), toml::Value::String(s.into()));
    }
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

// ── Shell completion candidates ──────────────────────────────────────

#[cfg(feature = "dynamic-completion")]
fn complete_store_names(_current: &OsStr) -> Vec<CompletionCandidate> {
    let Ok(registry) = StoreRegistry::load() else {
        return vec![];
    };
    registry
        .entries()
        .iter()
        .map(|e| CompletionCandidate::new(&e.name))
        .collect()
}

#[cfg(feature = "dynamic-completion")]
fn complete_journal_names(_current: &OsStr) -> Vec<CompletionCandidate> {
    journal_names_as_candidates(false)
}

#[cfg(feature = "dynamic-completion")]
fn complete_archived_journal_names(_current: &OsStr) -> Vec<CompletionCandidate> {
    journal_names_as_candidates(true)
}

#[cfg(feature = "dynamic-completion")]
fn journal_names_as_candidates(archived: bool) -> Vec<CompletionCandidate> {
    let Ok(exe) = std::env::current_exe() else {
        return vec![];
    };
    let store_name = std::env::var("FORAY_STORE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .and_then(|d| find_store_in_forayrc(&d))
        });
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("list").arg("--completion");
    if archived {
        cmd.arg("--archived");
    }
    if let Some(store) = &store_name {
        cmd.arg("--store").arg(store);
    }
    // Unset COMPLETE so the subprocess runs normally rather than completing.
    cmd.env_remove("COMPLETE");
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    let Ok(mut child) = cmd.spawn() else {
        return vec![];
    };
    let stdout = child.stdout.take();

    // Read stdout in a thread so we can enforce a timeout on slow/remote stores.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut out) = stdout {
            use std::io::Read;
            let _ = out.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });

    let buf = match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(buf) => buf,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return vec![];
        }
    };
    let _ = child.wait();

    let Ok(stdout) = String::from_utf8(buf) else {
        return vec![];
    };
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(CompletionCandidate::new)
        .collect()
}

fn print_item(item: &JournalItem) {
    let type_str = format!("{:?}", item.item_type).to_lowercase();
    println!(
        "[{}] ({}) {}",
        item.added_at.format("%Y-%m-%d %H:%M"),
        type_str,
        item.content
    );
    if let Some(r) = item
        .meta
        .as_ref()
        .and_then(|m| m.get("ref"))
        .and_then(|v| v.as_str())
    {
        println!("  ref: {r}");
    }
    if let Some(tags) = &item.tags {
        println!("  tags: {}", tags.join(", "));
    }
}

/// Execute a CLI command against the store.
pub async fn run(cli: &Cli, store: &dyn Store) -> anyhow::Result<()> {
    match &cli.command {
        Commands::Serve => {
            unreachable!("serve is handled in main")
        }
        Commands::Completions { .. } => {
            unreachable!("completions is handled in main")
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
            let (journal, total) = store.load(&journal_name, &pagination).await?;
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
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let all = Pagination::default();
                    let (journal, new_total) = store.load(&journal_name, &all).await?;
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
            let mut parsed_meta = parse_meta(meta);
            if let Some(r) = file_ref {
                parsed_meta
                    .get_or_insert_with(HashMap::new)
                    .entry("ref".to_string())
                    .or_insert_with(|| serde_json::Value::String(r.clone()));
            }
            let item = JournalItem {
                id: item_id(),
                item_type: it,
                content: content.clone(),
                tags: parsed_tags,
                added_at: Utc::now(),
                meta: parsed_meta,
            };
            let count = store.add_items(&journal_name, vec![item]).await?;
            println!("Added to {journal_name} ({count} items)");
        }
        Commands::Open { name, title, meta } => {
            validate_name(name).map_err(|e| anyhow::anyhow!(e))?;
            let meta = parse_meta(meta);
            let exists = store.exists(name).await?;

            if exists {
                println!("Journal already exists: {name}");
            } else {
                let title = title.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("--title is required when creating a new journal")
                })?;
                store.create(name, Some(title.clone()), meta).await?;
                println!("Created journal: {name}");
            }
            write_forayrc(name, cli.store.as_deref())?;
            println!("Set active journal in .forayrc");
        }
        Commands::List {
            json,
            archived,
            limit,
            offset,
            completion,
        } => {
            if *completion {
                let (summaries, _) = store.list(&Pagination::default(), *archived).await?;
                for s in &summaries {
                    println!("{}", s.name);
                }
                return Ok(());
            }
            let pagination = Pagination {
                limit: *limit,
                offset: *offset,
            };
            let (summaries, total) = store.list(&pagination, *archived).await?;

            if *json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"total": total, "journals": &summaries})
                    )?
                );
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
            store.archive(name).await?;
            println!("Archived: {name}");
        }
        Commands::Unarchive { name } => {
            store.unarchive(name).await?;
            println!("Unarchived: {name}");
        }
        Commands::Export { name, file } => {
            let (journal, _) = store.load(name, &Pagination::default()).await?;
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
            let raw: serde_json::Value = serde_json::from_str(&data)?;
            let value = match migrate::migrate(raw) {
                MigrateResult::Current(v) | MigrateResult::Migrated(v) => v,
                MigrateResult::TooNew { found, max } => {
                    return Err(anyhow::anyhow!(
                        "journal schema {found} is too new (max supported: {max})"
                    ));
                }
                MigrateResult::Invalid => {
                    return Err(anyhow::anyhow!("journal file is not a JSON object"));
                }
            };
            let journal: JournalFile = serde_json::from_value(value)?;
            validate_name(&journal.name).map_err(|e| anyhow::anyhow!(e))?;
            let name = journal.name;
            let items = journal.items;
            store.create(&name, journal.title, journal.meta).await?;
            if !items.is_empty() {
                store.add_items(&name, items).await?;
            }
            println!("Imported successfully");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize tests that mutate process-global state (env vars, cwd) to avoid races.
    static SERIAL_LOCK: Mutex<()> = Mutex::new(());

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
    fn write_forayrc_persists_store_when_given() {
        let _guard = SERIAL_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        write_forayrc("my-journal", Some("remote")).unwrap();
        std::env::set_current_dir(&orig).unwrap();
        let contents = std::fs::read_to_string(dir.path().join(".forayrc")).unwrap();
        let table: toml::Table = contents.parse().unwrap();
        assert_eq!(table["current-journal"].as_str(), Some("my-journal"));
        assert_eq!(table["current-store"].as_str(), Some("remote"));
    }

    #[test]
    fn write_forayrc_omits_store_when_none() {
        let _guard = SERIAL_LOCK.lock().unwrap();
        let dir = tempfile::TempDir::new().unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        write_forayrc("my-journal", None).unwrap();
        std::env::set_current_dir(&orig).unwrap();
        let contents = std::fs::read_to_string(dir.path().join(".forayrc")).unwrap();
        let table: toml::Table = contents.parse().unwrap();
        assert_eq!(table["current-journal"].as_str(), Some("my-journal"));
        assert!(!table.contains_key("current-store"));
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

    fn make_registry() -> (StoreRegistry, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let registry = StoreRegistry::for_test(dir.path().to_path_buf());
        (registry, dir)
    }

    #[test]
    fn resolve_store_uses_cli_flag() {
        let (registry, _dir) = make_registry();
        // "local" is the default name in for_test
        assert!(resolve_store(&registry, Some("local")).is_ok());
    }

    #[test]
    fn resolve_store_cli_flag_beats_env_var() {
        let _guard = SERIAL_LOCK.lock().unwrap();
        let (registry, _dir) = make_registry();
        unsafe {
            std::env::set_var("FORAY_STORE", "some-other-store");
        }
        let result = resolve_store(&registry, Some("local"));
        unsafe {
            std::env::remove_var("FORAY_STORE");
        }
        // CLI flag wins even though FORAY_STORE is set to an unknown store
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_store_cli_flag_unknown_errors() {
        let (registry, _dir) = make_registry();
        let result = resolve_store(&registry, Some("no-such-store"));
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("not found"));
        assert!(msg.contains("no-such-store"));
    }

    #[test]
    fn resolve_store_env_var() {
        let _guard = SERIAL_LOCK.lock().unwrap();
        let (registry, _dir) = make_registry();
        // env var wins when no CLI flag
        unsafe {
            std::env::set_var("FORAY_STORE", "local");
        }
        let result = resolve_store(&registry, None);
        unsafe {
            std::env::remove_var("FORAY_STORE");
        }
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_store_env_var_unknown_errors() {
        let _guard = SERIAL_LOCK.lock().unwrap();
        let (registry, _dir) = make_registry();
        unsafe {
            std::env::set_var("FORAY_STORE", "nope");
        }
        let result = resolve_store(&registry, None);
        unsafe {
            std::env::remove_var("FORAY_STORE");
        }
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("not found"));
    }

    #[test]
    fn resolve_store_forayrc() {
        let (_registry, rc_dir) = make_registry();
        let rc_path = rc_dir.path().join(".forayrc");
        std::fs::write(&rc_path, "current-store = \"local\"\n").unwrap();
        let found = find_store_in_forayrc(rc_dir.path());
        assert_eq!(found, Some("local".to_string()));
    }

    #[test]
    fn resolve_store_falls_back_to_default() {
        let _guard = SERIAL_LOCK.lock().unwrap();
        let (registry, dir) = make_registry();
        // Stop find_store_in_forayrc() from walking up into parent dirs.
        std::fs::write(dir.path().join(".forayrc"), "root = true\n").unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let prior_foray_store = std::env::var("FORAY_STORE").ok();
        unsafe {
            std::env::remove_var("FORAY_STORE");
        }
        // Single-store registry: implicit default is returned.
        let result = resolve_store(&registry, None);
        std::env::set_current_dir(&orig).unwrap();
        unsafe {
            match prior_foray_store {
                Some(v) => std::env::set_var("FORAY_STORE", v),
                None => std::env::remove_var("FORAY_STORE"),
            }
        }
        assert!(result.is_ok());
    }

    #[test]
    fn resolve_store_errors_without_spec_when_multiple_stores() {
        let _guard = SERIAL_LOCK.lock().unwrap();
        let dir1 = tempfile::TempDir::new().unwrap();
        let dir2 = tempfile::TempDir::new().unwrap();
        let registry =
            StoreRegistry::for_test_two(dir1.path().to_path_buf(), dir2.path().to_path_buf());
        // Stop find_store_in_forayrc() from walking up into parent dirs.
        std::fs::write(dir1.path().join(".forayrc"), "root = true\n").unwrap();
        let orig = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir1.path()).unwrap();
        let prior_foray_store = std::env::var("FORAY_STORE").ok();
        unsafe {
            std::env::remove_var("FORAY_STORE");
        }
        let result = resolve_store(&registry, None);
        std::env::set_current_dir(&orig).unwrap();
        unsafe {
            match prior_foray_store {
                Some(v) => std::env::set_var("FORAY_STORE", v),
                None => std::env::remove_var("FORAY_STORE"),
            }
        }
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(msg.contains("no store specified"));
        assert!(msg.contains("available:"));
    }

    #[test]
    fn find_store_in_forayrc_root_stops_walk() {
        let dir = tempfile::TempDir::new().unwrap();
        let rc_path = dir.path().join(".forayrc");
        std::fs::write(&rc_path, "root = true\n").unwrap();
        let child = dir.path().join("sub");
        std::fs::create_dir(&child).unwrap();
        assert_eq!(find_store_in_forayrc(&child), None);
    }
}
