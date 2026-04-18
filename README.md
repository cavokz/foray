# foray

[![crates.io](https://img.shields.io/crates/v/foray.svg)](https://crates.io/crates/foray)

*Start with a foray. Fork it when it branches. Keep the trail.*

An MCP server + CLI that gives AI assistants persistent, forkable journals. Use it for debugging, planning, design, feature work — any conversation worth continuing later.

## Problem

AI assistants lose context between sessions. When a conversation ends, findings, decisions, and in-progress work vanish. When work branches into multiple directions, there's no way to explore one without losing the other. And when multiple assistants work across different clients, their context stays siloed.

## Why It Matters

foray gives AI assistants a persistent, forkable journal backed by plain JSON files. Start a journal, record items as you work, fork when it branches, pick it back up in any session or client.

- **Persistent context** — findings, decisions, and work-in-progress survive across sessions
- **Forking with lineage** — branch without losing the original thread; compare paths side-by-side
- **Cross-client** — VS Code, Cursor, Claude Desktop share the same journals simultaneously
- **Human-editable** — plain JSON files you can `cat`, `jq`, `grep`, hand-edit

## How to Install

```sh
cargo install foray
```

Or download a pre-built binary from [GitHub Releases](https://github.com/cavokz/foray/releases/latest) and place it on your `PATH`.

Then direct your AI assistant to fetch the [Setup Guide](https://raw.githubusercontent.com/cavokz/foray/main/SETUP.md) and follow the steps for itself.

## MCP Tools

| Tool | Description |
|------|-------------|
| `hello` | Handshake — call first every session, returns `{version, nuance}` |
| `open_journal` | Create, fork, or reopen a journal |
| `sync_journal` | Read and/or write items (cursor-based) |
| `list_journals` | List active journals |

## MCP Prompts

| Prompt | Description |
|--------|-------------|
| `start_journal` | Create a journal and begin recording |
| `resume_journal` | Load a journal and continue |
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
