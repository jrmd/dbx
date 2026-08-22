# DBX

DBX is a native, fast database workbench built with Rust and [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui). It is intended to make everyday database work feel as direct as a desktop editor: inspect a schema, filter a table, edit a row, or run SQL without giving up the responsiveness and isolation of a native application.

The initial engine set is:

- PostgreSQL
- MySQL
- SQLite
- Redis

DBX is an early, runnable MVP. It already has a native connection picker, simultaneous connection tabs, schema/keyspace navigator, result grid, structured multi-rule filters, an all-field row inspector, guarded insert/update/delete operations, table truncate/drop menus, create-table SQL templates, and a raw SQL/Redis command console. It should not yet be treated as a production database administration tool.

## Implemented in this milestone

- Native GPUI application shell with IME-aware text editing and SQL syntax highlighting.
- Named connection profiles persisted as atomic, versioned JSON metadata; passwords are stripped from URLs and stored only in the OS keyring.
- Isolated simultaneous connection sessions with switchable, closable tabs and generation-safe asynchronous updates.
- Native SQLite file selection through the operating system file portal.
- PostgreSQL, MySQL, and SQLite connections through SQLx, plus Redis through redis-rs.
- Table/view discovery, PostgreSQL schema chips (including `public`, Drizzle-owned schemas, and an all-schemas view), column and primary-key introspection, bounded row pages, and result grids.
- Multiple structured GUI filters with column/comparator/value controls, typed values, dialect-aware identifier quoting, and PostgreSQL bind markers.
- Click-to-edit rows with every table field available in one scrollable, typed draft.
- Right-click table actions for refresh, truncate, and drop, with engine-aware SQL and explicit destructive confirmations.
- Full-row insertion with explicit Value/NULL/Default states, multi-field primary-key-guarded updates, and primary-key-guarded deletion.
- Raw SQL execution for relational databases and a raw Redis command surface; Redis browsing starts with incremental `SCAN`.
- Table and database import/export: table context menus support SQL dumps, CSV, and TSV files, while the database-level export flow lets users select tables, choose SQL/CSV/TSV, set an output name and folder, optionally gzip, and export schemas only. Database SQL imports replay dump statements against the active connection after explicit confirmation; delimited imports remain table-targeted so their header mapping stays explicit. Unquoted empty delimited fields import as NULL while quoted empties stay empty strings.
- Docker-backed PostgreSQL, MySQL, and Redis contract coverage plus file-backed SQLite coverage for discovery, create, inspect, insert, GUI-style filtering, update, delete, SQL/commands, and Redis TTL/type scanning.

Current UX limitations are deliberate and visible: the table designer begins from an engine-aware SQL template, Redis values use the command console for mutation, and the first grid renders a bounded page rather than a fully virtualized multi-million-row dataset.

## What DBX is for

The first release is centred on a short, reliable workflow:

1. Add a connection and browse its databases, schemas, tables, or Redis keyspaces.
2. Open a table in a virtualized data grid and inspect rows without loading an entire table into memory.
3. Filter and sort rows through a GUI filter bar; inspect or edit the generated query before applying it.
4. Add, edit, and delete rows with a reviewable change set and an explicit apply action.
5. Create tables using a schema form, with a SQL preview for the exact DDL.
6. Run ad-hoc SQL in a console, view results and errors, and keep a small per-connection query history.

Redis is intentionally modelled as a key/value data source rather than pretending it has relational tables. Its browser will expose key patterns, types, TTLs, and values, while the SQL console is not available for Redis unless a future Redis SQL-compatible integration is added.

## Current scope

| Area | MVP direction | Deliberately deferred |
| --- | --- | --- |
| Connections | PostgreSQL, MySQL, SQLite, Redis; per-connection settings | Cloud-provider login flows, SSH tunnel management, team sync |
| Browsing | Tables/keyspaces, columns, types, indexes, paged rows | Full ER diagrams, data lineage, server monitoring |
| Editing | Insert, update, delete, create-table form, SQL preview, table/database import-export (SQL/CSV/TSV, gzip, schema-only SQL) | Migrations, schema diff/deploy, server-side bulk loaders, partial-column CSV mapping |
| Queries | SQL editor for relational engines, cancellation, result grid | SQL autocomplete parity with a full IDE, query plan visualizer |
| Filtering | Structured predicates compiled to parameterized SQL; Redis key/type filters | Saved team-wide searches and cross-database joins |

The architecture keeps these deferred features possible without making them dependencies of the first usable build. See [docs/architecture.md](docs/architecture.md) for the boundaries and engine-specific rules.

## Getting started

The commands below are the intended development entry points. Adjust the binary/package names if the workspace layout changes while the MVP is being assembled.

