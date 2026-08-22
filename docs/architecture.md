# DBX architecture

This document describes the intended architecture for DBX, a native Rust/GPUI database manager. It is a design contract for the MVP, not a statement that every component already exists.

## Goals and boundaries

DBX should provide a responsive desktop surface for browsing and changing PostgreSQL, MySQL, SQLite, and Redis data. The common path is schema discovery → paged browsing → GUI filtering → reviewed CRUD or SQL execution.

The UI must never block on network or disk I/O. Database-specific syntax and metadata must stay behind connectors. The shared application layer owns product behaviour (selection, drafts, paging, errors, and history), while each connector owns capabilities and translation to its engine.

DBX is not a migration runner, backup system, monitoring console, or distributed collaboration service in the first release. Those features can build on the same connector and command boundaries later.

## Layer model

```text
┌─────────────────────────────────────────────────────────┐
│ GPUI presentation                                       │
│ app shell · connection picker · tree · data grid · SQL  │
└───────────────────────┬─────────────────────────────────┘
                        │ typed actions/state updates
┌───────────────────────▼─────────────────────────────────┐
│ Application/session layer                               │
│ connection sessions · selection · drafts · filters      │
│ pagination · query history · notifications              │
└───────────────────────┬─────────────────────────────────┘
                        │ connector commands/results
┌───────────────────────▼─────────────────────────────────┐
│ Database core                                           │
│ metadata model · value model · filter AST · mutations   │
│ capability checks · cancellation · redacted errors     │
└───────────────────────┬─────────────────────────────────┘
                        │ engine-specific translation
┌───────────────┬───────┴────────┬───────────────┐
│ PostgreSQL    │ MySQL          │ SQLite        │ Redis
│ driver        │ driver         │ driver        │ client
└───────────────┴────────────────┴───────────────┴───────────────┘
```

GPUI models and views should receive small, immutable result snapshots or streams of bounded pages. They should not hold a driver connection or call a blocking client directly. A session owns the lifetime of a connector task and sends cancellation when a tab closes, a new query supersedes an old one, or the user presses Stop.

## Core contracts

The exact Rust module names may change, but the seam should remain close to these concepts:

- `EngineKind`: PostgreSQL, MySQL, SQLite, or Redis.
- `ConnectionConfig`: a redacted/display-safe description plus secret material held separately. Variants should make engine differences explicit (for example, SQLite path versus host/port/database).
- `Capabilities`: whether the engine supports schemas, transactions, prepared parameters, offset/keyset paging, table DDL, row updates, and cancellation.
- `DatabaseConnector`: connect/close, ping, discover metadata, fetch a page, execute a parameterized statement, and expose engine-specific operations through capability-checked methods.
- `Metadata`: databases/namespaces, tables or keyspaces, columns, indexes, primary keys, and constraints. Redis metadata is key pattern/type/TTL information, not fabricated columns.
- `DbValue`: null, booleans, signed/unsigned numbers, floating values, text, bytes, timestamps, JSON, and an explicit opaque/display-only value for types the grid cannot safely edit.
- `FilterExpr`: a structured predicate (`and`, `or`, `not`, equality, comparison, null checks, text match, and membership) plus sort and page information. It compiles to SQL and bound parameters for relational engines; Redis uses an intentionally narrower key/type predicate set.
- `Mutation`: insert/update/delete or create-table intent, including the target identity, values, expected row count/version when available, and a preview representation before execution.
- `Transfer`: table and connection-level import/export between a connection and local files. SQL dumps (`.sql`), CSV/TSV, each optionally gzip-compressed. Database SQL exports can select tables and emit every generated table schema before the data phase, or schemas only; PostgreSQL/MySQL foreign-key constraints are added after data, while SQLite orders dependent tables before their rows. Delimited database exports write one independently consumable file per selected table. Exports page through the shared query path; SQL-dump imports replay statements through the same execute path as the console (behind an explicit confirmation), and delimited imports bulk-append parameterized multi-row inserts after mapping the file header onto real columns. Transfers are connection-level operations in core, so they inherit quoting, parameterization, capability checks, and redacted errors for free.

Identifiers are represented as identifiers, not raw SQL fragments. Each connector quotes them according to its engine. User-entered SQL is still allowed in the console, but is clearly separated from generated statements and is never silently mixed with GUI filter input.

## GPUI and asynchronous work

GPUI owns the main event/render loop. Connector calls should run in background tasks using the chosen async runtime or a dedicated worker abstraction, then return results through GPUI-compatible task handles/channels. The implementation should:

- keep the visible state small and update it on page boundaries;
- support cancellation and a per-operation timeout;
- use bounded channels and row limits so an accidental `SELECT *` cannot exhaust memory;
- distinguish connection, metadata, query, decode, and cancellation errors;
- avoid putting passwords, full query results, or parameter values in debug output;
- close or return a session's connection when a tab is disposed.

Connection pooling is an implementation choice per driver. A simple per-session connection is sufficient for the MVP, provided concurrent metadata and query operations do not race on a non-thread-safe client. Pooling and parallel query tabs can be added behind the same session interface.

## Main user flows

### Connect and discover

The connection form validates the engine-specific fields before attempting a connection. A successful session pings first, then loads namespaces/tables lazily. Discovery failures should leave the connection visible with a retry action rather than taking down the window. The tree should not issue a request for every collapsed node.

