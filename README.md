# foray

*Start with a foray. Fork it when it branches. Keep the trail.*

An MCP server + CLI that gives AI assistants persistent, forkable investigation journals.

## Problem

AI assistants lose their investigation trail between sessions. When a conversation ends, findings, dead ends, and decisions vanish. When an investigation branches ("is it the DB or the cache?"), there's no way to fork the reasoning and compare paths.

## Why It Matters

Engineers who debug complex issues across multiple sessions waste time re-discovering context. foray solves this by giving AI assistants a persistent journal they can write to, fork, and resume — across sessions, windows, and even different MCP clients.

- **Persistent trail** — findings survive across sessions
- **Forking with lineage** — branch an investigation without losing the original thread
- **Cross-client** — VS Code, Cursor, Claude Desktop can share the same journals
- **Human-editable** — plain JSON files you can `cat`, `jq`, `grep`, hand-edit

## How to Install

```sh
cargo install foray
```

Or download a pre-built binary from [GitHub Releases](https://github.com/cavokz/foray/releases/latest) and place it on your `PATH`.

Then follow the [Setup Guide](SETUP.md) to configure your MCP client.

## MCP Tools

| Tool | Description |
|------|-------------|
| `open_journal` | Create, fork, or reopen a journal |
| `sync_journal` | Read and/or write items (cursor-based) |
| `list_journals` | List active journals |

## MCP Prompts

| Prompt | Description |
|--------|-------------|
| `start_investigation` | Create a journal and begin recording |
| `resume_investigation` | Load a journal and continue |
| `summarize` | Read all items and produce a synthesis |

## CLI Usage

Create a journal:

```
$ foray open auth-triage --title "Auth cache investigation"
Created journal: auth-triage
Set active journal in .forayrc
```

Add findings (uses `.forayrc` created by open):

```
$ foray add "Race condition in session.go:142" --type finding --ref src/auth/session.go:142
Added to auth-triage (1 items)
```

Fork when the investigation branches:

```
$ foray open db-theory --title "DB pooling theory" --fork auth-triage
Forked auth-triage → db-theory (1 items)
Set active journal in .forayrc
```

View a journal:

```
$ foray show auth-triage
Journal: auth-triage
Title:   Auth cache investigation
Items:   1 / 1

[2026-04-15 10:15] (finding) Race condition in session.go:142
  ref: src/auth/session.go:142
```

List all journals with fork lineage:

```
$ foray list --tree
auth-triage
└── db-theory
```

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
```

Single binary. No database. No daemon. Journals are plain JSON files.

## License

Apache-2.0
