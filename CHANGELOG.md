## 0.10.0 - 2026-10-13

- Make the schema designer compact with wrapping action groups, existing index and constraint choices, and custom enum type shortcuts.
- Protect constraint-backed indexes from DROP INDEX, show full index definitions and owning constraints, and let column shortcuts toggle selections.
- Clear fields when changing designer actions while preserving queued SQL, and allow reviewing a queued plan from an empty form.
- Prefer GitHub CLI authentication for update checks, including standard Homebrew installations on macOS. Fall back to the public GitHub API if the CLI is missing or fails, with bounded runtime and metadata size.
- List PostgreSQL enum types and their definitions in the schema explorer, show ordered enum values in table details, and include enum definitions before tables in SQL exports.
- Preserve empty enum labels and separator characters, resolve enum values through domains, decode enum arrays as editable PostgreSQL array literals, and retain case in quoted type names.

## 0.9.1 - 2026-10-13

- Fix scrolling in Structure so the column grid stays complete and index predicates no longer overlap the next entry.
- Keep the table header visible, allow horizontal column scrolling in narrow windows, and group metadata into expandable sections with counts and per-tab state.

## 0.9.0 - 2026-10-12

- Highlight JSON values in table cells and inspectors with the editor's syntax colors.
- Keep vertical scrolling in the row inspector working over wide JSON and hex previews.
- Make the table designer easier to use with grouped Columns, Indexes and Constraints actions, type and column shortcuts, and labelled checkboxes.
- Keep designer fields and queued changes when closing the form, and bring Add to plan and Review SQL plan together in its footer.

## 0.8.1 - 2026-10-12

- Preview UTF-8 JSON objects and arrays returned as bytes as formatted JSON in row and query inspectors, and copy the full JSON text.
- Use the configured monospace font and horizontal scrolling for JSON and hex previews so long lines keep their layout.
- Fix keyboard accessibility tests on macOS by using platform shortcuts and a settings control available on every platform.
- Wait for the agent cancellation fixture to finish starting before checking its process, avoiding a race on busy CI runners.

## 0.7.3 - 2026-10-09

- Drag the edge of the explorer sidebar or the row details panel to resize it. DBX remembers the widths; double-click an edge to restore the default.

# Changelog

## 0.7.2 - 2026-10-09

MariaDB fixes:

- Open MariaDB tables again. Loading a table's indexes failed with `Unknown column 'EXPRESSION'`, as did MySQL releases before 8.0.13.
- Read MariaDB column defaults correctly: nullable columns no longer show a `'NULL'` default, and string and function defaults keep their SQL form.
- Show only a table's own check constraints on MariaDB, where constraint names are only unique per table.
- Show MariaDB JSON columns, and any MySQL text column with a binary collation such as `utf8mb4_bin`, as text instead of raw bytes.
- Back up MariaDB databases with either MariaDB's or MySQL's client tools, including installs that only provide `mariadb-dump` and `mariadb`.
- Export MySQL string defaults containing an apostrophe; they previously failed with "invalid metadata expression".

## 0.7.1 - 2026-10-09

- Show schema objects as compact sidebar rows: names truncate cleanly, the object type sits on the right, and hovering shows the full signature.
- Stop building Intel macOS packages; Apple Silicon is the supported Mac platform.

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
