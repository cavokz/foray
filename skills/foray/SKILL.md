---
name: foray
requires: foray >= 0.2.0
update-url: https://github.com/cavokz/foray/releases/latest/download/SKILL.md
user-invocable: false
---

# foray — Journal Companion

You have access to **foray**, a persistent journal system via MCP tools. Use it to record findings, track decisions, and maintain context across sessions — for any work that spans multiple conversations, involves decisions worth tracing, or may need to be picked up later.

## When to Use

Use foray when the conversation involves **substantive, evolving work** — not just a quick question. This includes debugging, but also design, planning, research, refactoring, feature development, and anything the user may want to continue later.

**Triggers:**
- You are in a planning or design discussion and decisions are being made — even before any code is written
- User says "investigate", "debug", "figure out", "triage", "deep dive"
- User says "design", "plan", "draft", "build", "implement", "research"
- User asks you to work on something over multiple steps or sessions
- You discover something worth remembering across sessions
- The work might branch into multiple directions
- You're making decisions that should be traceable later
- The user explicitly asks you to use foray or open a journal

## Tools Available

| Tool | Use |
|------|-----|
| `hello` | Establish handshake and get `nuance` + available `stores` — call this first, every session |
| `list_journals` | Check existing journals before creating; capture `avg_item_size` and `std_item_size` to compute a safe `limit` for `sync_journal` |
| `open_journal` | Create or reopen a journal |
| `sync_journal` | Read items and/or add new ones (the workhorse) |
| `archive_journal` | Archive a journal (readable but not writable) |
| `unarchive_journal` | Restore an archived journal |

## Starting a Journal

1. Call `hello` to get the `nuance` token and available `stores` — capture both, you'll use them on every subsequent call
2. Call `list_journals` to check for existing related journals (pass `nuance` and `store`) — capture `avg_item_size` and `std_item_size` for any journal you plan to sync
3. If none fit, call `open_journal` with a descriptive `name` and `title`
4. Begin adding items as you work

```
hello()  → { "version": "1.2.3", "nuance": "abc123", "stores": [{"name": "local", "description": "Default local journal store"}, {"name": "work", "description": "Work projects"}] }
list_journals(store: "local", nuance: "abc123")
open_journal(name: "auth-cache-race", title: "Auth cache race condition", store: "local", nuance: "abc123")
```

### Using Multiple Stores

`store` is required on every tool call that targets a journal. Pass the store name exactly as returned by `hello`.

```
open_journal(name: "auth-cache-race", title: "Auth cache race", store: "work", nuance: "abc123")
sync_journal(name: "auth-cache-race", store: "work", nuance: "abc123", items: [...])
list_journals(store: "work", nuance: "abc123")
```

Stick to one store per journal within a session — a journal's store must be specified consistently.

## Recording Findings

Add items as you discover things. Use the right type:

| Type | When |
|------|------|
| `finding` | You discovered something relevant |
| `decision` | A choice was made (and why) |
| `snippet` | Code or config worth preserving |
| `note` | Context, questions, or observations |

**Sync as decisions are made, not just after implementation.** If a planning discussion produces a design choice, record it immediately as a `decision` item. Don't wait for code to exist.

Always set `ref` when the finding relates to a specific file, URL, or ticket:

```
sync_journal(
  name: "auth-cache-race",
  nuance: "...",
  items: [{
    content: "Race condition: two goroutines access session cache without lock",
    item_type: "finding",
    ref: "src/auth/session.go:142",
    tags: ["race-condition", "auth"]
  }]
)
```

### Anchoring to Version Control

When working in a version-controlled checkout, set VCS metadata on items so `ref` paths can be resolved to exact codebase states:

```
sync_journal(
  name: "auth-cache-race",
  nuance: "...",
  items: [{
    content: "Lock added around cache access",
    item_type: "decision",
    ref: "src/auth/session.go:142",
    meta: {
      "vcs-repo": "https://github.com/org/repo",
      "vcs-branch": "main",
      "vcs-revision": "abc123def"
    }
  }]
)
```

## Cursor Tracking

Every `sync_journal` response includes `cursor` and `total`. Track one cursor per journal and always pass it on the next call — this returns only new items, keeping responses small.

**Before the first `sync_journal` call**, compute a safe page size from `list_journals`. Each journal entry includes `avg_item_size` (average serialized JSON byte size of all items) and `std_item_size` (standard deviation of item sizes). Use your tool-output size budget (in bytes) to derive a limit:
- `avg_item_size` absent → old server, size unknown; use `limit: 5`
- `avg_item_size: 0` → journal is empty, no items to fetch
- `avg_item_size: N, std_item_size: S` → `limit = floor(output_budget / (N + 2×S))`, minimum 1 (covers ~95th percentile item size)
- `avg_item_size: N, std_item_size` absent → `limit = floor(output_budget / N)`, minimum 1

