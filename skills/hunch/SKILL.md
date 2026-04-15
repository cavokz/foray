---
name: hunch
description: Manage investigation journals — persistent, forkable contexts for tracking findings, decisions, and notes across sessions.
user-invocable: false
---

# hunch — Investigation Journals

You have access to **hunch**, a tool for managing persistent investigation journals. Use it to keep track of findings, decisions, code references, and notes as you work.

## When to use hunch

- **At the start of a conversation**: Call `get_status` to check if there's an active investigation context. If there is, call `get_context` to load existing findings before starting work.
- **When you discover something significant**: Call `add_item` with type `finding` for discoveries, `decision` for choices made, `snippet` for important code, or `note` for general observations.
- **When the investigation branches**: Suggest `fork_context` when the user wants to explore a different direction while preserving the current trail.
- **When switching topics**: Use `switch_context` to move to a different investigation.
- **When reviewing progress**: Use `list_contexts` to see all investigations and their fork lineage.

## Using the `ref` field

Always use the `ref` field when an item relates to a specific location or source:
- File paths: `src/auth/session.go:142`
- URLs: `https://github.com/org/repo/pull/87`
- Tickets: `https://jira.example.com/browse/PROJ-1234`
- Documentation: `https://docs.example.com/api/auth`

## Guidelines

- Prefer `finding` type for things discovered during investigation
- Prefer `decision` type for choices and their rationale
- Use comma-separated tags to categorize items (e.g., `auth,race-condition,critical`)
- Keep item content concise but informative — these are journal entries, not essays
- When forking, name the new context descriptively (e.g., `auth-deep-dive`, `perf-cache-layer`)
- Don't ask permission to record findings — just add them as you discover things
