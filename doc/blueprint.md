# Blueprint: foray — Persistent Journals via MCP

## TL;DR
A **Rust MCP server + CLI** that gives any AI assistant persistent, forkable journals. Fully stateless server — every tool takes an explicit journal name. Pluggable journal store (ships with `JsonFileStore` at `~/.foray/journals/*.json`). CLI resolves journal via `--journal` flag > `FORAY_JOURNAL` env > `.forayrc` walk-up; store via `--store` flag > `FORAY_STORE` env > `.forayrc current-store` > registry default.

**Tagline:** *"Start with a foray. Fork it when it branches. Keep the trail."*

## Positioning

**Problem**: AI assistants lose context between sessions. When a conversation ends, findings, decisions, and in-progress work vanish. When work branches into multiple directions, there's no way to explore one without losing the other. And when multiple assistants — or people — work across clients and machines, their context stays siloed.

**Solution**: foray gives AI assistants a persistent, forkable journal backed by a pluggable store. Start a journal, record items as you work, fork when it branches, pick it back up in any session or client. Because the default store uses plain JSON files, multiple assistants across different clients and environments can read and write to the same journal simultaneously. This is cross-client context fusion.

**Use cases**: debugging and investigation, architecture design and planning, feature development across sessions, team stand-ups (shared journal per team, each assistant contributes updates), research, and any work that spans multiple conversations or needs to be handed off.

**Two-layer architecture**:
- **The binary** (infrastructure) — a minimal Rust MCP server + CLI. 6 tools, pluggable journal store, stateless. Rarely changes. Ships via `cargo install` or prebuilt binaries.
- **The companion skill** (product) — an agent skill that teaches the AI *when* and *how* to use journals. Evolves independently as prompting patterns improve. Self-updates from GitHub.

**Why this matters**:
- **Persistent context** — findings, decisions, and work-in-progress survive across sessions, windows, and clients
- **Forking with lineage** — branch work without losing the original thread; compare paths side-by-side
- **Human-editable** — default store uses pretty-printed JSON you can `cat`, `jq`, `grep`, hand-edit
- **Distributable** — local JSON store today; SSH and team backends planned, so intelligence isn't trapped on one machine
- **Radically simple** — 6 tools, single binary, no database, no daemon
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
  --journal flag > FORAY_JOURNAL env > .forayrc current-journal > error
CLI store resolution:
  --store flag > FORAY_STORE env > .forayrc current-store > registry default (single-store) or error
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
| **CLI journal** | Resolution chain: `--journal` > `FORAY_JOURNAL` > `.forayrc current-journal` walk-up > error | User / `foray open` |
| **CLI store** | Resolution chain: `--store` > `FORAY_STORE` > `.forayrc current-store` walk-up > registry default (single-store) or error | User / `foray open` |

