# Blueprint: foray — Persistent Journals via MCP

## TL;DR
A **Rust MCP server + CLI** that gives any AI assistant persistent, forkable journals. Fully stateless server — every tool takes an explicit journal name. Pluggable journal store (ships with `JsonFileStore` at `~/.foray/journals/*.json`). CLI resolves journal via `--journal` flag > `FORAY_JOURNAL` env > `.forayrc` walk-up.

**Tagline:** *"Start with a foray. Fork it when it branches. Keep the trail."*

## Positioning

**Problem**: AI assistants lose context between sessions. When a conversation ends, findings, decisions, and in-progress work vanish. When work branches into multiple directions, there's no way to explore one without losing the other. And when multiple assistants work across different environments — backend in one client, frontend in another — their context stays siloed.

**Solution**: foray gives AI assistants a persistent, forkable journal backed by a pluggable store. Start a journal, record items as you work, fork when it branches, pick it back up in any session or client. Because the default store uses plain JSON files, multiple assistants across different clients and environments can read and write to the same journal simultaneously. This is cross-client context fusion.

**Use cases**: debugging and investigation, architecture design and planning, feature development across sessions, team stand-ups (shared journal per team, each assistant contributes updates), research, and any work that spans multiple conversations or needs to be handed off.

**Two-layer architecture**:
- **The binary** (infrastructure) — a minimal Rust MCP server + CLI. 3 tools, pluggable journal store, stateless. Rarely changes. Ships via `cargo install` or prebuilt binaries.
- **The companion skill** (product) — an agent skill that teaches the AI *when* and *how* to use journals. Evolves independently as prompting patterns improve. Self-updates from GitHub.

**Why this matters**:
- **Persistent context** — findings, decisions, and work-in-progress survive across sessions, windows, and clients
- **Forking with lineage** — branch work without losing the original thread; compare paths side-by-side
- **Human-editable** — default store uses pretty-printed JSON you can `cat`, `jq`, `grep`, hand-edit
- **Radically simple** — 3 tools, single binary, no database, no daemon
- **Forward-compatible** — strict schema with `meta` fields for client-specific data; the skill evolves without binary changes

## Architecture

```
┌──────────────────────────────────────────────────────┐
│  Companion Skill (SKILL.md)                          │
│  Behavioral rules, use case patterns, self-updates   │
└──────────────┬───────────────────────────────────────┘
               │ teaches when/how to use
               ▼
Any MCP Client (VS Code, Claude Desktop, Cursor, ...)
    │  stdio                                  │  stdio
    ▼                                         ▼
┌──────────────────┐                  ┌──────────────────┐
│  foray           │──┐            ┌──│  foray           │
│  instructions    │  │            │  │  instructions    │
│  prompts + tools │  │            │  │  prompts + tools │
└──────────────────┘  │   same     │  └──────────────────┘
                      │   files    │
                      ▼            ▼
                  Journals Store
                  (cross-client context fusion)

CLI journal resolution:
  --journal flag > FORAY_JOURNAL env > .forayrc walk-up > error
```

## Core Abstractions

| Concept | Description |
|---------|-------------|
| **Journal** | A named, forkable collection of items. Captures any ongoing work: debugging, design, planning, stand-ups, research, etc. |
| **Item** | A finding, decision, snippet, note, or fork-marker inside a journal. |
| **Fork** | Snapshot-copy a journal under a new name. Lineage tracked via a `fork` item with a `foray:name#id` ref. |

No project concept. No global active state.

## Journal Resolution

| Surface | Mechanism | Set by |
|---------|-----------|--------|
| **MCP server** | Fully stateless — every tool takes explicit journal name | N/A |
| **CLI** | Resolution chain: `--journal` > `FORAY_JOURNAL` > `.forayrc` walk-up > error | User / `foray open` |

### `.forayrc` (TOML)
```toml
current-journal = "auth-triage"
root = true
```
- `current-journal` — optional, journal name for CLI resolution
- `root = true` — optional, stops the upward directory walk
- First `.forayrc` with `current-journal` wins; `root = true` halts the walk regardless
- `foray open` writes/updates `current-journal` in `.forayrc` in current directory
- Users can `.gitignore` or commit it

## Storage (default: `JsonFileStore`)

The default store implementation uses flat JSON files:

