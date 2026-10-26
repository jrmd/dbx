# Changelog

## 0.18.0 - 2026-10-26

- Use an AppKit sidebar and toolbar for Liquid Glass on macOS 26 or later, with native scroll-edge effects and stable opaque tables, editors and forms. Turn it on or off in Settings → Appearance; macOS accessibility preferences take priority. Connection tabs show their tag in its colour, and content panels follow the window's corner radius.
- Drag connection tabs and document tabs to reorder them; the order is restored with the workspace. Untitled queries keep their numbers when moved.

## 0.17.3 - 2026-10-21

- Keep sidebar scrolling outside unfocused row-value fields, and reveal hidden text while drag-selecting beyond a field’s edges.

## 0.17.2 - 2026-10-21

- Right-click document tabs to close the tab, other tabs, or tabs to the right; right-click connection tabs to close the connection or other connections. Bulk closing respects unsaved edits.

## 0.17.1 - 2026-10-21

- Show readable binary columns, including JSON stored in MySQL BLOBs, as UTF-8 text by default. Choose Text, Hex, Bits or UUID in the grid and row details; text edits preserve binary storage.

## 0.17.0 - 2026-10-20

- Choose a separate row font and font size for table values and row details in Appearance settings, with a monospace default.
- Fix a crash when rendering numeric settings controls with accessibility enabled.
- Build, sign and notarize releases locally on the maintainer’s Mac, including Linux packages through Docker; retain automatic fast CI and database checks.

## 0.16.1 - 2026-10-20

- Switch binary values between hex, bits, UUID and MySQL swapped UUID directly in row details.
- Keep overflowing tables' horizontal scrollbar visible so columns remain reachable with a mouse.

## 0.16.0 - 2026-10-20

- Select multiple sidebar tables with Control/Command-click and open, copy names, export, truncate or delete the selection. Truncate confirmations offer optional cascading for PostgreSQL and CockroachDB.
- Simplify New connection: a quieter picker with a More menu, cloud presets alongside the databases and community drivers folded away; PostgreSQL/MySQL authentication is one selector beside the credentials, and Network settings stay collapsed until configured.
- Choose a connection folder from a menu of existing folders, or create one inline with New folder…; names are checked before they are applied and match existing folders regardless of case.
- Centre database tile contents vertically, with consistent padding and room for subtitles at larger font sizes.

## 0.15.0 - 2026-10-20

- Add optional Touch ID vault unlock on Macs with a suitably signed and provisioned build, while keeping passphrase unlock available.
- Match context-menu spacing across the workbench, remove the extra row-number column, and let Backspace stage selected-row deletions alongside Delete.
- Allow selecting and copying part of a row-inspector value; align its row-action buttons and the filter controls, with a compact Add condition button.
- Choose the query-assistant provider from a menu so its label stays readable at any width or interface size; row-inspector Copy buttons now use a copy icon and briefly confirm with a check and “Copied”.
- Simplify empty structure sections and show table definitions as formatted, syntax-highlighted SQL.

## 0.14.0 - 2026-10-20

- Show failed statements as errors without a contradictory success message; preserve database detail, hints and source positions, and offer an editable AI repair request.
- Accept SQL completions with Tab or Enter and suggest JOIN conditions from foreign keys.
- Open SQLite from plain paths, home-directory paths, native file selection, dropped files and terminal arguments; new connections land in an empty workspace.
- Keep long notifications readable and pause their dismissal on hover. Prioritize grid column names and route ER relationships around intervening tables.
- Add an actions palette, a keyboard-shortcuts settings screen and editable JSON bindings. Settings opens on Appearance.
- Add table name/DDL/INSERT copying and rename/duplicate SQL drafts; require the exact table name for production truncate/drop and warn about production/protected UPDATE or DELETE without WHERE.
- Add connection folders, production window tint, hosted provider presets, and reviewed imports from DBeaver, Docker Compose and DATABASE_URL files.
- Add bounded staged-edit undo/redo, bulk value/NULL edits, Markdown copying and XLSX export of loaded results.
- Save and open shareable SQL files with directory-based favourites, alongside the existing encrypted workspace.
- Bundle ten palettes and import Zed theme families as well as VS Code themes.
- Add visual plan trees, explicit PostgreSQL EXPLAIN ANALYZE in a separate rolled-back read-only transaction, and migration drafts comparing two open connections.
- Add PostgreSQL query watch intervals and clearly scoped statistics for loaded result columns.
- Add MongoDB document replacement and structured Redis value/stream editors with explicit review and conflict checks.
- Generate query drafts through OpenAI, Anthropic or Ollama without an installed agent CLI; API keys stay in the encrypted vault.
- Register community driver executables through a bounded JSON process protocol, without adding native driver dependencies to DBX.
- Add the MIT license, contribution guidance and GitHub templates, and publish version-specific changelog text in release bodies.

