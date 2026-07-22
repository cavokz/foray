# Foray Model Eval Harness

This directory contains scenario files for evaluating model behaviour when using
foray's MCP tools. Each scenario exercises a specific part of the companion
skill's protocol — pagination, cross-journal references, archived reads, schema
migration, and the append-only correction rule.

## Layout

```
tests/eval/
  run-eval.py          — eval runner script
  checker.py           — mechanical tool-call trace verifier
  README.md            — this file
  scenarios/           — one TOML file per scenario
    *.toml
```

Fixture journals live in `tests/fixtures/journals/`. The eval runner uses
`OPENCODE_CONFIG_CONTENT` to activate only the `foray-eval` MCP server (which
points at `tests/fixtures`) and disable all other foray servers, preventing
tool-name collisions.

### Running

Requires Python 3.9+. Install dependencies:

```sh
pip install -r requirements.txt
```

Run:

```sh
python3 tests/eval/run-eval.py github-copilot/claude-sonnet-4.6
python3 tests/eval/run-eval.py github-copilot/claude-sonnet-4.6 github-copilot/claude-haiku-4.5
```

Results are written to `eval-results-{timestamp}-{model-slug}.txt` in the
current directory.

### Manual with the foray-eval MCP server

The workspace `opencode.json` defines a `foray-eval` MCP server. To use it
manually in an opencode session, set `"enabled": true` on the `foray-eval`
entry in `opencode.json` and ensure any other foray servers are disabled.

Or start a standalone server:

```sh
FORAY_HOME=tests/fixtures cargo run -- serve
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

# Mechanical checks — boolean true enables, integer sets a threshold.
[checks]
all_syncs_use_journal = true
min_sync_calls        = 2
from_tracking         = true
size_not_fallback     = true

# Observable behaviours for manual / semantic review.
[[expected_behaviors]]
description = "..."
```

### Automated Checks

`checker.py` parses the JSON event stream from `opencode run --format json` and
runs checks declared in the scenario's `[checks]` TOML section. Universal
checks (`tool_errors`, `default` call order, `archived` flag derived from the
scenario's `archived` field) always run — no need to declare them.

Available `[checks]` keys:

| Key | Value | What it checks |
|-----|-------|----------------|
| `all_syncs_use_journal` | bool | Every `sync_journal` targets the scenario's journal |
| `first_sync_uses_journal` | bool | First `sync_journal` targets the scenario's journal |
| `from_tracking` | bool | `from` offsets advance by items-returned per page |
| `size_not_fallback` | bool | Size is not the fallback 5 when `avg_item_size` is available |
| `no_writes` | bool | No `create_journal` or `sync_journal` with items |
| `min_sync_calls` | int | At least N `sync_journal` calls |
| `max_sync_calls` | int | At most N `sync_journal` calls |
| `exact_item_count` | int | Total items retrieved matches N |
| `min_journals_read` | int | At least N distinct journal names read |

Semantic checks (e.g. "synthesizes findings from referenced journals") remain
manual — the checker reports only mechanical failures.

### Scoring

All checks must pass for the scenario to pass. Results are printed per scenario
and aggregated per model.

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
| `single-item.toml` | `stats-single` | `std_item_size` `Some(0)` → formula reduces to `floor(budget/avg)` |
