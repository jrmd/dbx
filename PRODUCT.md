# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Users

DBX is for developers and data operators who need to inspect, query, and safely edit databases from a fast native desktop application.

## Product Purpose

DBX provides one workbench for PostgreSQL, MySQL, SQLite, and Redis. It lets users configure a connection through equal Details and Connection String modes, save and name connections, keep several connections open simultaneously, browse schemas and tables, filter records with structured controls, inspect and edit complete rows, create tables, and run highlighted SQL.

## Positioning

DBX combines the dense, keyboard-friendly workflow of established database clients with a fully native GPUI interface and no Electron, Tauri, webview, or browser runtime.

## Operating Context

Users move repeatedly between saved connections, schema navigation, table data, table structure, row inspection, and query execution. PostgreSQL users often scope navigation to a schema such as `public` or `drizzle`; SQLite users select a database file directly.

## Capabilities and Constraints

- PostgreSQL, MySQL, SQLite, and Redis are required engines.
- Connection setup must offer Details fields for host, port, user, password, and database alongside an equally capable Connection String mode; SQLite must offer a native database-file chooser.
- Connections can be named, saved to disk, opened concurrently, and switched through persistent primary tabs.
- Test Connection must validate the entered configuration without saving it, opening a workspace, or changing database state.
- Saving uses the user-provided connection name for the profile and OS-keyring lookup; connection secrets must use the OS keyring and must never be written to the saved connection JSON file.
- Database-only rail controls must remain hidden until a connection is active.
- Every connection can keep multiple independent query documents and table-bound structure documents open at once.
- SQL editing requires syntax highlighting.
- Table browsing supports predefined multi-row filters, full-row insert and edit, refresh, and destructive table actions behind confirmation.
- Tables can be exported to SQL dump, CSV, or TSV files (optionally gzip-compressed) through a native save dialog, and compatible files can be imported: SQL dumps replay their statements behind an explicit confirmation, while CSV/TSV files bulk-append rows whose header maps to the table's columns.
- Structure documents expose columns, primary keys, and normalized foreign-key relationships for PostgreSQL, MySQL, and SQLite.
- The application must remain native and responsive under large result sets.

## Brand Commitments

The product name is DBX. The supplied DBX logo asset anchors the disconnected connection setup/top-bar identity and the connected app rail; the connected workspace should not repeat the same mark in both places. The supplied DBX screen-map image at `/tmp/codex-clipboard-9272fa2f-5a32-4169-962f-0c077297d380.png` is the binding visual reference for the desktop workbench: compact dark surfaces, crisp blue navigation and actions, green connection health, and dense professional database tooling.

## Evidence on Hand

The repository contains working native GPUI screens and database operations for the required engines, persisted profiles, structured filters, multi-connection sessions, row editing, schema filtering, query highlighting, and Docker-backed integration fixtures. No customer, benchmark, or commercial claims are supplied.

## Product Principles

- Keep database context visible while users move between data, structure, and queries.
- Prefer immediate native interaction and dense scanability over decorative chrome.
- Make destructive operations explicit and recoverable where the database permits.
- Preserve user input and connection context through errors and asynchronous refreshes.
- Never trade secret safety for connection convenience.
