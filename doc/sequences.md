# foray — Sequence Diagrams

Key runtime flows. Diagrams use Mermaid and render on GitHub.

---

## 1. Session Bootstrap (`hello`)

Every session starts with `hello`. The client gets back the session-scoped `nuance`
token (replay-protection), the wire protocol version, and the list of available stores.
All subsequent tool calls must include the `nuance` and a `store` from this list.

```mermaid
sequenceDiagram
    participant C as MCP Client
    participant S as foray server
    participant R as StoreRegistry

    C->>S: call_tool("hello")
    S->>R: entries()
    R-->>S: [(name, description), ...]
    S-->>C: {version, nuance, protocol, stores, skill_uri}
    note over C: save nuance + store names for all subsequent calls
```

---

## 2. `create_journal` — Local Store (Happy Path & Duplicate)

`create_journal` is strict create-only — it returns `AlreadyExists` if the journal
is already present rather than silently reopening it. This makes "create" and "open"
distinct intents, preventing accidental clobbers.

```mermaid
sequenceDiagram
    participant C as MCP Client
    participant S as foray server
    participant J as JsonFileStore
    participant FS as Filesystem

    C->>S: call_tool("create_journal", {name, title, store, nuance})
    S->>S: preflight(nuance)
    S->>S: validate_name(name)
    S->>S: validate_title(title)
    S->>J: create(name, title, meta)
    J->>FS: stat ~/.foray/journals/<name>.json

    alt journal already exists
        FS-->>J: file found
        J-->>S: Err(AlreadyExists)
        S-->>C: ErrorData(INVALID_PARAMS, "journal already exists: <name>",\n  data:{type:"journal_already_exists", name})
    else new journal
        FS-->>J: not found
        J->>FS: write <name>.json atomically (schema=1)
        J-->>S: Ok(())
        S-->>C: {name, title}
    end
```

---

## 3. `sync_journal` — Read + Write, Local Store

`sync_journal` is the workhorse: optionally append items, then return the requested
page. Writes and reads happen in the same call so callers always see their own items.
`migrate()` runs on every read — schema upgrades are transparent and trigger an
atomic file rewrite.

```mermaid
sequenceDiagram
    participant C as MCP Client
    participant S as foray server
    participant J as JsonFileStore
    participant M as migrate
    participant FS as Filesystem

    C->>S: call_tool("sync_journal",\n  {name, from, size, items?, store, nuance})
    S->>S: preflight(nuance) · validate_name(name)

    opt items provided
        S->>J: add_items(name, items)
        J->>FS: lock <name>.lock · read · append · write · unlock
        FS-->>J: ok
    end

    S->>J: load(name, {from, size})
    J->>FS: read <name>.json
    FS-->>J: raw JSON bytes
    J->>M: migrate(raw)

    alt schema == CURRENT_SCHEMA
        M-->>J: Current(value)
    else schema < CURRENT_SCHEMA
        M-->>J: Migrated(value)
        J->>FS: atomic rewrite <name>.json (bumped schema)
    else schema > CURRENT_SCHEMA
        M-->>J: TooNew{found, max}
        J-->>S: Err(SchemaTooNew)
        S-->>C: ErrorData(INVALID_PARAMS,\n  "journal schema N is too new (max: M)")
    end

    J-->>S: (JournalFile, total)
    S-->>C: {schema, name, title, items[from..from+size],\n  from, total, added_ids}
```

---

## 4. StdioStore — Lazy Connect & Protocol Negotiation

`StdioStore` connects lazily on the first call and reuses the connection for all
subsequent ones. The `hello` handshake discovers the server's protocol version;
`check_protocol` gates any future calls. Old servers (protocol 0) get transparent
adaptation on every call.

