# Plan: hunch — Investigation Journals via MCP

## TL;DR
A **Rust MCP server** that gives any AI assistant (Copilot, Claude Desktop, Cursor, etc.) persistent, forkable investigation journals. Start with a hunch, fork it when it branches, keep the trail. Contexts are named collections of findings/decisions/notes scoped to a project. Git branch detection is a convenience layer, not the core.

**Tagline:** *"Start with a hunch. Fork it when it branches. Keep the trail."*

## Positioning (for README)
In a space of 100+ AI memory tools, hunch is not "yet another memory server." It's the **only MCP server where you can fork an investigation.** Differentiators:
- **Forking with lineage** — first-class `fork_context` with `forked_from` tracking. No other tool has this.
- **Radical simplicity** — 7 tools (competitors have 22-38). Zero runtime deps.
- **Human-editable** — pretty-printed JSON files you can `cat`, `jq`, `grep`, hand-edit.
- **Single Rust binary** — no Node, Python, Ollama, or SQLite.
- **Investigation-first** — not generic "memory" (100+ exist) or "sessions" (temporal). Named journals with typed items.
- **Universal** — any MCP client, cross-window via filesystem.

Closest competitor: `mcp-memory-keeper` (session branching buried in 38 tools, SQLite, Claude-focused). We're simpler, portable, and investigation-structured.

## Architecture

```
Any MCP Client (VS Code, Claude Desktop, Cursor, ...)
    │  stdio
    ▼
┌────────────────┐
│     hunch      │──► ~/.hunch/
│  (Rust binary)  │       └── <project>/
└────────────────┘            ├── .active
                              ├── auth-investigation.json
                              └── perf-deep-dive.json
```

## Core Abstractions

| Concept | Description |
|---------|-------------|
| **Project** | A namespace. Auto-detected from git repo name or directory name. |
| **Context** | A named, forkable collection of items. Like an investigation journal. |
| **Item** | A finding, decision, snippet, or note inside a context. |
| **Active context** | The currently selected context for the project. Reads/writes go here by default. |
| **Fork** | Snapshot-copy a context under a new name. Child gets all parent's items at fork time. `parent` field tracks lineage but has no runtime behavior. |

Git is optional: if available, branch name is suggested as default context name and auto-switch is possible. Without git, everything still works.

## Tech Stack
- **Language**: Rust (new for the user, challenge requirement)
- **MCP SDK**: `rmcp` v1.4.0 (official, `#[tool]`/`#[tool_router]` macros, tokio, stdio)
- **Deps**: `rmcp`, `tokio`, `serde`/`serde_json`, `uuid`, `chrono`, `dirs`, `anyhow`, `clap`

## Storage

```
~/.hunch/
  └── kibana/                        ← project (from git repo name or dirname)
      ├── .active                    ← plain text: name of active context
      ├── auth-investigation.json
      ├── perf-deep-dive.json
      └── main-cleanup.json
```

### Context file format (self-contained — all items in one file):
```json
{
  "_note": "Edit this file freely. Each file is self-contained.",
  "name": "auth-deep-dive",
  "project": "kibana",
  "parent": "auth-triage",
  "items": [
    {
      "id": "a1b2c3d4",
      "type": "finding",
      "content": "Race condition in auth cache — token refresh and session check compete for the same lock",
      "ref": "src/auth/session.go:142",
      "tags": ["auth", "race-condition"],
      "added_at": "2026-04-15T10:15:00Z"
    }
  ],
  "created_at": "2026-04-15T10:00:00Z",
  "updated_at": "2026-04-15T14:30:00Z"
}
```

A root context (no parent) works identically — its file contains all its items.

### `.active` file:
Plain text, single line: `auth-investigation`

## MCP Tools (7 tools)

| Tool | Description | Default Behavior |
|------|-------------|-----------------|
| `get_context` | Read all items in a context | Active context if no name given |
| `add_item` | Add a finding/decision/snippet/note | To active context |
| `fork_context` | Copy a context under a new name | From active context |
| `switch_context` | Set active context (create if new) | — |
| `list_contexts` | List all contexts in the project | — |
| `remove_item` | Delete a specific item by ID | From active context |
| `get_status` | Return project name, active context, git branch (if available) | — |

### Tool Details

**Response format**: All MCP tools return JSON-serialized structs. No human formatting in the server — the LLM presents data as it sees fit. CLI has its own human-friendly formatting in `cli.rs`.

**`get_context`**
- Params: `name: Option<String>`
- Reads a single file — no chain resolution
- Returns JSON: `{ name, parent, items: [{ id, type, content, ref, tags, added_at }], item_count }`
- Default: active context

