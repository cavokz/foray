# Schema & Protocol Misalignment: Detection and Resolution

Foray has two independent version axes that can get out of sync as the codebase evolves:

- **Schema** (`CURRENT_SCHEMA`) tracks the on-disk journal file format — which fields are present, their types, and their semantics. It is embedded in every journal file as `"schema": N` and checked whenever a journal is read, regardless of which store backend is doing the reading.
- **Protocol** (`CURRENT_PROTOCOL`) tracks the wire envelope exchanged between a `StdioStore` client and a remote foray MCP server — which tool parameters exist, which response fields are emitted, and what error shapes are used. It is only relevant when the remote transport is in use.

Each axis has its own detection path, its own error variant (`SchemaTooNew` / `ProtocolTooNew`), and its own resolution strategy.

### When this matters for a new store implementation

The `Store` trait is schema-aware but not protocol-aware. Any implementation that reads journal files from disk (like `JsonFileStore`) must call `migrate()` on the raw JSON before deserializing, and must handle `MigrateResult::TooNew` by returning `StoreError::SchemaTooNew { origin: Storage }`. It does not need to know about protocol versions.

Implementations that communicate with a remote foray server over MCP (like `StdioStore`) must additionally handle protocol negotiation: call `hello` to discover `server_protocol`, run `check_protocol()` before typed deserialization, and wrap every outbound/inbound call with `adapt_send`/`adapt_receive`. Schema migration still applies on top — the items returned by `sync_journal` may themselves be at an older schema and need migrating.

Implementations that use a completely different transport or storage format (e.g. a hypothetical HTTP store) are free to manage their own versioning, but must still produce `SchemaTooNew` when they encounter journal content they cannot safely read.

---

## Axis 1 — Schema (`CURRENT_SCHEMA = 1`)

Detected by `migrate()` when reading any journal — whether from a local file or from a `sync_journal` wire response.

| Scenario | What happens | Reported as |
|---|---|---|
| File has no `schema` field (written by foray v0.2.0 or earlier) | Treated as schema 0 → `v0_to_v1()` strips `created_at`/`updated_at`, injects `schema: 1` | Transparent — file rewritten on next write (`Migrated`) |
| File has `schema: 1` | No-op | Transparent (`Current`) |
| File has `schema: 2` (written by a newer foray) — local storage | `MigrateResult::TooNew` → `StoreError::SchemaTooNew { origin: Storage }` | Error: *"journal schema 2 is too new (max supported: 1)"* |
| `sync_journal` response carries items with `schema: 0` (v0.2.0 remote, protocol 0) | `adapt_receive` injects `schema: 0`; `migrate()` runs and migrates items transparently | Transparent |
| `sync_journal` response carries items with `schema: 2` (newer remote server) | `MigrateResult::TooNew` → `StoreError::SchemaTooNew { origin: Wire }` | Error: *"journal schema 2 is too new (max supported: 1)"* |

---

## Axis 2 — Protocol (`CURRENT_PROTOCOL = 1`)

Detected by `check_protocol()` in `connect()`, which runs against the raw `protocol` field **before** any typed deserialization.

| Scenario | What happens | Reported as |
|---|---|---|
| v0.2.0 remote server (no `protocol` field in `hello`) | Defaults to 0; `check_protocol(0)` passes; `adapt_send`/`adapt_receive` activate for all subsequent calls | Transparent with functional limitations (see below) |
| Current server (`protocol: 1`) | `check_protocol(1)` passes; no adaptation needed | Transparent |
| Future server (`protocol: 2+`) | `check_protocol` fires before `from_value` so `deny_unknown_fields` cannot race it | Error: *"wire protocol 2 is too new (max supported: 1)"* |
| `protocol` field encodes a value beyond `u32::MAX` | `u32::try_from(n).unwrap_or(u32::MAX)` saturates to `u32::MAX` → same path as above | Error: *"wire protocol 4294967295 is too new (max supported: 1)"* |

---

## Protocol 0 (v0.2.0 Servers) — Per-call Adaptation

When connected to a protocol 0 server, `adapt_send` and `adapt_receive` wrap every tool call transparently or fail fast with a clear message.