### Browse and filter

Opening a table creates a data-grid model with a stable row identity (primary key where available; an engine-specific fallback otherwise). The grid requests a bounded page and renders only visible rows. A filter builder produces `FilterExpr`; the UI shows the active predicates and offers a SQL/parameter preview. Applying a filter replaces the page request and resets the cursor.

For tables without a primary key, DBX should warn that updates/deletes may be unsafe or unavailable. An offset page is convenient for the MVP but can become slow or unstable on changing tables; connectors should advertise keyset support for a future upgrade.

### Edit rows

Cell edits are local drafts. A draft records the original value, new value, target identity, and validation state. Apply builds a parameterized mutation, displays the target and affected-row expectation, executes it in a transaction when the engine supports one, and refreshes the affected page. If the row changed underneath the user, report a conflict instead of overwriting silently. Delete follows the same review/confirmation path.

### Create a table

The schema form covers the portable MVP fields (name, columns, nullability, default, primary key). The connector validates and renders engine-specific DDL. Advanced constraints, generated columns, indexes, and engine options should be exposed only when the connector advertises support. Always show the exact DDL preview before applying it.

### Run SQL

The SQL console is available for relational connectors. It should support a selected statement or a clearly marked script, Stop/cancel, execution duration, affected-row count, and a result grid/error panel. Multiple statements and transaction semantics need an explicit policy per connector; do not imply that a script is atomic unless it is executed inside a transaction.

Redis gets a command-oriented surface in the same area of the application, with a safe command allowlist or a prominent acknowledgement for commands that mutate or enumerate large keyspaces. Redis values and TTL changes should use Redis-native operations and must not be represented as fake SQL.

## Engine-specific adapter rules

### PostgreSQL

Model database and schema separately. Quote identifiers with PostgreSQL rules, preserve rich types such as JSON/JSONB, arrays, UUIDs, and timestamps in `DbValue`, and use `$n` parameters. Prefer primary-key predicates for row mutations. Schema discovery should include indexes and constraints when available.

### MySQL

Distinguish server/database and account settings, quote identifiers with backticks, use `?` parameters according to the selected driver, and preserve charset/collation metadata where it affects editing or table creation. Account for MySQL-specific zero dates, unsigned integers, JSON, and generated columns in display/validation.

### SQLite

Treat the database path and its sidecar files as one database resource. Use SQLite's `PRAGMA` metadata and preserve declared types, `INTEGER PRIMARY KEY`/rowid behaviour, foreign keys, and transaction locking errors. The local SQLite connector is the preferred deterministic integration-test target. Do not copy or auto-delete a user's database file.

### Redis

Redis has no tables or SQL schema. Browse by key pattern, show key type and TTL, and load values with a bounded response. Support common string/hash/list/set/sorted-set/stream display and edits behind capability checks; large scans must use incremental `SCAN` rather than `KEYS`. Keep mutation commands explicit and surface TTL/expiry effects.

## Safety and security

DBX is a privileged client. The connector boundary is responsible for parameter binding, identifier quoting, TLS defaults, timeout/cancellation propagation, and redacted errors. The application boundary is responsible for confirmation UX, affected-row expectations, and not persisting secrets in workspace state.

Named profile metadata is persisted as versioned JSON in the platform configuration directory and re-read for each operation rather than cached as the source of truth. Passwords are removed from stored URLs and kept in the OS keychain. When the keychain is unavailable, saving a secret-bearing profile fails visibly; DBX does not silently degrade to a memory-only saved credential. Connection URLs shown in the UI and logs must redact passwords and tokens. Certificate verification stays enabled by default. Users should connect with least-privilege accounts and should be able to label a connection as read-only; a label alone is not a server-side permission.

Potentially destructive SQL should be visually distinct, but DBX cannot reliably classify every hand-written statement. Users remain responsible for the selected connection. Audit history should store redacted metadata (timestamp, connection label, duration, outcome), not secrets or full result sets, unless an explicit future setting says otherwise.

## Testing and verification

Tests should be layered:

1. Pure core tests for filter parsing/compilation, identifier quoting, value conversion, paging, and mutation previews.
2. Connector contract tests using a fake connector and SQLite fixtures for deterministic CRUD/error cases.
3. Engine integration tests against disposable PostgreSQL, MySQL, and Redis instances, gated behind explicit environment/configuration.
4. GPUI smoke tests for connection state, empty/loading/error states, keyboard navigation, filter application, edit review, and cancellation.
5. Manual checks for TLS, secret redaction, large result limits, and interrupted writes.

The normal local gate should be documented by the repository as it takes shape; the placeholder commands in the README are a starting point, not evidence that all drivers or UI flows are complete.

## Incremental delivery

The safest vertical slices are:

- GPUI shell + fake connector + loading/error states.
- SQLite discovery, table grid, filter builder, and CRUD.
- PostgreSQL and MySQL adapter parity for metadata, paging, filters, and writes.
- Relational SQL console with cancellation and result limits.
- Redis keyspace browser and command-aware value editor.
- Keychain/TLS hardening, conflict handling, accessibility, transfer progress/streaming for very large files, and future migration tooling.

Each slice should keep connector-specific code behind the core contracts and add an integration test before widening the UI surface.