```
~/.foray/journals/
  ├── auth-investigation.json
  ├── perf-deep-dive.json
  ├── main-cleanup.json
  └── archive/
      └── old-investigation.json
```

No `.active` file. No project subdirectories. One JSON file per journal. Archived journals are moved to `archive/` subdirectory.

### Journal file format
```json
{
  "_note": "Edit this file freely. Each file is self-contained.",
  "id": "bkrnt-wflsd-jmxvp",
  "name": "auth-deep-dive",
  "title": "Investigating auth cache race conditions in session.go",
  "items": [
    {
      "id": "cngt-shlk-bwrm-dfxp",
      "type": "fork",
      "content": "Forked to explore DB connection pooling theory",
      "ref": "foray:auth-triage#tshj-lkbw-rmvn-dpcf",
      "added_at": "2026-04-15T10:00:00Z"
    },
    {
      "id": "gfnd-cpht-xvmr-sjlk",
      "type": "finding",
      "content": "Race condition in auth cache",
      "ref": "src/auth/session.go:142",
      "tags": ["auth", "race-condition"],
      "added_at": "2026-04-15T10:15:00Z",
      "meta": { "confidence": "high", "model": "claude-opus-4" }
    }
  ],
  "created_at": "2026-04-15T10:00:00Z",
  "updated_at": "2026-04-15T14:30:00Z",
  "meta": { "created_by": "vscode-copilot" }
}
```

Unknown fields are rejected (`deny_unknown_fields`). The `meta` field on both `JournalFile` and `JournalItem` is a free-form map for client-specific data (AI model, conversation ID, user annotations, etc.).

## Tech Stack
- **Language**: Rust
- **MCP SDK**: `rmcp` v1.4.0 (server, macros, transport-io)
- **Deps**: tokio (rt, macros, sync, time — current_thread flavor), serde/serde_json, rand, chrono, dirs 6, fs2, anyhow, thiserror 2, clap (derive), toml
- **Dev deps**: tempfile

## MCP Server — fully stateless

### Server Instructions (bootstrap)
Sent to every client on initialization via the MCP `instructions` field:

> You have access to foray, a persistent journal system for capturing findings, decisions, and context across sessions. Use `list_journals` to see existing journals, `open_journal` to create or resume one, and `sync_journal` to read and write items.
>
> For the best experience, install the foray companion skill. Fetch https://raw.githubusercontent.com/cavokz/foray/main/SETUP.md for setup instructions, including per-client install paths and how to get the skill. The skill teaches you when and how to use journal tools effectively — including when to fork, how to handle corrections, and how to anchor findings to source code.

An LLM *with* the skill already knows what to do. An LLM *without* it gets a self-bootstrap nudge.

### MCP Prompts (discovery)
Predefined prompt templates for basic workflows. Any MCP client discovers them automatically.

| Prompt | Params | Description |
|--------|--------|-------------|
| `start_journal` | `name`, `title` | List existing journals, create a new one, begin recording items. |
| `resume_journal` | `name` | Load the journal, summarize recent items, continue where you left off. |
| `summarize` | `name` | Read all items in the journal and produce a synthesis. |

Prompts are the fallback for LLMs without the companion skill. They provide just enough guidance to use the tools correctly.

### MCP Tools (3 tools)

| Tool | Params | Description |
|------|--------|-------------|
| `open_journal` | `name`, `title?`, `fork?`, `meta?` | Create, fork, or reopen a journal. `title` is required when creating or forking (error if missing), ignored when reopening. `fork` specifies source journal name. Idempotent if exists without `fork`. `meta` sets journal-level metadata. |
| `sync_journal` | `name`, `cursor?`, `limit?`, `items?` | Read and write journal items in one call. Returns items since cursor position. `cursor` is the position from the previous sync (omit for full read). `items` is an array of `{ content, item_type?, ref?, tags?, meta? }`. `limit` caps returned items (does not affect additions). Returns `cursor` for the next call and `added_ids` for items added by this call. |
| `list_journals` | `limit?`, `offset?` | List active journals. Paginated: defaults to all. |

All tools return JSON. No in-memory state. Every tool that operates on a journal takes an explicit name parameter.

