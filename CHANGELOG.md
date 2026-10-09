# Changelog

## 0.7.0 - 2026-10-09

A much stronger SQL editor:

- Check SQL as you type with a real SQL parser and the connection's schema: unknown statements, tables and columns are underlined, with the message on hover.
- Hover tables, columns, aliases and CTEs for their columns and types; Cmd/Ctrl-click a table to open it, or an alias or CTE to jump to where it is defined.
- Show signature help for built-in functions while typing their arguments, with signatures in hover cards and completion.
- Add multiple selections: Cmd/Ctrl-D adds the next match, F2 or Cmd/Ctrl-Shift-L selects every use of a name to rename it, and Escape returns to one caret.
- Fold parenthesized blocks and block comments from the gutter or with Cmd-Alt-[ / ] (Ctrl-Shift-[ / ]).
- Add line numbers, current-line, matching-bracket and identifier-occurrence highlights, bracket and quote pairing, Tab/Shift-Tab indentation, line comments, moving, duplicating and deleting lines, and word-sized undo.

Also:

- Group the query options menu into Diagnostics, Copy result, Export, Saved queries, History and Timeout submenus, with checkmarks for the active settings, and line up the query name and Save on the left of the toolbar.
- Bind PostgreSQL query parameters with the types the server expects, so values compare correctly against uuid, numeric and other typed columns.
- Split scripts with one shared scanner, so the statement the editor highlights is exactly the one the console, imports and runs execute.
- Stop treating `#` as a comment outside MySQL, BigQuery and ClickHouse, so PostgreSQL `#`, `#>`, `#>>` and `#-` operators are no longer cut from executed statements.
- Run a SQLite trigger's whole `BEGIN … END` body when the caret is inside it.

## 0.6.0 - 2026-10-08

- Move profile import/export into Settings and the demo database option into New connection.
- Keep cell text in place when entering inline edit mode and typing.
- Read PostgreSQL browser values as text and preserve guarded update behavior.

- Fix literal dotted identifiers and atomic result-file replacement; add explicit loaded/full-query export choices.
- Add protected profile migration, duplicate/delete management and authenticated encrypted portable bundles, including TablePro v1 imports.
- Integrate PostgreSQL/MySQL native backup tools, cancellation, private credentials and reviewed restores.
- Add type/default/nullability and PK/FK/CHECK schema drafts, metadata selectors and combined SQL review.
- Make SQL Server query tabs independent and staged changes atomic; show connector workflow capabilities.
- Preserve strict TLS identity through SSH for query connections; refresh supported CLI cloud credentials on connect and new pool connections.
- Encrypt staged changeset recovery and require schema/conflict validation before applying it.
- Add searchable history, recording/retention controls, reviewed CSV/JSON mapping, bounded copy and data comparison.
- Add explicitly paired local read-only MCP access, a disposable demo, task-oriented docs and Intel Mac build/release jobs.

See the capability matrix and verification report for connector and environment limits.