**`add_item`**
- Params: `content: String`, `item_type: Option<String>` (finding/decision/snippet/note, default: note), `file_ref: Option<String>`, `tags: Option<String>` (comma-separated)
- Returns JSON: `{ id, context, item_count }`
- Writes to active context

**`fork_context`**
- Params: `new_name: String`, `from: Option<String>` (default: active)
- **Snapshot-copies** all items from source into a new context with `parent` set
- Switches active to the new context
- Returns JSON: `{ name, parent, item_count }`

**`switch_context`**
- Params: `name: String`
- Creates new empty context if name doesn't exist
- Updates `.active` file
- Returns JSON: `{ name, item_count, created }`

**`list_contexts`**
- No params
- Returns JSON: `{ contexts: [{ name, parent, item_count, active }], tree: "ascii rendered tree" }`
- Tree field shows fork lineage for convenience

**`remove_item`**
- Params: `item_id: String`
- Removes item by ID from active context
- Returns JSON: `{ removed: true, id, context }` or error if not found

**`get_status`**
- No params
- Returns JSON: `{ project, active_context, item_count, git_branch }`

## Steps

### Phase 1: Scaffold (30 min)
1. `cargo init .` in repo
2. Cargo.toml deps: `rmcp`, `tokio`, `serde`, `serde_json`, `uuid`, `chrono`, `dirs`, `anyhow`, `clap` (with `derive` feature)
3. Module structure:
   - **Library** (`src/lib.rs` re-exports): `types.rs`, `store.rs`, `tree.rs`, `git.rs` — all logic, fully unit-testable
   - **Thin wrappers**: `main.rs` (entry + routing), `cli.rs` (clap defs + output formatting), `server.rs` (MCP tool defs → call lib)

### Phase 1b: CLI Skeleton (30 min)
4. `cli.rs` — `clap` derive-based subcommands (thin wrapper, no logic):
   ```
   hunch serve [--workspace <path>]   # Start MCP stdio server
   hunch status [--json]              # Project, active context, branch
   hunch show [name] [--json]         # Full context with items
   hunch add <content> [--type finding|decision|snippet|note] [--ref <file>] [--tags <csv>]
   hunch fork <new-name> [--from <name>]
   hunch switch <name>
   hunch list [--json]                # Tree view (plain) or array (json)
   hunch remove <item-id>
   ```
   - Each subcommand: parse args → call lib → print formatted output to stdout
   - `--json` flag on read commands: outputs raw JSON (same structs as MCP responses). No color, no extra crate — plain text + ASCII tree by default.
5. `main.rs`:
   - Parse CLI with clap
   - Auto-detect project name via `git::detect_project(cwd)`
   - Construct `JsonFileStore`
   - `serve` → create `HunchServer` with store, `serve(stdio()).await`
   - All other subcommands → call lib functions with store, print to stdout

### Phase 2: Types + Store (1.25 hr)
4. `types.rs`:
   - `ContextFile` { `_note`, name, project, parent, items, created_at, updated_at } — `_note` first for JSON field ordering
   - `ContextItem` { id, item_type, content, file_ref, added_at, tags }
     - `file_ref: Option<String>` with `#[serde(rename = "ref", skip_serializing_if = "Option::is_none")]`
     - `tags: Option<Vec<String>>` with `#[serde(skip_serializing_if = "Option::is_none")]`
   - `ItemType` enum { Finding, Decision, Snippet, Note } with `#[serde(rename_all = "lowercase")]`
   - `ContextSummary` { name, item_count, active, parent }
   - `parent: Option<String>` with `#[serde(skip_serializing_if = "Option::is_none")]` on `ContextFile`
   - `_note: Option<String>` — always set on save, `#[serde(skip_serializing_if = "Option::is_none")]`
   - No `#[serde(deny_unknown_fields)]` — tolerant of hand-edited files with extra fields
   - All structs: `#[derive(Debug, Clone, Serialize, Deserialize)]`, items also `schemars::JsonSchema` where needed