**Use that computed limit on every page** — not just the first. Do not raise it mid-pagination without a reason (e.g. if actual items turn out smaller than the average).

```
# output_budget: 50_000 bytes; avg_item_size: 444, std_item_size: 120
# limit = floor(50_000 / (444 + 2×120)) = floor(50_000 / 684) = 73
# First call — omit cursor (read from beginning), use computed limit
sync_journal(name: "my-journal", nuance: "...", limit: 73)    → cursor: 73, total: 91

# cursor < total → paginate with the same limit
sync_journal(name: "my-journal", cursor: 73, nuance: "...", limit: 73)  → cursor: 91, total: 91

# cursor == total → all items received
```

Track one cursor per journal. Always pass it — except on the very first call when starting or resuming a session (omit cursor to read from the beginning).

## Resuming Work

When the user returns to continue:

1. Call `hello` to get the nuance token
2. Call `list_journals` to find relevant journals (pass nuance)
3. Call `sync_journal` without `cursor` to start reading from the beginning — paginate (passing `cursor` each time) until `cursor == total`; capture the final `cursor`
4. Summarize recent items for the user
5. Continue adding via `sync_journal`, always passing `cursor` and `nuance` from the previous response

## Summarizing a Journal

When the user asks to summarize a journal:

1. Call `list_journals` to get `total` — note the size before deciding page size
2. Call `sync_journal` without `cursor` (omit `limit` on the first call)
3. Accumulate items across pages, paginating until `cursor == total`
4. Synthesize: group by type or theme, highlight key decisions and findings, note open questions

For large journals, summarize incrementally — synthesize each page as you go rather than waiting for all items. This avoids holding everything in context at once.

## Cross-Referencing Journals

To reference another journal's finding, use the `ref` field with foray's cross-reference format:

```
sync_journal(
  name: "db-pooling-theory",
  nuance: "...",
  items: [{
    content: "This contradicts the earlier finding about cache timing",
    item_type: "note",
    ref: "foray:auth-cache-race#tshj-lkbw-rmvn-dpcf"
  }]
)
```

Format: `foray:<journal-name>#<item-id>`

## Cross-Journal Routing

When an item surfaces that is tangential to the current journal — a stray bug, an unrelated decision, a note that belongs to another current journal — write it to the more appropriate journal instead. Use `list_journals` to check what's available if unsure.

Always tell the user when routing an item to a different journal, so they are not surprised.

```
# Bug found during a design session — route to the bugs journal, not the current one
sync_journal(
  name: "bugs",
  nuance: "...",
  items: [{ content: "...", item_type: "finding" }]
)
# → tell the user: "I've added this to the 'bugs' journal instead."
```

## Corrections

Journals are **append-only**. Never ask to delete or edit items. If a finding was wrong, add a new item explaining the correction:

```
sync_journal(
  name: "auth-cache-race",
  nuance: "...",
  items: [{
    content: "CORRECTION: session.go:142 is thread-safe — the race is in cache.go:89 instead",
    item_type: "finding",
    ref: "src/auth/cache.go:89"
  }]
)
```

## Rules

- Always call `list_journals` before creating a new journal
- When opening an existing journal, omit `title`
- When creating a new journal, always provide `title`
- Use descriptive, lowercase, hyphenated journal names
- Set `ref` for file paths, URLs, ticket links, PR links
- Don't use foray for simple one-shot Q&A with no follow-up work
- Track `cursor` per journal: capture it from every `sync_journal` response and pass it on the next call — never omit it after the first call within a session. Always check `cursor == total` to confirm all items were received; paginate if not. **Compute a safe limit from `list_journals` before the first call**: `floor(output_budget / (avg + 2×std))` if both present, `floor(output_budget / avg)` if only avg, or `5` if absent. Keep that limit on every subsequent page — never raise it mid-pagination.
- Route tangential items to the most appropriate journal, not always the current one
- **Sync proactively** — don't wait to be asked. Sync at natural completion points: after a task is done, when a round of review comments is addressed, when the user signals approval ("good", "done", "looks good"). Batch items when it makes sense, but don't defer indefinitely.

## Self-Update

This skill can be updated from its source URL. To check for updates, fetch the `update-url` from the frontmatter, diff against this file, and offer to replace if changed.