## 0.13.0 - 2026-10-19

- View and edit binary values as hex bytes, binary digits, UUIDs, or MySQL swapped UUIDs.
- Keep single-line fields tall enough for larger text, preserving vertical padding and text visibility.

## 0.12.0 - 2026-10-15

- Choose from six colour themes (DBX, Ocean, Forest, Plum, Sand, and Graphite), each with light and dark variants, in a Theme group at the top of Appearance settings.
- Import VS Code `.json`/`.jsonc` colour themes and DBX theme files, keep several custom themes, and remove each one from its own thumbnail. Re-importing a theme with the same name updates it.
- Edit any interface, grid, or SQL syntax colour with a searchable colour editor and live preview. Editing a preset saves it as a new custom theme, and themes can be exported to share.
- Switch to light or dark automatically when selecting a theme made for one appearance.
- Keep imported themes readable: faint or translucent text and borderless inputs are raised to accessible contrast, and syntax colours come from each theme's general scopes rather than language-specific overrides.

## 0.11.1 - 2026-10-14

- Keep the active row highlighted across its full width when selecting individual cells, and preserve selected-row highlights when hovering.

## 0.11.0 - 2026-10-14

- Choose interface and monospace fonts with searchable font pickers, and adjust each font size independently using numeric inputs and minus/plus buttons.
- Grow controls, table rows, and columns with larger text, wrap crowded layouts, and keep column resize handles aligned at different zoom levels.
- Select data rows by clicking and dragging, with a preference enabled by default. Shift-click and Command/Control-click remain available.
- Apply context-menu JSON, TSV, SQL, and cell copying, duplication, deletion/restoration, and reversion to the selected rows. Single-row inspector, editor, and value-filter actions are disabled for multiple selections.
- Clear stale column context when right-clicking the row gutter and protect row actions from stale results.

## 0.10.2 - 2026-10-13

- Fix JSON and JSONB cell editors so typing, selection, keyboard editing, and staging changes work.
- Prompt for the destination table before copying or exporting query results as INSERT statements.

## 0.10.1 - 2026-10-13

- Use the theme's monospaced font in SQL, JSON, and Redis editors, including single-line SQL expressions and SQL line numbers.
- Use the same monospaced font in SQL previews, table defaults, and schema definitions.

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

## 0.8.0 - 2026-10-11

- Improve keyboard navigation, accessibility and workbench performance.
- Add accessibility verification coverage for native and virtualized data tables.

## 0.7.3 - 2026-10-09

- Drag the edge of the explorer sidebar or the row details panel to resize it. DBX remembers the widths; double-click an edge to restore the default.


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

## 0.5.0 - 2026-10-08

- Stage and review multi-row changesets before saving; resize columns, jump to tables with quick open, and browse large tables with bounded paging.

## 0.4.0 - 2026-10-07

- Add safer query sessions, database diagnostics, workspace recovery and expanded connector integration coverage.
- Configure CLI query agents and use table row actions.

## 0.3.2 - 2026-10-01

- Fix SQL autocomplete in release builds when consuming opening quotes.

## 0.3.1 - 2026-10-01

- Add the ClickHouse connector and reuse release build dependencies.

## 0.3.0 - 2026-10-01

- Add eight database connectors, a database-type-first connection picker and a Settings screen.

## 0.2.2 - 2026-10-01

- Fix the macOS updater signing requirement.

## 0.2.1 - 2026-10-01

- Improve application updates and add Unix socket and SSH connections.

## 0.2.0 - 2026-10-01

- Move updates and release notes into Settings.
- Package Apple Silicon macOS builds and use the host xkbcommon library in Linux AppImages.

## 0.1.0 - 2026-09-30

- First public release: browse PostgreSQL, MySQL, SQLite and Redis in a native Rust/GPUI workbench.
- Edit relational rows, run SQL or Redis commands, import/export data and store credentials in an encrypted vault.