5. `store.rs` — **trait + impl pattern**:
   - Custom error type:
     ```
     enum StoreError {
         NotFound(String),
         AlreadyExists(String),
         InvalidName(String),
         Io(std::io::Error),
         Parse(String),
     }
     ```
   - `trait Store: Send + Sync`:
     - `fn load(&self, name: &str) -> Result<ContextFile, StoreError>`
     - `fn save(&self, ctx: &ContextFile) -> Result<(), StoreError>` — full write (for fork, create)
     - `fn add_item(&self, name: &str, item: ContextItem) -> Result<(), StoreError>` — atomic read-append-write
     - `fn remove_item(&self, name: &str, item_id: &str) -> Result<bool, StoreError>` — atomic read-filter-write, returns false if ID not found
     - `fn list(&self) -> Result<Vec<ContextSummary>, StoreError>`
     - `fn delete(&self, name: &str) -> Result<(), StoreError>`
     - `fn get_active(&self) -> Result<Option<String>, StoreError>` — None if no `.active` file
     - `fn set_active(&self, name: &str) -> Result<(), StoreError>`
     - `fn exists(&self, name: &str) -> Result<bool, StoreError>`
   - `JsonFileStore` implements `Store`:
     - `JsonFileStore::new(base_dir, project)` — `base_dir` defaults to `~/.hunch/`
     - Creates project dir on first write (`ensure_project_dir()`)
     - `save()` — atomic write (tmp + fsync + rename). Used for fork/create (new files, no merge needed).
     - `add_item()` — read file, append item, atomic write. Two concurrent adds = both preserved.
     - `remove_item()` — read file, filter out ID, atomic write. Two concurrent removes = both applied.
     - `.active` file: direct write (single line, no merge needed)
     - `list()` scans dir for `.json` files, skipping `.tmp_*` and `.active`
   - `fork()` — free function in lib: `store.load(source)`, clone items, set `parent`/new name/timestamps, `store.save(new_ctx)`. Snapshot copy.
   - The server holds `Arc<dyn Store + Send + Sync>` — `Arc` required because `#[derive(Clone)]` on the server struct. No `Mutex` needed since `Store` methods are sync and file writes are atomic.
   - `build_tree(summaries) -> String` — in `tree.rs`. Renders ASCII tree from `list()` results using `parent` field for lineage. Used by `list_contexts`/`hunch list`.

### Phase 3: Git Detection (30 min)
6. `git.rs`:
   - `detect_project(workspace_path) -> String` — try git repo name, fallback to dir name
   - `detect_branch(workspace_path) -> Option<String>` — `git rev-parse --abbrev-ref HEAD`
   - Used for: project auto-naming, `get_status` output, suggesting context names

### Phase 4: MCP Server (1 hr)
7. `server.rs` — `HunchServer` struct, thin wrapper:
   ```
   #[tool_router(server_handler)]
   impl HunchServer { ... 7 tools ... }
   ```
   Each tool: parse MCP params → call lib functions (store CRUD, build_tree, fork, git) → return JSON via `CallToolResult::success(Content::text(serde_json::to_string(...)))`
   Server holds `Arc<dyn Store + Send + Sync>`, project name, workspace path. No logic beyond param parsing + response formatting.

### Phase 5: Config, Companion Skill + README (1.5 hr)
9. Config examples for 3 clients:
   - `.vscode/mcp.json` for VS Code Copilot
   - `claude_desktop_config.json` for Claude Desktop
   - Cursor MCP settings
10. **Companion Agent Skill** — `skills/hunch/SKILL.md`:
    - Follows open Agent Skills standard (agentskills.io)
    - Frontmatter: `name: hunch`, `description`, `user-invocable: false` (auto-triggered, not a slash command)
    - Body instructs agents to:
      - Call `get_status` at conversation start to check for active investigation context
      - Call `get_context` to load existing findings before starting work
      - Call `add_item` when discovering something significant (findings, decisions, code refs)
      - Use `ref` field for file paths, URLs, ticket links, PR links — anything that locates the source
        - Example: finding with `ref: "https://github.com/org/repo/pull/87"` for a PR that introduced a regression
        - Example: note with `ref: "https://jira.example.com/browse/PROJ-1234"` to link a tracking ticket
      - Suggest `fork_context` when changing investigation direction
      - Call `switch_context` when user moves to a different topic
    - Ships in repo at `skills/hunch/SKILL.md` (project skill)
    - User can also copy to `~/.copilot/skills/hunch/` for personal use across all projects
11. README per challenge template:
    - Tagline: "Start with a hunch. Fork it when it branches. Keep the trail."
    - Problem (investigation context lost), Why It Matters, Language (Rust), How Cursor Helped
    - Competitive positioning: not "another memory tool" — the only one with forking
    - Multi-client config examples
    - Companion skill installation instructions (project vs personal)
    - Example session transcript showing the tools in action

### Phase 6: CI + Test + Polish (1.5 hr)
12. **GitHub Actions CI** — `.github/workflows/ci.yml`:
    - Trigger: push + PR to main
    - Matrix: `ubuntu-latest`, `macos-latest`, `windows-latest`
    - Steps: checkout → install Rust (stable) → `cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test` → `cargo build --release`
    - Artifact: upload release binaries per platform