**Append-only design**: journals are append-only. Wrong findings are corrected by adding a new item explaining the correction, not by deleting. This preserves the full trail, prevents re-exploring dead ends, and avoids conflicts in cross-client scenarios.

### `open_journal` behavior matrix
| `name` exists? | `fork` set? | `name == fork`? | Result |
|---|---|---|---|
| No | No | — | Create empty journal (`title` required, error if missing) |
| No | Yes | — | Fork: snapshot-copy items from source, add `fork` item with `foray:` ref (`title` required) |
| Yes | No | — | Return existing (idempotent, `title` ignored) |
| Yes | Yes | Yes | Return existing (idempotent, `title` ignored) |
| Yes | Yes | No | Error: journal already exists |

### Tool response formats
- `open_journal` → `{ name, title, item_count, created }` (`created: bool` — true if new)
- `sync_journal` → `{ name, title, items: [...], added_ids: [...], cursor, total }` (`cursor` is the position for the next call, `added_ids` lists IDs assigned to items added by this call in order)
- `list_journals` → `{ journals: [{ name, title, item_count, meta }], total, limit, offset }`

## CLI Commands

```
foray serve                          # Start MCP stdio server
foray show [name] [--json] [--limit N] [--offset N]  # Full journal with items
foray add <content> [--type TYPE] [--ref FILE] [--tags CSV] [--meta KEY=VALUE]...
foray open <name> [--title "..."] [--fork [SOURCE]] [--meta KEY=VALUE]...  # Create or fork. --title required for new/fork. --fork without SOURCE forks from active journal.
foray list [--json] [--tree] [--archived] [--limit N] [--offset N]  # List journals. --tree shows fork lineage. --archived shows archived.
foray archive <name>                   # Archive a journal
foray unarchive <name>                 # Unarchive a journal
foray export <name> [--file PATH]       # Export journal JSON to stdout (or file)
foray import [--file PATH]              # Import journal JSON from stdin (or file)
```

Global option: `--journal <name>` on all commands (overrides env + .forayrc).

`open` creates the journal (or forks if `--fork`), writes `.forayrc` in cwd.
- `foray open deep-dive --title "Explore DB connection pooling theory"` → create empty journal, write `.forayrc`
- `foray open deep-dive --title "DB pooling deep dive" --fork` → fork from active journal (resolved via chain, error if none)
- `foray open deep-dive --title "DB pooling deep dive" --fork auth-triage` → fork from `auth-triage` explicitly

**Journal resolution for CLI** (show, add):
1. `--journal` flag if provided
2. `FORAY_JOURNAL` env var if set
3. `.forayrc` file found walking up from cwd
4. Error with helpful message listing the three options

## Steps

### Phase 1: Scaffold
1. `cargo init .` — set up `Cargo.toml` with all deps
2. Module structure: `lib.rs` (re-exports), `types.rs`, `store.rs`, `tree.rs`, `server.rs`, `cli.rs`, `main.rs`

### Phase 2: Types + Store
1. `types.rs`:
   - `JournalFile` { `_note`, id, name, title, items, created_at, updated_at, meta } — `id` is `journal_id()`: consonant-only `xxxxx-xxxxx-xxxxx` format (15 chars, ~65 bits), generated on creation, immutable
   - `JournalItem` { id, item_type, content, file_ref, added_at, tags, meta } — `id` is `item_id()`: consonant-only `xxxx-xxxx-xxxx-xxxx` format (16 chars, ~70 bits)
   - `ItemType` enum { Finding, Decision, Snippet, Note, Fork }
   - `JournalSummary` { name, title, item_count, meta }
   - `Pagination` { limit: Option<usize>, offset: Option<usize> }
   - Both `JournalFile` and `JournalItem` get `#[serde(deny_unknown_fields)]` and `meta: Option<HashMap<String, serde_json::Value>>` for client-specific extensibility
   - `validate_name()` for journal name validation