| Scenario | Direction | What `adapt_send` / `adapt_receive` does | Reported as |
|---|---|---|---|
| Any call with `store: "local"` | Send | Strips `store` (implicit on v0.2.0) | Transparent |
| Any call with `store: "work"` (non-default store) | Send | Cannot adapt — v0.2.0 has no multi-store | Error: *"store 'work' not found: protocol 0 server exposes a single implicit store 'local'"* — also caught eagerly at connect time |
| `list_journals` with `archived: false` | Send | Strips `archived` param | Transparent |
| `list_journals` with `archived: true` | Send | Cannot adapt — archive feature did not exist in v0.2.0 | Error: *"archived journals not supported by protocol 0 server; upgrade the remote foray"* |
| `archive_journal` / `unarchive_journal` | Send | Tool did not exist in v0.2.0 | Error: *"'archive_journal' is not supported by protocol 0 server; upgrade the remote foray"* |
| `hello` response missing `protocol`, `stores` | Receive | `adapt_receive` injects `protocol: 0`, synthesises `stores: [{name:"local", …}]` | Transparent |
| `sync_journal` response missing `id` | Receive | `adapt_receive` injects `id: "<unknown>"` | Transparent (callers see `<unknown>` as journal ID) |
| `open_journal` response missing `name`, `title`, `item_count` | Receive | `adapt_receive` injects empty/zero defaults | Transparent |
| `list_journals` response missing `limit`, `offset` | Receive | `adapt_receive` injects `null` for both | Transparent |

---

## Updating `migrate`, `adapt_send`, and `adapt_receive`

### Bumping the schema version (`migrate`)

Schema bumps affect on-disk journal files. The migration chain runs forward-only — each step `vN_to_vN+1` transforms a raw `serde_json::Value` from the previous schema to the next.

**Checklist when adding schema version N+1:**

1. Increment `CURRENT_SCHEMA` to `N+1` in `migrate.rs`.
2. Add a private function `vN_to_vN1(obj: Map<String, Value>) -> Map<String, Value>` that applies the transform and injects `"schema": N+1`.
3. Add `if schema < N+1 { obj = vN_to_vN1(obj); }` to the migration chain in `migrate()`, **after** all earlier steps and in ascending order. The chain must be ordered lowest-to-highest so a file at schema 0 migrates through every intermediate version in sequence.
4. Update any `JournalFile` / `JournalItem` struct fields in `types.rs` that the new schema changes.
5. Add tests: one for the new migration step, one that verifies `MigrateResult::TooNew` is returned for `schema: N+1`.

Fields are always **added at the top of the chain** (newest step last) and **never removed from earlier steps** — earlier steps are frozen history.

---

### Bumping the protocol version (`adapt_send` / `adapt_receive`)

Protocol bumps affect the wire envelope between `StdioStore` and the foray MCP server. They are independent of schema bumps, but often accompany them when new tool parameters or response fields are introduced.

**Checklist when adding protocol version N+1:**

1. Increment `CURRENT_PROTOCOL` to `N+1` in `migrate.rs`.
2. In `adapt_send`, add a new block `if server_protocol < N+1 { … }` **below** all existing blocks. Inside, strip or reject every parameter that old servers do not understand. The blocks must be ordered lowest-to-highest so a protocol 0 server passes through all applicable transformations.
3. In `adapt_receive`, add a matching block `if server_protocol < N+1 { … }` **below** all existing blocks. Inside, inject synthesised defaults for every field that old servers did not emit.
4. In `store_stdio.rs`, update the relevant wire struct (e.g. `SyncJournalWire`) to declare any new fields. Because all wire structs use `#[serde(deny_unknown_fields)]`, omitting a field here will surface as a `from_value` failure — the compiler enforces that `adapt_receive` and the struct stay in sync.
5. Add tests for `adapt_send` (new param stripped/rejected) and `adapt_receive` (new field synthesised) for the old protocol value.

**Field ordering rule:** new fields are documented and synthesised in the **newest block** (highest protocol threshold). Older blocks are frozen — never modify them to add new fields, as that would misrepresent which protocol version introduced the change.

**Removal rule:** removing a field from the wire is also a protocol bump. Add it to `adapt_send` as a field to strip before sending to old servers, and remove it from the wire struct (or mark it `#[allow(dead_code)]` if the struct shape still requires it for deserialization).

---

## `deny_unknown_fields` — The Enforcement Backstop

All wire structs (`HelloWire`, `SyncJournalWire`, `OpenJournalWire`, etc.) use `#[serde(deny_unknown_fields)]`. This turns adaptation gaps into loud failures rather than silent data loss.

| Scenario | Effect |
|---|---|
| Future server adds a new field to any response | `from_value` fails — signals that `adapt_receive` is missing a rule for this protocol bump |
| `adapt_receive` synthesises a field not declared in the wire struct | `from_value` fails — signals the struct declaration is incomplete |
| Both wire struct and `adapt_receive` are in sync | Clean deserialization, no silent data loss |

`adapt_receive` and the wire struct field declarations form a jointly-enforced contract: neither half can silently diverge from the other.