```mermaid
sequenceDiagram
    participant L as foray (local server)
    participant SS as StdioStore
    participant R as remote foray
    participant RF as Remote FS

    L->>SS: store operation (first call — conn is None)
    SS->>R: spawn "foray ... serve" subprocess
    R-->>SS: MCP initialize / capabilities exchange

    SS->>R: call_tool("hello")
    R-->>SS: {version, nuance, protocol, stores, skill_uri}
    SS->>SS: peek raw["protocol"] before typed deserialize
    SS->>SS: check_protocol(server_protocol)

    alt protocol too new
        SS-->>L: Err(ProtocolTooNew{found, max})
    else ok
        note over SS: store conn{peer, nuance, store_name, protocol}
        note over SS: background task drains subprocess stderr
    end

    note over SS: subsequent calls — fast path: conn already set
    SS->>SS: adapt_tool(protocol, tool)
    SS->>SS: adapt_send(protocol, tool, args)
    SS->>R: call_tool(adapted_tool, adapted_args)
    R->>RF: store operation
    RF-->>R: result
    R-->>SS: MCP response
    SS->>SS: adapt_receive(protocol, tool, response)
    SS-->>L: typed result
```

---

## 5. Protocol 0 Compatibility — `create_journal` → `open_journal`

Protocol 0 servers (foray v0.2.0) expose `open_journal` (upsert semantics) instead
of `create_journal` (strict create). The adapt layer rewrites the tool name, strips
unknown params, and maps the v0 `created: false` field to `AlreadyExists` so callers
see consistent semantics regardless of the server version they are talking to.

```mermaid
sequenceDiagram
    participant C as MCP Client
    participant L as foray (StdioStore, protocol=0)
    participant A as adapt layer
    participant R as remote foray v0.2.0

    C->>L: store.create(name, title)
    L->>A: adapt_tool(0, "create_journal") → "open_journal"
    L->>A: adapt_send(0, "create_journal", {name, title, store:"local"})
    note over A: strip `store` — protocol 0 has single implicit store
    A-->>L: {name, title}

    L->>R: call_tool("open_journal", {name, title})

    alt journal did not previously exist
        R-->>L: {name, title, item_count:0, created:true}
        L->>A: adapt_receive(0, "create_journal", response)
        note over A: strip item_count · strip created (was true)
        A-->>L: {name, title}
        L-->>C: Ok(())
    else journal already existed
        R-->>L: {name, title, item_count:N, created:false}
        L->>A: adapt_receive(0, "create_journal", response)
        note over A: created:false → AdaptError::AlreadyExists(name)\n→ StoreError::AlreadyExists in call_mcp
        A-->>L: Err(AdaptError::AlreadyExists)
        L-->>C: Err(AlreadyExists)
    end
```

---

## 6. Schema Migration on Wire — `sync_journal` from Protocol 0 Server

When `StdioStore` talks to a protocol 0 server (foray v0.2.0), items in the
`sync_journal` response may be at schema 0 (no `schema` field, `ref` at top level).
`adapt_receive` injects `schema:0` so `migrate()` can normalise them to schema 1
before deserialization — completely transparent to the caller.

```mermaid
sequenceDiagram
    participant SS as StdioStore (protocol=0)
    participant A as adapt layer
    participant R as remote foray v0.2.0
    participant M as migrate

    SS->>A: adapt_tool(0, "sync_journal") → "sync_journal" (unchanged)
    SS->>A: adapt_send(0, "sync_journal",\n  {name, from:5, size:10, store:"local"})
    note over A: strip store · rename from→cursor · size→limit
    A-->>SS: {name, cursor:5, limit:10}

    SS->>R: call_tool("sync_journal", {name, cursor:5, limit:10})
    R-->>SS: {name, title, items:[...no schema field...],\n  cursor:15, limit:10, offset:5}

    SS->>A: adapt_receive(0, "sync_journal", response)
    note over A: inject schema:0\nrename cursor→from\nstrip limit and offset
    A-->>SS: {schema:0, name, title, items:[...], from:15, total:N, added_ids:[]}

    SS->>M: migrate({schema:0, name, title, items})
    note over M: v0_to_v1: move top-level `ref` into meta.ref\ninject schema:1 into each item
    M-->>SS: Migrated({schema:1, name, title, items})

    SS-->>SS: deserialize JournalFile · return to caller
```