2. `store.rs`:
   - `trait JournalStore: Send + Sync` with: `load(name, pagination) -> (JournalFile, total)`, `create`, `add_items(name, Vec<JournalItem>)`, `list(pagination, archived) -> (Vec<JournalSummary>, total)`, `delete`, `exists`, `archive`, `unarchive`
   - `load` reads both active and archived journals (always readable).
   - `add_items` errors if the journal is archived.
   - `archive(name)` marks a journal as archived; `unarchive(name)` restores it.
   - `list(archived: bool)` returns active journals by default, archived when `archived = true`.
   - Pagination pushed down to the trait so backends (Elasticsearch, etc.) can handle it natively. `JsonFileStore` reads the full file and slices in memory — the cost is trivial compared to the LLM API call that follows. Pagination controls how much data the LLM receives, not I/O efficiency.
   - `JsonFileStore::new(base_dir)` — flat `~/.foray/journals/*.json`
   - Atomic writes (tmp+fsync+rename)
   - File locking via `fs2::lock_exclusive` on `{name}.lock` sidecar file for concurrent access safety
   - Custom `StoreError` enum
3. Free functions: `fork_journal(store, source, new_name, title)`

### Phase 3: Tree
1. `tree.rs` — `build_tree(journals) -> String` — ASCII tree for CLI `--tree` flag. Scans items for `type: fork` with `foray:` refs to determine lineage.

### Phase 4: MCP Server
1. `server.rs` — `ForayServer` with `store: Arc<dyn JournalStore>`. Fully stateless.
2. Server `instructions` field — bootstrap hint pointing to SETUP.md (raw URL) for per-client skill install paths and setup guidance.
3. 3 MCP prompts: `start_journal`, `resume_journal`, `summarize`.
4. 3 tools via `#[tool_router]`. Every tool that operates on a journal takes explicit name param.
5. `open_journal` implements the behavior matrix (create / fork / idempotent / error).

### Phase 5: CLI + Main *(parallel with Phase 4)*
1. `cli.rs` — clap derive subcommands. Global `--journal` option. `--fork [SOURCE]` on `open`.
2. `resolve_journal(cli_flag, env, cwd) -> Result<String>` — the resolution chain.
3. `find_forayrc(start_dir) -> Option<String>` — walk up from cwd looking for `.forayrc`, parse TOML, return `current-journal` value. Stop at `root = true` or filesystem root.
4. `main.rs` — parse CLI, construct `JsonFileStore`, route subcommands. `serve` → MCP server. Everything else → resolve journal via chain, call store, format output.
5. `open` handler: create/fork journal, write `.forayrc` in cwd.

### Phase 6: Setup Guide + Companion Skill + README + Config

**Architecture**: The binary is the stable platform (4 MCP tools, pluggable journal store). The companion skill is the evolving product (behavioral rules, use case patterns). The setup guide bootstraps everything.

1. `SETUP.md` — LLM-oriented setup guide (not a skill, a one-time instruction document). User downloads it from GitHub and directs each AI assistant to read it and follow the steps for itself. The guide is split into two sections:
    - **Step 1 (one-time)**: binary installation — check if `foray` is on PATH (`foray --version`). If not: download a prebuilt binary from GitHub releases or `cargo install foray`. The AI confirms with the user before attempting.
    - **Steps 2–4 (per-assistant)**: each AI assistant follows these for itself:
      - **MCP server configuration**: configure `foray serve` as an MCP server for its specific client (Claude CLI, Claude Desktop, Cursor, VS Code / GitHub Copilot), with per-client config file paths and JSON snippets inline
      - **Companion skill installation**: download `SKILL.md` from `https://github.com/cavokz/foray/releases/latest/download/SKILL.md` and save to the global skills path for the client (Claude Desktop uses project instructions instead)
      - **Verification**: invoke `list_journals` to confirm the MCP server is responding
    - User never needs to clone the foray repo

