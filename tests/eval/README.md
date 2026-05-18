# Foray Model Eval Harness

This directory contains scenario files for evaluating model behaviour when using
foray's MCP tools. Each scenario exercises a specific part of the companion
skill's protocol — pagination, cross-journal references, archived reads, schema
migration, and the append-only correction rule.

## Layout

```
tests/eval/
  README.md            — this file
  scenarios/           — one TOML file per scenario
    *.toml
```

Fixture journals live in `tests/fixtures/journals/`. The eval must not mutate
them, so the server is started with `FORAY_HOME` pointing at a copy.

### VS Code

The workspace ships a `.vscode/mcp.json` with a ready-made `foray-eval` server
entry. It runs `cargo run -- serve` with `FORAY_HOME` set to `tests/fixtures`,
so no manual setup is needed — just activate the `foray-eval` server from the
MCP panel and open a Copilot Chat session.

### Manual

```sh
TMP=$(mktemp -d)
cp -r tests/fixtures/journals "$TMP/journals"
FORAY_HOME="$TMP" foray serve
```

## Scenario Format

Each `.toml` file describes one eval scenario:

```toml
# The fixture journal to use (must exist under tests/fixtures/journals/).
journal = "stats-uniform"

# Whether the journal is archived (sync_journal calls must pass archived = true).
archived = false

# Prompt given to the model.
prompt = "..."

# List of observable behaviours that the model's tool-call trace must exhibit.
[[expected_behaviors]]
description = "..."
```

### Scoring

Evaluate the model's tool-call trace against each `expected_behaviors` entry.
Each entry is a plain-English description of an observable behaviour; a human or
automated scorer marks it pass/fail. All entries must pass for the scenario to
pass.

## Scenarios

| File | Journal | Purpose |
|------|---------|---------|
| `pagination-uniform.toml` | `stats-uniform` | Multi-page pagination with zero variance (size ≈ budget/avg) |
| `pagination-high-variance.toml` | `stats-high-variance` | Conservative paging when std is large |
| `pagination-realistic.toml` | `stats-realistic` | Full 200-item pagination, offset tracking |
| `archived-read.toml` | `stats-high-variance-archived` | `archived: true` in all sync calls |
| `cross-reference.toml` | `cross-reference` | Follow `foray:` refs, open referenced journals |
| `schema-migration.toml` | `schema-v0` | Read pre-v1 journal, migration transparent |
| `empty-journal.toml` | `stats-empty` | Absent stats → safe default size |
| `single-item.toml` | `stats-single` | Absent std\_item\_size → graceful handling |