### `.forayrc` (TOML)
```toml
current-journal = "auth-triage"
current-store = "remote"
root = true
```
- `current-journal` — optional, journal name for CLI resolution
- `current-store` — optional, store name for CLI resolution; written by `foray open --store <name>`
- `root = true` — optional, stops the upward directory walk
- First `.forayrc` with the relevant key wins; `root = true` halts the walk regardless
- `foray open` writes/updates `current-journal` (and `current-store` when `--store` was given) in `.forayrc` in current directory
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
  "schema": 1,
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
  "meta": { "created_by": "vscode-copilot" }
}
```

Unknown fields are rejected (`deny_unknown_fields`). The `meta` field on both `JournalFile` and `JournalItem` is a free-form map for client-specific data (AI model, conversation ID, user annotations, etc.).

### Schema Migration

Runtime migration runs on raw `serde_json::Value` **before** serde deserialization (`migrate::migrate()`), so it can add, remove, or reshape fields freely. A `Current` or `Migrated` result is guaranteed to be a JSON object matching the current schema and will deserialize cleanly with `deny_unknown_fields`. A non-object input returns `Invalid`, which the caller maps to `StoreError::Io(InvalidData)`.

`schema` absent in the JSON → version 0 (pre-versioning era). New journals always include `schema: CURRENT_SCHEMA`.

| Change type | Mechanism | User action required |
|-------------|-----------|----------------------|
| New optional field | Runtime migration, self-heal rewrite | None — transparent |
| Field removal | Runtime migration, self-heal rewrite | None — transparent |
| Schema stamp injection | Runtime migration 0→1 | None — transparent |
| Field rename | Offline tool (planned: `foray migrate`) | Explicit, one-time |
| Type change | Offline tool (planned: `foray migrate`) | Explicit, one-time |
| `schema > CURRENT_SCHEMA` | Hard error: `SchemaTooNew` | Upgrade foray binary |
| Non-object file content | Hard error: `StoreError::Io(InvalidData)` | Fix or remove the file |

**Self-healing**: migration happens lazily on write. `read_journal` migrates the value in memory but does not rewrite the file. The next `add_items` call holds the exclusive lock and rewrites the file — at that point the old fields are gone and `schema: CURRENT_SCHEMA` is persisted. This keeps migration safe: the sole writer already holds the lock, so there is no race between migration and concurrent writes.

**`StdioStore`**: the `sync_journal` response carries a `schema` field (the wire protocol schema version). `StdioStore::load` runs `migrate()` on the received data before deserializing items. If the server is newer than the client (`schema > CURRENT_SCHEMA`), `load` returns `StoreError::SchemaTooNew { origin: Wire }` — the same hard error as storage-side, with a hint naming the connected foray binary as the one to upgrade.

**Wire protocol adaptation**: `migrate::adapt_send(server_protocol, tool, args)` strips or transforms outbound request fields that old servers don't accept (e.g. `archived` in `list_journals` added at protocol 1). `migrate::adapt_receive(server_protocol, tool, response)` inserts synthesised defaults for response fields that old servers don't emit (e.g. `stores`, `protocol` in `hello` added at protocol 1). Each function contains an explicit `if server_protocol < N { match tool { … } }` block per protocol boundary. Wire structs use `deny_unknown_fields`, so any field added in a future protocol must be declared in the struct *and* synthesised by `adapt_receive` for old servers — making an incomplete adaptation a loud failure rather than silent misbehaviour.

**Dual role of `schema` and `CURRENT_SCHEMA`**: for `JsonFileStore`, the storage schema and wire schema are the same constant. For alternative store implementations (e.g. Elasticsearch), the internal on-disk representation may differ, but `sync_journal` responses must always carry `schema: CURRENT_SCHEMA` and serialize `JournalItem` fields in the current wire format. A storage-internal change must not bump `CURRENT_SCHEMA`; only changes to the `JournalItem` wire format do.

**Rationale for strict post-migration deserialization**: keeping `deny_unknown_fields` means an older foray reading a journal written by a newer binary fails loudly rather than silently dropping unknown fields and corrupting the file on write-back.

## Tech Stack
- **Language**: Rust
- **MCP SDK**: `rmcp` v1.4.0 (server, client, macros, transport-io, transport-child-process)
- **Deps**: tokio (rt, macros, sync, time — current_thread flavor), serde/serde_json, rand, chrono, dirs 6, fs2, anyhow, thiserror 2, clap (derive), toml, async-trait
- **Dev deps**: tempfile
- `async-trait` used on `trait Store` for `dyn Store` object safety with async methods. `rmcp` client + transport-child-process features enable `StdioStore` to act as an MCP client that tunnels over a subprocess stdio channel.

## Development Workflow

Pre-commit hooks enforce formatting and lint checks on every commit. After cloning, activate them once:

```sh
git config core.hooksPath .githooks
```

Hooks live in `.githooks/` (committed). `core.hooksPath` is stored in the main repo's `.git/config` and is inherited by all worktrees via `commondir` — no per-worktree setup needed. Each commit runs:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets -- -D warnings`

## MCP Server — fully stateless

### Server Identity (initialize response)

The `initialize` response includes `serverInfo` with:

| Field | Value |
|-------|-------|
| `name` | `"foray"` |
| `version` | `CARGO_PKG_VERSION` (e.g. `"0.3.0"`) |
| `title` | `"Foray — Persistent Journals for AI Agents"` |
| `description` | `CARGO_PKG_DESCRIPTION` (from `Cargo.toml`) |

`title` is the human-readable display name shown by MCP clients (e.g. VS Code MCP server list). `description` is the crates.io package description.

### Server Instructions (bootstrap)
Sent to every client on initialization via the MCP `instructions` field:

> You have access to foray, a persistent journal system for capturing findings, decisions, and context across sessions. Always call `hello` first to obtain the nuance token and available stores list. Then pass both `nuance` and a `store` name (from the `hello` stores list) on every subsequent tool call. Use `list_journals` to see existing journals, `open_journal` to create or resume one, and `sync_journal` to read and write items.
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

### MCP Tools (6 tools)

| Tool | Params | Description |
|------|--------|-------------|
| `hello` | *(none)* | Establish handshake. Returns `{ version, nuance, protocol, stores }`. Always call first — every session — then pass `nuance` and `store` on subsequent calls. |
| `open_journal` | `name`, `title?`, `fork?`, `meta?`, `store`, `nuance` | Create, fork, or reopen a journal. `title` is required when creating or forking (error if missing), ignored when reopening. `fork` specifies source journal name. Idempotent if exists without `fork`. `meta` sets journal-level metadata. `store` must be a name from the `hello` response. |
| `sync_journal` | `name`, `cursor?`, `limit?`, `items?`, `store`, `nuance` | Read and write journal items in one call. Returns items since cursor position. `cursor` is the position from the previous sync (omit for full read). `items` is an array of `{ content, item_type?, ref?, tags?, meta? }`. `limit` caps returned items (does not affect additions). Returns `cursor` for the next call and `added_ids` for items added by this call. `store` must be a name from the `hello` response. |
| `list_journals` | `limit?`, `offset?`, `archived?`, `store`, `nuance` | List journals in the selected store. Pass `archived: true` to list archived journals instead of active ones. Paginated: defaults to all. |
| `archive_journal` | `name`, `store`, `nuance` | Archive a journal. Archived journals are readable but not writable. |
| `unarchive_journal` | `name`, `store`, `nuance` | Restore an archived journal, making it writable again. |

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

### Nuance + Preflight

The `nuance` parameter is a session epoch token — a deterministic fingerprint of everything that would make a cached client session stale. Current inputs: sorted store identity fingerprints (`"name=path"` for json_file, `"name=command arg0 …"` for foray_stdio) plus `"schema=N"` for the current storage schema version and `"protocol=N"` for the current wire protocol version. All inputs are hashed with FNV-1a 64-bit. The client must obtain it from `hello` and pass it on every subsequent tool call. If the config changes (stores added/removed/moved) or the binary is upgraded to a new schema or protocol version, `nuance` changes, automatically invalidating stale sessions. If the nuance is missing or wrong, the tool returns an error with `data: { hint: "call 'hello' to get the current nuance" }`.

**Purpose**: forces the client to call `hello` first; any server-side change that would make a cached session incorrect changes the nuance and triggers re-handshake.

### Error Structure (`data` field)

Errors include a structured `data` object with machine-readable fields for programmatic dispatch and AI assistant guidance:

| Condition | Error code | `data.type` | Extra fields | `data.remedy` | `data.hint` |
|-----------|-----------|-------------|--------------|---------------|-------------|
| `nuance` missing or wrong | `invalid_params` | *(none)* | — | — | `"call 'hello' to get the current nuance"` |
| Journal not found | `invalid_params` | `"journal_not_found"` | `name` | — | Call `list_journals` hint |
| Journal already exists | `invalid_params` | `"journal_already_exists"` | `name` | — | Use different name hint |
| Journal is archived | `invalid_params` | `"journal_archived"` | `name` | `"call_unarchive_journal"` | Call `unarchive_journal` hint |
| Schema too new (storage) | `internal_error` | `"schema_too_new"` | `found`, `max` | `"upgrade_foray"` | Upgrade the connected foray MCP server |
| Schema too new (wire) | `internal_error` | `"schema_too_new"` | `found`, `max` | `"upgrade_foray"` | Upgrade the connected foray MCP server |
| Protocol too new (wire) | `internal_error` | `"protocol_too_new"` | `found`, `max` | `"upgrade_foray"` | Upgrade the connected foray MCP server |
| `store` missing | `invalid_params` | *(none)* | — | — | Store names hint |
| Unknown store name | `invalid_params` | *(none)* | — | — | Available stores hint |
| Other store errors | `internal_error` | *(none)* | — | — | *(none)* |