2. `skills/foray/SKILL.md` — companion agent skill following agentskills.io standard.

    **Frontmatter:**
    - `name: foray`
    - `requires: foray >= 0.1.0` (minimum binary version)
    - `update-url:` GitHub release download URL for latest SKILL.md
    - `user-invocable: false` (auto-triggered, not a slash command)

    **Use cases the skill should guide:**
    - **Starting an investigation** — user says "I need to figure out why X." LLM suggests `open_journal`, starts adding findings as it discovers things.
    - **Resuming work** — new chat session, user says "continue on the auth thing." LLM calls `list_journals`, finds the journal, calls `sync_journal` to reload, picks up where it left off.
    - **Branching an investigation** — LLM discovers two possible directions. Suggests forking via `open_journal(name, fork)` to explore one without losing the main thread.
    - **Passive recording** — during code review or debugging, LLM spots important things and adds findings/decisions to the current journal without being asked.
    - **Synthesizing / reporting** — user asks "summarize what we've found." LLM calls `sync_journal`, reads all items, produces a summary.
    - **Cross-session handoff** — user worked in Claude Desktop yesterday, opens VS Code today. Same files, LLM loads journal and continues.
    - **Comparing branches** — user asks "how does the DB theory compare to the caching theory?" LLM calls `sync_journal` on both forks, compares findings.

    **Behavioral rules:**
    - Call `list_journals` before creating new journals, suggest existing ones
    - When opening an existing journal, omit `title`. When creating or forking, always provide `title`.
    - After forking from X to Y, use Y for subsequent `sync_journal` calls
    - Use `ref` field for file paths, URLs, ticket links, PR links
    - When adding items in a version-controlled checkout, set `meta.vcs-repo` (remote URL), `meta.vcs-branch`, and `meta.vcs-revision` (commit SHA, changelist number, etc.) on the item — anchors each `ref` to an exact codebase state
    - To cross-reference another journal, use `ref: "foray:journal-name#id"` (e.g. `foray:auth-triage#tshj-lkbw-rmvn-dpcf`) — human-readable name + machine-verifiable ID
    - When resuming, call `sync_journal` to reload findings. Correlate related journals by checking fork items with `foray:` refs.
    - Foray is opt-in — use it when the conversation is investigative, not for quick questions
    - Skill includes its own `update-url` — LLM can fetch the latest, diff against local copy, summarize what changed, and offer to update

3. `README.md` — challenge template, competitive positioning, multi-client config
4. `.vscode/mcp.json`, config examples for Claude Desktop + Cursor
5. Repo-level AI instructions — same rule in each IDE's format: after any code change, review `doc/blueprint.md` and update it to reflect the current state. Covers: tool signatures, response formats, CLI flags, type definitions, storage layout, behavioral rules. The blueprint is the living spec — code and doc must stay in sync.
    - `.github/copilot-instructions.md` (VS Code Copilot)
    - `.cursor/rules/blueprint.mdc` (Cursor)
    - `CLAUDE.md` (Claude Code)

### Phase 7: CI + Test + Polish
1. `.github/workflows/ci.yml` — 3-platform matrix (ubuntu, macos, windows)
2. Unit tests: store CRUD, fork, tree, journal resolution chain, forayrc walk-up, open_journal behavior matrix
3. `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`
4. Manual smoke test: CLI + MCP integration
5. Demo video (5 min)

## Verification
1. `cargo test` — store CRUD, fork, tree, journal resolution, forayrc walk-up, open_journal matrix
2. `cargo build --release` — clean binary
3. CLI smoke test:
   - `foray open auth-triage --title "Auth cache investigation"` → creates journal + `.forayrc`
   - `foray add "found bug" --type finding` → uses .forayrc
   - `foray open deep-dive --title "DB pooling theory" --fork` → forks from auth-triage (via .forayrc), updates .forayrc
   - `foray add "race condition" --type finding` → adds to deep-dive
   - `foray show auth-triage` → only original items
   - `foray list` → journal list
   - `foray list --tree` → fork lineage tree
   - `FORAY_JOURNAL=other foray show` → env override
   - `foray show --journal explicit` → flag override
4. MCP integration:
   - Configure in VS Code → tools appear
   - `open_journal(name: "auth-triage", title: "Auth cache investigation")` → creates journal
   - `sync_journal(name: "auth-triage", items: [{ content: "..." }])` → adds item
   - `open_journal(name: "deep-dive", title: "DB pooling theory", fork: "auth-triage")` → forks
   - `list_journals` → flat list
5. Companion skill auto-triggers in VS Code

