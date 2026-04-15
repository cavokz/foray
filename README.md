# hunch

**Start with a hunch. Fork it when it branches. Keep the trail.**

A Rust MCP server + CLI that gives any AI assistant persistent, forkable investigation journals.

---

## Problem

When investigating a bug or exploring a codebase with an AI assistant, your findings disappear the moment the conversation ends. Start a new chat, open a new window, switch branches — all context is lost. You end up re-explaining the same background over and over.

Existing "memory" tools (100+ on the MCP marketplace) store flat key-value pairs or chat logs. None of them model how investigations actually work: you start with a hunch, go deep, hit a fork in the road, and need to explore both directions without losing the trail.

## Why It Matters

- **Engineers investigating bugs** can persist findings, decisions, and code references across sessions
- **Cross-window workflows** (multiple VS Code windows, git worktrees) share the same investigation state via the filesystem
- **Any MCP client** works — VS Code Copilot, Claude Desktop, Cursor, and anything else that speaks MCP
- **Forking** lets you branch an investigation the same way you branch code — snapshot the state and explore a new direction

## Language Used

**Rust** — new language for me. The official MCP SDK (`rmcp`) made it the natural choice.

## How Cursor Helped

This entire project was built with AI assistance (GitHub Copilot in VS Code, actually — which is what I had available). The AI helped with:

- **Architecture & design** — iterating through 3 approaches (VS Code extension → MCP server → CLI+MCP hybrid) in conversation
- **Rust learning** — writing idiomatic Rust as a first-time Rust developer (trait objects, serde derives, error handling patterns)
- **API discovery** — understanding the `rmcp` SDK's `#[tool_router]` macro system from docs and examples
- **Competitive analysis** — surveying 100+ existing memory tools to identify the novel contribution (forking)
- **Implementation** — scaffolding all modules, fixing compile errors, writing tests

## How to Run

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (stable)

### Build

```sh
git clone <this-repo>
cd hunch
cargo build --release
```

The binary is at `target/release/hunch`.

### CLI Usage

```sh
# Create your first investigation context
hunch switch auth-triage

# Add findings as you investigate
hunch add "auth token refresh has a race condition" --type finding --ref src/auth/session.go:142
hunch add "decided to use mutex instead of channel" --type decision --tags auth,concurrency

# Fork when the investigation branches
hunch fork auth-deep-dive

# Add to the fork — parent is unchanged
hunch add "root cause: goroutine leak in token watcher" --type finding

# See the tree
hunch list
#   auth-triage [2 items]
# └── * auth-deep-dive [3 items] (forked from auth-triage)

# Check status
hunch status
# Project:  my-project
# Active:   auth-deep-dive
# Items:    3
# Branch:   fix/auth-race

# View full context
hunch show
```

### MCP Server Setup

#### VS Code (GitHub Copilot)

Add to `.vscode/mcp.json` in your project:

```json
{
  "servers": {
    "hunch": {
      "command": "/path/to/hunch",
      "args": ["serve", "--workspace", "${workspaceFolder}"]
    }
  }
}
```

#### Claude Desktop

Add to `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "hunch": {
      "command": "/path/to/hunch",
      "args": ["serve", "--workspace", "/path/to/your/project"]
    }
  }
}
```

#### Cursor

Add via Cursor Settings → MCP Servers:

```json
{
  "mcpServers": {
    "hunch": {
      "command": "/path/to/hunch",
      "args": ["serve", "--workspace", "/path/to/your/project"]
    }
  }
}
```

### Companion Agent Skill

For automatic integration (the AI uses hunch without being asked), copy the skill:

```sh
# Project-level (just this project)
cp -r skills/hunch .copilot/skills/

# Personal (all projects)
cp -r skills/hunch ~/.copilot/skills/
```

## MCP Tools

| Tool | Description |
|------|-------------|
| `get_status` | Project name, active context, item count, git branch |
| `get_context` | Read all items in a context (defaults to active) |
| `add_item` | Add a finding, decision, snippet, or note |
| `fork_context` | Snapshot-copy a context under a new name |
| `switch_context` | Switch active context (creates if new) |
| `list_contexts` | List all contexts with fork lineage tree |
| `remove_item` | Remove an item by ID |

## Example Output

```
$ hunch list
  auth-triage [2 items]
  ├──   auth-deep-dive [3 items] (forked from auth-triage)
  └──   auth-mutex-approach [2 items] (forked from auth-triage)
  perf-investigation [5 items]
  └── * perf-cache-layer [7 items] (forked from perf-investigation)

$ hunch show auth-deep-dive
Context: auth-deep-dive
Parent:  auth-triage
Items:   3

  [a1b2c3d4] (finding) auth token refresh has a race condition
         ref: src/auth/session.go:142
         tags: auth, race-condition
  [e5f6a7b8] (decision) decided to use mutex instead of channel
         tags: auth, concurrency
  [c9d0e1f2] (finding) root cause: goroutine leak in token watcher
```

## Storage

Human-readable JSON files at `~/.hunch/<project>/`:

```
~/.hunch/
  └── my-project/
      ├── .active                    ← plain text: active context name
      ├── auth-triage.json
      ├── auth-deep-dive.json
      └── perf-investigation.json
```

Files are self-contained and hand-editable. Use `cat`, `jq`, or your editor.

## What Makes This Different

In a space of 100+ AI memory tools, hunch is the **only MCP server where you can fork an investigation**:

- **7 tools** (competitors have 22–38)
- **Single binary** — no Node, Python, Ollama, or SQLite
- **Human-editable JSON files** — `cat`, `jq`, `grep`, hand-edit
- **Universal** — any MCP client, cross-window via filesystem
- **Investigation-first** — typed items (finding/decision/snippet/note), not generic "memory"

## License

Apache 2.0