**AI assistant guidance**: inspect `data.type` for programmatic dispatch; surface `data.hint` verbatim to the user; if `data.remedy` is present, act on it (e.g. `"upgrade_foray"` → tell the user to upgrade the foray binary they are using as the MCP server). Old servers (pre-structured-errors) omit `data.type`; `classify_mcp_error` falls back to message-prefix matching.

### Tool response formats
- `hello` → `{ version, nuance, protocol, stores: [{name, description}] }` (e.g. `{ "version": "1.2.3", "nuance": "abc123", "protocol": 1, "stores": [{"name": "local", "description": "Default local journal store"}] }`)
- `open_journal` → `{ name, title, item_count, created }` (`created: bool` — true if new)
- `sync_journal` → `{ schema, id, name, title, items: [...], added_ids: [...], cursor, total }` (`schema` is the wire protocol schema version; `id` is the journal's immutable ID; `cursor` is the position for the next call; `added_ids` lists IDs assigned to items added by this call in order)
- `list_journals` → `{ journals: [{ name, title, item_count, meta }], total, limit, offset }` (pass `archived: true` to list archived journals)
- `archive_journal` → `{ archived: "<name>", id: "<id>" }`
- `unarchive_journal` → `{ unarchived: "<name>", id: "<id>" }`

## CLI Commands

```
foray serve                          # Start MCP stdio server
foray show [name] [--json] [--limit N] [--offset N]  # Full journal with items
foray add <content> [--type TYPE] [--ref FILE] [--tags CSV] [--meta KEY=VALUE]...
foray open <name> [--title "..."] [--fork [SOURCE]] [--meta KEY=VALUE]...  # Create or fork. --title required for new/fork. --fork without SOURCE forks from active journal.
foray list [--json] [--tree] [--archived] [--limit N] [--offset N]  # List journals. --tree shows fork lineage. --archived shows archived. --json outputs {"total": N, "journals": [...]}
foray archive <name>                   # Archive a journal
foray unarchive <name>                 # Unarchive a journal
foray export <name> [--file PATH]       # Export journal JSON to stdout (or file)
foray import [--file PATH]              # Import journal JSON from stdin (or file)
```

Global options: `--journal <name>` and `--store <name>` on all commands (override env + .forayrc).

`open` creates the journal (or forks if `--fork`), writes `.forayrc` in cwd.
- `foray open deep-dive --title "Explore DB connection pooling theory"` → create empty journal, write `.forayrc`
- `foray open deep-dive --title "DB pooling deep dive" --fork` → fork from active journal (resolved via chain, error if none)
- `foray open deep-dive --title "DB pooling deep dive" --fork auth-triage` → fork from `auth-triage` explicitly

**Journal resolution for CLI** (show, add):
1. `--journal` flag if provided
2. `FORAY_JOURNAL` env var if set
3. `.forayrc` file found walking up from cwd
4. Error with helpful message listing the three options

**Store resolution for CLI**:
1. `--store` flag if provided
2. `FORAY_STORE` env var if set
3. `.forayrc current-store` found walking up from cwd
4. Registry default if only one store is configured
5. Error with available store names if multiple stores and none selected

## Steps

### Phase 1: Scaffold
1. `cargo init .` — set up `Cargo.toml` with all deps
2. Module structure: `lib.rs` (re-exports), `types.rs`, `store.rs`, `store_json.rs`, `store_stdio.rs`, `tree.rs`, `server.rs`, `cli.rs`, `main.rs`

### Phase 2: Types + Store
1. `types.rs`:
   - `JournalFile` { `_note`, schema, id, name, title, items, meta } — `schema` is `CURRENT_SCHEMA` (u32), always set on creation. `id` is `journal_id()`: consonant-only `xxxxx-xxxxx-xxxxx` format (15 chars, ~65 bits), generated on creation, immutable
   - `JournalItem` { id, item_type, content, file_ref, added_at, tags, meta } — `id` is `item_id()`: consonant-only `xxxx-xxxx-xxxx-xxxx` format (16 chars, ~70 bits)
   - `ItemType` enum { Finding, Decision, Snippet, Note, Fork }
   - `JournalSummary` { name, title, item_count, meta }
   - `Pagination` { limit: Option<usize>, offset: Option<usize> }
   - Both `JournalFile` and `JournalItem` get `#[serde(deny_unknown_fields)]` and `meta: Option<HashMap<String, serde_json::Value>>` for client-specific extensibility
   - `validate_name()` for journal name validation
2. `store.rs`:
   - `#[async_trait] trait Store: Send + Sync` with async methods: `load(name, pagination) -> (JournalFile, total)`, `create`, `add_items(name, Vec<JournalItem>)`, `list(pagination, archived) -> (Vec<JournalSummary>, total)`, `delete`, `exists`, `archive`, `unarchive`
   - `load` reads both active and archived journals (always readable).
   - `add_items` errors if the journal is archived.
   - `archive(name) -> Result<String>` marks a journal as archived and returns the journal id; `unarchive(name) -> Result<String>` restores it and returns the journal id. `unarchive` on an already-active journal is idempotent (returns id). `archive` on an already-archived journal returns `StoreError::Archived`.
   - `list(archived: bool)` returns active journals by default, archived when `archived = true`.
   - Pagination pushed down to the trait so backends (Elasticsearch, etc.) can handle it natively. `JsonFileStore` reads the full file and slices in memory — the cost is trivial compared to the LLM API call that follows. Pagination controls how much data the LLM receives, not I/O efficiency.
   - `JsonFileStore::new(base_dir)` — flat `~/.foray/journals/*.json`
   - Atomic writes (tmp+fsync+rename)
   - File locking via `fs2::lock_exclusive` on `{name}.lock` sidecar file for concurrent access safety
   - Custom `StoreError` enum
3. Free functions: `fork_journal(store, source, new_name, title)`
4. `config.rs` — `StoreRegistry` and config parsing:
   - Parses `~/.foray/config.toml` with `[stores.<name>]` sections; two backends: `type = "json_file"` (`path`, `description`) and `type = "foray_stdio"` (`command`, `args`, `description`, `store?`). `StdioStore` always appends `serve` to the configured command, so `args` contains only transport arguments — e.g. for SSH: `command = "ssh"`, `args = ["user@host", "--", "foray"]` → spawns `ssh user@host -- foray serve`
   - Falls back to implicit `local` `JsonFileStore` at `~/.foray/journals/` when config is absent or has no stores
   - `StoreRegistry` holds a `Vec<StoreEntry>` (name, description, `Arc<dyn Store>`) and a `nuance: String`
   - `nuance` is a FNV-1a hash of sorted fingerprints (`"name=path"` for json_file, `"name=command arg0 …"` for foray_stdio) plus `"schema=N"` and `"protocol=N"` — deterministic, stable across restarts, changes when config, schema, or protocol version changes
   - `StoreRegistry::get(name)` — look up by name; `default_store()` — first entry; `names_hint()` — comma-joined names for error messages
   - `StoreRegistry::load()` — public constructor; `StoreRegistry::implicit_local()` — fallback constructor
5. `store_stdio.rs` — `StdioStore`: spawns subprocess via rmcp `TokioChildProcess`, performs MCP `initialize` handshake via `serve_client`, caches remote `nuance` + `store_name` + `protocol` obtained from `hello`. Checks `hello.protocol` against `migrate::CURRENT_PROTOCOL` at connect time — returns `StoreError::ProtocolTooNew` if the server's protocol is newer. All `Store` trait methods map to MCP tool calls via a generic `call_mcp::<T>` that wraps each outbound call with `migrate::adapt_send(server_protocol, tool, args)` and each inbound response with `migrate::adapt_receive(server_protocol, tool, raw_json)` before typed deserialization. Wire structs all use `#[serde(deny_unknown_fields)]` — this is intentional: any field added in a future protocol must be declared in the struct *and* synthesised by `adapt_receive` for old servers, guaranteeing `adapt_receive` tells the whole compatibility story. Typed wire structs: `HelloWire`, `SyncJournalWire`, `OpenJournalWire`, `ListJournalsWire`, `ArchiveWire`, `UnarchiveWire`. Connection is lazily established and cached in `Mutex<Option<Connection>>`; `Peer<RoleClient>` is cloned out before `.await` to avoid holding the lock across the await point.

   **Trust model**: `StdioStore` spawns arbitrary commands from `~/.foray/config.toml` by design — this is the mechanism that enables SSH remotes and other transports. The config file must be user-controlled; its integrity is the security boundary. Foray does not sandbox or validate the spawned command. Ensure `~/.foray/config.toml` has appropriate file permissions (`chmod 600`).

### Phase 3: Tree
1. `tree.rs` — `build_tree(journals) -> String` — ASCII tree for CLI `--tree` flag. Scans items for `type: fork` with `foray:` refs to determine lineage.

### Phase 4: MCP Server
1. `server.rs` — `ForayServer` with `registry: StoreRegistry`. Fully stateless.
2. `resolve_store(store_name: Option<&str>)` — returns `invalid_params` error (with store names hint) if `None` (`store` is required); else calls `registry.get(name)` or `invalid_params` error with store names hint if unknown.
3. Server `instructions` field — bootstrap hint pointing to SETUP.md (raw URL) for per-client skill install paths and setup guidance.
4. 3 MCP prompts: `start_journal`, `resume_journal`, `summarize`.
5. 8 tools via `#[tool_router]`. Every tool that operates on a journal takes explicit `name`, `store`, and `nuance` params. Tools that take a journal `name` (`open_journal`, `sync_journal`, `archive_journal`, `unarchive_journal`) validate it with `validate_name()` and return `invalid_params` on violation before any store access.
6. `open_journal` implements the behavior matrix (create / fork / idempotent / error).

### Phase 5: CLI + Main *(parallel with Phase 4)*
1. `cli.rs` — clap derive subcommands. Global `--journal` and `--store` options. `--fork [SOURCE]` on `open`.
2. `resolve_journal(cli_flag, explicit_name) -> Result<String>` — the resolution chain.
3. `resolve_store<'a>(registry, cli_flag) -> Result<&'a dyn Store>` — `--store` > `FORAY_STORE` > `.forayrc current-store` > registry default (single-store) or error.
4. `find_forayrc(start_dir) -> Option<String>` — walk up from cwd looking for `.forayrc`, parse TOML, return `current-journal` value. Stop at `root = true` or filesystem root.
5. `find_store_in_forayrc(start_dir) -> Option<String>` — same walk, returns `current-store` value.
6. `main.rs` — parse CLI, load `StoreRegistry`, call `resolve_store()`, route subcommands. `serve` → MCP server. Everything else → resolve journal + store via chains, call store, format output.
7. `open` handler: create/fork journal, write `.forayrc` in cwd (includes `current-store` if `--store` was passed).

### Phase 6: Setup Guide + Companion Skill + README + Config

**Architecture**: The binary is the stable platform (6 MCP tools, pluggable journal store). The companion skill is the evolving product (behavioral rules, use case patterns). The setup guide bootstraps everything.

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
    - `requires: foray >= 0.2.0` (minimum binary version)
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
3. `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
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
- **Binary = stable platform, skill = evolving product**: binary rarely changes (6 tools, storage), skill evolves with better prompting and use case patterns. Distributed independently.
- **No `foray skill` command**: companion skill is downloaded from GitHub, not embedded in binary
- **Skill versioning**: no version field in the skill file — the content *is* the version. LLM fetches `update-url`, diffs against local, summarizes changes, offers to update. `requires` in frontmatter ensures binary compatibility.
- **Setup guide (`SETUP.md`)**: one-time LLM-oriented instructions. User never clones the repo.
- Rust with official `rmcp` SDK, stdio transport, pluggable journal store (ships with `JsonFileStore`: JSON files, atomic writes)
- **Out of scope**: UI, auto-summarization, remote sync, `edit_item` tool, `remove_item` tool
- **Append-only journals**: wrong findings are corrected by adding a new item, not deleting. Preserves the trail, prevents re-exploring dead ends, avoids cross-client conflicts.
- **Archive**: MCP tools `archive_journal` and `unarchive_journal` (also available via CLI). Archived journals are readable but not writable (`sync_journal` with items/`open` with create/fork error). `unarchive_journal` to restore. `list_journals` with `archived: true` lists archived journals. `JsonFileStore` implements this by moving files to an `archive/` subdirectory. `StdioStore` implements all archive operations via MCP tool calls.
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
- **`rmcp` pattern**: `#[derive(Clone)]` server, `StoreRegistry` (not a bare `Arc<dyn Store>`), `Parameters<T>` for tool args, `CallToolResult::success(Content::text(...))` for returns. `serve_server()` returns a `RunningService` — must call `.waiting()` to keep the process alive. `use rmcp::schemars;` is required at module level for `#[derive(JsonSchema)]` to resolve.