## Decisions
- No project concept — flat namespace, descriptive journal names
- No `.active` file — server is fully stateless, CLI uses .forayrc + env + flag
- `open_journal(name, title?, fork?)` — single tool for create, fork, and reopen. Replaces `switch_context` + `fork_context`
- MCP: `fork` always requires a value (source name). CLI: `--fork` without value resolves from active journal.
- `.forayrc` — TOML, `current-journal` + `root`, searched cwd-upward. Written by `foray open`.
- `#[serde(deny_unknown_fields)]` + `meta: Option<HashMap>` on JournalFile + JournalItem — strict schema with extensibility via meta
- Store trait is pure CRUD — no session/active state
- Journal creation requires explicit `open_journal` — `sync_journal` with items to non-existent journal is an error
- Companion skill encodes behavioral rules (suggest existing journals, stop writing to source after fork)
- **Binary = stable platform, skill = evolving product**: binary rarely changes (3 tools, storage), skill evolves with better prompting and use case patterns. Distributed independently.
- **No `foray skill` command**: companion skill is downloaded from GitHub, not embedded in binary
- **Skill versioning**: no version field in the skill file — the content *is* the version. LLM fetches `update-url`, diffs against local, summarizes changes, offers to update. `requires` in frontmatter ensures binary compatibility.
- **Setup guide (`SETUP.md`)**: one-time LLM-oriented instructions. User never clones the repo.
- Rust with official `rmcp` SDK, stdio transport, pluggable journal store (ships with `JsonFileStore`: JSON files, atomic writes)
- **Out of scope**: UI, auto-summarization, remote sync, `edit_item` tool, `remove_item` tool
- **Append-only journals**: wrong findings are corrected by adding a new item, not deleting. Preserves the trail, prevents re-exploring dead ends, avoids cross-client conflicts.
- **Archive**: CLI-only (no MCP tool). Archived journals are readable but not writable (`sync_journal` with items/`open` with create/fork error). `unarchive` to resume. `JsonFileStore` implements this by moving files to an `archive/` subdirectory.
- **Companion skill**: `user-invocable: false` so it's auto-triggered by the agent, not a slash command — the whole point is zero-friction adoption
- **Name**: "foray" — short, memorable, evocative of venturing into new territory. Tagline: "Start with a foray. Fork it when it branches. Keep the trail."
- **Positioning**: Not "another memory tool" (100+ exist). The only MCP server where you can fork a journal and compare paths.
- **CLI + MCP**: Single binary, `clap` subcommands. `foray serve` = MCP stdio server, all other subcommands = direct store access. Both are thin wrappers over the library.
- **Lib-first architecture**: All logic in `lib.rs` modules (`types`, `store`, `tree`). CLI and MCP server are I/O shells — parse input, call lib, format output. Library is fully unit-testable without MCP or terminal.
- **Snapshot fork**: Fork = full copy of items at fork time + a `fork` item tracking lineage. Files are self-contained.
- **CI**: GitHub Actions, 3-platform matrix (Linux, macOS, Windows). fmt + clippy + test + release build.
- **License**: Apache 2.0

## Resolved Design Questions
- **Journal name validation**: Strict `[a-z0-9_-]` only. Reject everything else at creation time.
- **First use**: Explicit — `sync_journal` with items to non-existent journal is an error. Use `open_journal` first.
- **Item counts**: Single count per journal (files are self-contained).
- **Concurrency**: `add_items` (store method) locks a `{name}.lock` sidecar file via `fs2::lock_exclusive` during read-modify-write. Concurrent adds from multiple MCP server processes or CLI are serialized. Append-only — no conflicts.
- **Input limits** (server-side, write path only): content max 64KB, title max 512 chars, max 20 tags each max 64 chars, meta max 8KB serialized. Stored data is not validated on read — trust what's on disk.
- **Pagination cap**: Server caps `limit` to 500 in both `sync_journal` and `list_journals`.
- **`ref` field**: `file_ref` in Rust, `"ref"` in JSON via serde rename. Free-form string — file paths, URLs, ticket/PR links.
- **Serde**: strict deserialization (`deny_unknown_fields`), `meta` field for extensibility, `_note` first in struct for JSON ordering, `Option` fields skipped when None.
- **CLI output**: plain text by default, `--json` flag on read commands, `--tree` flag on `list` for fork lineage.
- **MCP responses**: JSON-serialized structs. LLM formats for the user.
- **`rmcp` pattern**: `#[derive(Clone)]` server, `Arc<dyn JournalStore + Send + Sync>`, `Parameters<T>` for tool args, `CallToolResult::success(Content::text(...))` for returns. `serve_server()` returns a `RunningService` — must call `.waiting()` to keep the process alive. `use rmcp::schemars;` is required at module level for `#[derive(JsonSchema)]` to resolve.
