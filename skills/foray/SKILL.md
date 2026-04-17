---
name: foray
requires: foray >= 0.1.0
update-url: https://github.com/cavokz/foray/releases/latest/download/SKILL.md
user-invocable: false
---

# foray — Investigation Journal Companion

You have access to **foray**, a persistent investigation journal system via MCP tools. Use it to record findings, track decisions, and maintain context across sessions.

## When to Use

Use foray when the conversation is **investigative** — debugging, exploring architecture, triaging issues, researching options. Don't use it for quick questions or simple tasks.

**Triggers:**
- User says "investigate", "debug", "figure out", "triage", "deep dive"
- You discover something worth remembering across sessions
- The investigation might branch into multiple theories
- You're working in a codebase and finding things that matter later

## Tools Available

| Tool | Use |
|------|-----|
| `list_journals` | Check existing journals before creating |
| `open_journal` | Create, fork, or reopen a journal |
| `sync_journal` | Read items and/or add new ones (the workhorse) |

## Starting an Investigation

1. Call `list_journals` to check for existing related journals
2. If none fit, call `open_journal` with a descriptive `name` and `title`
3. Begin adding findings as you discover them

```
open_journal(name: "auth-cache-race", title: "Investigating auth cache race conditions")
```

## Recording Findings

Add items as you discover things. Use the right type:

| Type | When |
|------|------|
| `finding` | You discovered something relevant |
| `decision` | A choice was made (and why) |
| `snippet` | Code or config worth preserving |
| `note` | Context, questions, or observations |

Always set `ref` when the finding relates to a specific file, URL, or ticket:

```
sync_journal(
  name: "auth-cache-race",
  items: [{
    content: "Race condition: two goroutines access session cache without lock",
    item_type: "finding",
    file_ref: "src/auth/session.go:142",
    tags: ["race-condition", "auth"]
  }]
)
```

### Anchoring to Version Control

When working in a version-controlled checkout, set VCS metadata on items so `ref` paths can be resolved to exact codebase states:

```
sync_journal(
  name: "auth-cache-race",
  items: [{
    content: "Lock added around cache access",
    item_type: "decision",
    file_ref: "src/auth/session.go:142",
    meta: {
      "vcs-repo": "https://github.com/org/repo",
      "vcs-branch": "main",
      "vcs-revision": "abc123def"
    }
  }]
)
```

## Forking an Investigation

When the investigation branches into distinct theories, fork:

```
open_journal(
  name: "db-pooling-theory",
  title: "Exploring DB connection pooling as root cause",
  fork: "auth-cache-race"
)
```

After forking:
- Use the **new** journal for subsequent `sync_journal` calls
- The original journal is preserved as-is
- A `fork` item in the new journal tracks lineage

## Resuming Work

When the user returns to continue an investigation:

1. Call `list_journals` to find relevant journals
2. Call `sync_journal` to reload context (omit `cursor` for full read)
3. Summarize recent findings for the user
4. Continue adding new findings via `sync_journal`, passing `cursor` from the previous response

## Comparing Branches

When the user asks to compare investigation paths:

1. Call `sync_journal` on each fork
2. Compare findings side by side
3. Highlight which theory has more evidence

## Cross-Referencing Journals

To reference another journal's finding, use the `ref` field with foray's cross-reference format:

```
sync_journal(
  name: "db-pooling-theory",
  items: [{
    content: "This contradicts the earlier finding about cache timing",
    item_type: "note",
    file_ref: "foray:auth-cache-race#tshj-lkbw-rmvn-dpcf"
  }]
)
```

Format: `foray:<journal-name>#<item-id>`

## Corrections

Journals are **append-only**. Never ask to delete or edit items. If a finding was wrong, add a new item explaining the correction:

```
sync_journal(
  name: "auth-cache-race",
  items: [{
    content: "CORRECTION: session.go:142 is thread-safe — the race is in cache.go:89 instead",
    item_type: "finding",
    file_ref: "src/auth/cache.go:89"
  }]
)
```

## Rules

- Always call `list_journals` before creating a new journal
- When opening an existing journal, omit `title`
- When creating or forking, always provide `title`
- Use descriptive, lowercase, hyphenated journal names
- Set `ref` for file paths, URLs, ticket links, PR links
- After forking from X to Y, use Y for subsequent adds
- Don't use foray for quick Q&A — only for investigations
- Track `cursor` per journal: remember the `cursor` value from each `sync_journal` response and pass it back on the next call to the same journal

## Self-Update

This skill can be updated from its source URL. To check for updates, fetch the `update-url` from the frontmatter, diff against this file, and offer to replace if changed.