```bash
# Rust stable is recommended.
rustup toolchain install stable
rustup default stable

# From the repository root:
cargo run --release --package dbx-ui

# Fast local checks:
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

For local macOS development, use the Makefile workflow instead of `cargo run`:

```bash
make build
make run
```

The first run creates a self-signed `DBX Local Development` code-signing
identity in the Mac login keychain, then reuses it to sign a stable
`target/macos/DBX.app` bundle with identifier `dbx.jrmd.app`. This is only for
local development; it does not require an Apple Developer account and does not
produce a distributable or notarized app. Launching the same signed bundle on
every run gives macOS a stable app identity for Keychain access. The first
access to an existing credential may still require choosing `Always Allow`
once, especially if it was created by an older unsigned build.

Use `DBX_FOREGROUND=1 make run` when you want the app's stdout/stderr in the
terminal. `make cargo-build` and `make cargo-run` remain available for the raw
Cargo workflow.

The canonical brand asset is [logo.svg](logo.svg). DBX embeds that SVG in the
native UI, so the active rail or disconnected top-bar mark stays sharp at any display scale without duplicating the brand in a connected workspace.

For packaged local builds:

```bash
# macOS: target/macos/DBX.app, with Contents/Resources/DBX.icns
make macos-build

# Linux: target/linux/DBX plus target/linux/dbx-linux.tar.gz
make linux-build
```

The Linux staging tree includes `usr/share/applications/dbx.desktop` and the
scalable `usr/share/icons/hicolor/scalable/apps/dbx.svg` icon. The macOS
bundle includes the SVG source and generates the Finder `.icns` resource from
the same artwork.

Run the disposable connector suite with Docker:

```bash
./scripts/test-integration.sh
```

The script starts PostgreSQL 16, MySQL 8.4, and Redis 7 on loopback-only test ports, creates a temporary SQLite database, waits for health checks, runs the ignored connector tests serially, and always tears everything down. It uses only fixed test credentials and ephemeral `tmpfs`/temporary-file storage. Existing `DBX_TEST_POSTGRES_URL`, `DBX_TEST_MYSQL_URL`, `DBX_TEST_REDIS_URL`, and `DBX_TEST_SQLITE_URL` values override its defaults.

Never commit a real connection string, password, certificate private key, or local database file. `.env.example` may document safe placeholders; `.env` and local database artifacts are ignored by default.

## Product principles

- **Native first:** GPUI owns the window, input, layout, and rendering. There is no Electron, Tauri, embedded browser, or webview requirement.
- **Fast by default:** connection work and queries run away from the GPUI event loop; row fetching is streamed and bounded, with full grid virtualization planned next.
- **Explicit writes:** GUI edits are drafts until the user reviews and applies them. Destructive operations need a clear confirmation and report the generated SQL.
- **Engine-aware, not lowest-common-denominator:** a shared interface covers common work, while PostgreSQL, MySQL, SQLite, and Redis keep their important differences.
- **Safe failure:** timeouts, cancellation, typed errors, and redacted diagnostics are part of the interface, not afterthoughts.

## Security expectations

DBX handles credentials and potentially destructive commands, so an MVP must be honest about what it does and does not protect:

- Named profile metadata lives in the platform configuration directory rather than a process-only cache. Passwords are never written to the profile JSON; if the OS keyring is unavailable, saving a secret-bearing profile fails visibly instead of silently retaining the secret only in memory.
- TLS certificate verification is enabled by default. An insecure or certificate-bypass option, if ever added, must be conspicuous and scoped to one connection.
- Query text, parameters, result values, and credentials must not be written to logs or crash reports by default. Diagnostics should redact connection URLs and secrets.
- GUI filters and edits use bound parameters wherever the target engine supports them. Identifiers are quoted by the connector, never interpolated from unchecked text.
- A connection should default to read-only browsing where the engine permits it, and write actions should show the target connection/database and affected-row estimate.
- DBX is a client, not a privilege boundary. Use a least-privilege database account and a disposable database for experiments.

Robust read-only sessions, SSH tunnels, and a formal audit log remain security work items—not claims made by this scaffold.

## Roadmap

1. Add test-connection UX, explicit TLS controls, and editable engine-specific connection fields.
2. Replace the bounded first grid with multi-million-row virtualization, resizable columns, multi-column edits, and keyset pagination.
3. Add a structured table designer with generated DDL review, indexes, constraints, and engine-aware types.
4. Expand Redis into typed string/hash/list/set/sorted-set/stream inspectors with TTL editing.
5. Harden cancellation, transactions, query history, SSH tunnels, accessibility, and cross-platform packaging through real-world testing; extend transfers with progress, streaming, and native bulk-loader hand-off for very large files.

Contributions should keep the UI responsive, preserve parameterization, and include a connector-level test for any engine-specific behaviour.