13. `cargo test` — unit tests for store (CRUD, add_item, remove_item, fork, concurrent adds), tree rendering, git detection
14. `cargo build --release`
15. Manual end-to-end test via CLI + in VS Code (or Claude Desktop)
16. Record 5-min demo video

## Verification
1. `cargo test` — all store (CRUD, merge-on-save, fork), tree, git tests pass
2. `cargo build --release` — clean binary
3. **CLI smoke test** (no MCP client needed):
   - `hunch switch auth-triage` → creates root context
   - `hunch add "auth is broken" --type finding` → persists item
   - `hunch fork auth-deep-dive` → snapshot copy, new context has parent's items
   - `hunch add "race condition in token refresh" --type finding` → adds to child only
   - `hunch show` → shows all items in child (copied + new)
   - `hunch show auth-triage` → shows only parent's items (unchanged since fork)
   - `hunch list` → tree with fork lineage
   - `hunch status` → project, active, branch
4. **MCP integration**:
   - Configure in VS Code → tools appear in Copilot tool list
   - "What am I working on?" → `get_status`
   - "Save this finding" → `add_item`
   - "Fork this to auth-deep-dive" → `fork_context` snapshot copies
   - Add item to parent after fork → child is unaffected (snapshot)
   - "List my contexts" → tree view showing lineage
   - Open new window → same active context
   - Configure Claude Desktop → same tools work
5. **Companion skill**: Place `skills/hunch/SKILL.md` → Copilot auto-loads it
6. Verify skill in VS Code Chat Customizations editor

## Key Decisions
- **Model B**: contexts are named journals, not git-bound
- **Rust** with official `rmcp` SDK (challenge "new language" rule)
- **stdio transport** — universal, no port management
- **JSON files** — inspectable, no DB dependency
- **Atomic writes** — read-merge-write with rename. Merge by item ID: concurrent adds from multiple windows are preserved (union). No `edit_item` tool = no true conflicts. `.active` is direct write.
- **Store trait** — sync `trait Store: Send + Sync` with `JsonFileStore` impl; server holds `Arc<dyn Store>`. Atomic `add_item`/`remove_item` methods handle concurrency (read-modify-write per operation). `save()` for full writes (fork/create). Custom `StoreError` enum. `fork()` stays outside the trait as a free function.
- **Git as convenience** — auto-detect project name, show branch in status, not required
- **Out of scope**: UI, auto-summarization, remote sync, `edit_item` tool
- **Companion skill**: `user-invocable: false` so it's auto-triggered by the agent, not a slash command — the whole point is zero-friction adoption
- **Name**: "hunch" — short, memorable, investigation-evocative. Tagline: "Start with a hunch. Fork it when it branches. Keep the trail."
- **Positioning**: Not "another memory tool" (100+ exist). The only MCP server where you can fork an investigation.
- **CLI + MCP**: Single binary, `clap` subcommands. `hunch serve` = MCP stdio server, all other subcommands = direct store access. Both are thin wrappers over the library.
- **Lib-first architecture**: All logic in `lib.rs` modules (`types`, `store`, `tree`, `git`). CLI and MCP server are I/O shells — parse input, call lib, format output. Library is fully unit-testable without MCP or terminal.
- **Snapshot fork**: Fork = full copy of items at fork time. `parent` field is lineage metadata only, no runtime behavior. Files are self-contained.
- **CI**: GitHub Actions, 3-platform matrix (Linux, macOS, Windows). fmt + clippy + test + release build.
- **License**: Apache 2.0

## Resolved Design Questions
- **Context name validation**: Strict `[a-z0-9_-]` only. Reject everything else at creation time.
- **First use (no active context)**: Explicit — return "No active context. Use switch_context to create one." No auto-create.
- **Item counts**: Single count per context (files are self-contained).
- **Concurrency**: `add_item`/`remove_item` are atomic read-modify-write. Concurrent adds from multiple windows are preserved. No `edit_item` = no true conflicts.
- **`ref` field**: `file_ref` in Rust, `"ref"` in JSON via serde rename. Free-form string — file paths, URLs, ticket/PR links.
- **Serde**: tolerant deserialization (no `deny_unknown_fields`), `_note` first in struct for JSON ordering, `Option` fields skipped when None.
- **CLI output**: plain text + ASCII tree by default, `--json` flag on read commands.
- **MCP responses**: JSON-serialized structs. LLM formats for the user.
- **`rmcp` pattern**: `#[derive(Clone)]` server, `Arc<dyn Store + Send + Sync>`, `Parameters<T>` for tool args, `CallToolResult::success(Content::text(...))` for returns.
