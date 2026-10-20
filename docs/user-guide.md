# Daily work and switching to DBX

DBX is a preview workbench for Linux x86_64 and Apple Silicon macOS. Begin with
non-production data and a database account whose privileges match your work.
The connection form shows the connector's workflow capabilities. See the
[capability matrix](capability-matrix.md) before migrating engine-specific tasks.

## Try a sample

Unlock the vault, open **New connection**, and choose **Try a demo database**. Each
click creates a separate local SQLite file with teams and 250 projects; existing
files are preserved. Browse projects, open a row's team link, filter budgets,
stage a name change, review the SQL and commit or discard. The demo profile
persists normally and can be deleted through the connection form. Deleting a
profile removes its saved credentials, not its database file.

## Switch from TablePlus or TablePro

For TablePro, export connections and choose **Import profiles** in DBX under
**Settings → Connections**. Plaintext
JSON and encrypted `TPRO` v1 bundles are accepted; review every host/database,
TLS/SSH setting and warning. Imported profiles start protected and never connect
or execute startup commands. Key/certificate paths are references; copy those
files yourself. Unsupported custom credentials and tunnel/startup settings need
manual review. TablePlus's proprietary `.tableplusconnection` format is not
parsed: copy its connection URL into DBX and review TLS/SSH details in the form.

Rename a selected profile and Save; Duplicate makes a new identity with its own
credential entry; Delete is refused while that profile has open sessions. In
**Settings → Connections**, use **Export profiles** for password-free metadata.
Use **Export encrypted bundle** only when moving
credentials, with a separate 12-character-or-longer passphrase. Keep both exports
private; neither includes key/certificate file contents.

## Find and query

Use **Import connections…** on the connection screen for DBeaver connection
JSON, Docker Compose YAML and `.env` files containing `DATABASE_URL`, as well
as the existing DBX/TablePro formats. Review detected hosts, published ports,
database names and warnings before saving. Compose variables are not evaluated
as shell commands; unresolved values require manual entry. Imports do not run
startup SQL. Profiles can be grouped in named folders from their connection form.

`dbx /path/to/file.sqlite` and `dbx postgres://…` connect directly to that file
or URI after vault unlock. SQLite also accepts `~` paths,
the native file picker and dropped files. New connections do not automatically
open their first table.

Quick open searches tables, open documents and saved queries. The query menu's
history search accepts `history: invoice success:true after:2026-10-01
before:2026-10-09 connection:local`. Dates are UTC; history for currently open
connections is searchable. Selecting a result opens a new draft without running
it. History is plaintext local JSON; pause recording or clear it when SQL contains
private business data. Encrypted workspace recovery is a separate store.

Run a selection or the current statement, use parameters, and inspect each
statement's result. Native PostgreSQL/MySQL/SQLite/CockroachDB and SQL Server
query tabs own independent sessions. An interrupted transaction is not replayed.
See [query tools](query-tools.md) for plan trees, opt-in PostgreSQL ANALYZE,
two-connection schema comparison, watch intervals and loaded-column statistics.
The [query assistant](query-agents.md) supports CLI agents, direct OpenAI and
Anthropic API keys, and Ollama. **Ask AI to fix** prepares an editable request
from a failed query; generation and execution remain separate actions.

## Write SQL

The editor checks SQL as you type against a real parser and the connection's
loaded schema, underlining unknown statements, tables and columns; hover an
underline for the message. Hover a table, column, alias or CTE to see its
columns and types, and a built-in function for its signature. While typing a
call's arguments, the current parameter is shown above the line.
Cmd/Ctrl-click a table to open it, or an alias or CTE to jump to its definition.

| Action | macOS | Linux |
| --- | --- | --- |
| Add next match as a caret | Cmd-D | Ctrl-D |
| Rename every use of a name | F2 | F2 |
| Back to one caret | Escape | Escape |
| Fold / unfold block | Cmd-Alt-[ / ] | Ctrl-Shift-[ / ] |
| Toggle line comment | Cmd-/ | Ctrl-/ |
| Move lines | Alt-Up / Down | Alt-Up / Down |
| Duplicate / delete lines | Shift-Cmd-D / K | Ctrl-Shift-D / K |

Folding also works from the chevrons beside the line numbers. Rename and
occurrence highlighting cover the current statement only.

### Vim bindings

Enable **Settings → Editor → Vim mode** for SQL query tabs. The preference is
off by default, survives relaunch, and applies to open and newly created SQL
tabs. Other inputs and query languages keep their ordinary editing behavior.

SQL tabs start in **Normal** mode with a block caret. Press `i` to type in
**Insert** mode, and `Escape` or `Ctrl-[` to return to Normal. The mode indicator
stays visible below the query toolbar. Completion appears in Insert mode.

| Task | Keys |
| --- | --- |
| Move | `h/j/k/l`, arrows, `w/b/e`, `W/B/E`, `0`, `^`, `$`, `gg`, `G` |
| Repeat a motion or operator by count | `3w`, `2dd`, `d2w`, `2d3w` |
| Insert | `i/a/I/A`, `o/O` (retains indentation) |
| Select | `v` for characters, `V` for lines; `o` swaps the selection endpoint |
| Delete, change, yank | `d/c/y` followed by a motion, or `dd/cc/yy`; also works on a Visual selection |
| Edit characters / line remainder | `x/X`, `r` followed by a character, `D/C` |
| Put | `p/P` |
| Undo / redo | `u`, `Ctrl-R`; one Insert session is one undo step |
| Move by about half a page | `Ctrl-U/Ctrl-D` |
| Leave the editor | `F6/Shift-F6`; Normal mode also supports `Tab/Shift-Tab` |

Query execution and clipboard shortcuts remain available in every mode.
Use the existing Cmd/Ctrl-F find bar for search. This binding set covers the
commands above; dot repeat, text objects, named registers, macros, Ex commands
and block Visual mode are not implemented.

## Navigate by keyboard

Cmd/Ctrl-Shift-P opens the action command palette. Cmd/Ctrl-P keeps destination
search for tables, queries, SQL files and connections. Use `files:` or
`favorites:` to narrow SQL file search; star buttons manage favourites.
**Settings → Keyboard shortcuts** groups live bindings by area and lets you
search by action or key. macOS and Linux show their own shortcut labels.
Expand **Customize shortcuts with JSON…** to edit/save the contextual
`keymap.json`; **Restore saved content** discards unsaved editor changes.
Unassigned palette actions can also receive custom bindings. Hover a row for
its action identifier and context. The screen displays the keymap's exact path. Each JSON block has an optional `context`
and a `bindings` object mapping keys to action names; `null` unbinds a shortcut.
Changes are validated before the file and live bindings change.

| Action | macOS | Linux |
| --- | --- | --- |
| Next / previous control | Tab / Shift-Tab | Tab / Shift-Tab |
| Leave an editor that uses Tab for indentation | F6 / Shift-F6 | F6 / Shift-F6 |
| Open a focused menu or activate a button | Enter or Space | Enter or Space |
| Focus the table explorer | Cmd-Shift-E | Ctrl-Shift-E |
| Move through tables | Up / Down, Home / End | Up / Down, Home / End |
| Open the table under the cursor | Enter | Enter |
| Table menu | Shift-F10 or Menu | Shift-F10 or Menu |
| Next / previous tab of the connection | Cmd-Alt-Right / Left | Alt-PageDown / PageUp |
| Next / previous connection | Cmd-Shift-] / [ | Ctrl-Tab / Ctrl-Shift-Tab |

Down in the explorer's search box continues into the list. In the table menu,
Up / Down / Home / End move over the available items, Enter runs one, and
Escape closes the menu and returns to the list. Items that the connection
cannot perform are skipped.

Quick Open and Settings return focus to their opener when dismissed. Closing a
tab through its focused close control continues to a nearby control. The
diagram's schema-filter popover keeps Tab and Shift-Tab within its controls;
Escape returns to the filter button. Focus a pane splitter with Tab and use
Left/Right to resize it in 10-pixel steps, or Home to restore its default width.

## Edit safely and resume work

Stage inserts, edits and deletes; inspect their combined SQL before committing.
Primary keys and original values detect concurrent changes. Protected profiles
block writes; provider permissions remain the final boundary. Encrypted staged
changes recover after reopening with matching database and column metadata.
Review recovered drafts explicitly; they never auto-apply. Query results and
open transactions are not restored. If the recovery budget is exceeded, save
large work separately before exiting.

Undo/Redo in the data toolbar or context menu changes staged edits only; it
does not reverse a committed database write. Select multiple rows for bulk
value/NULL staging. Production connections tint the workspace and require the
exact table name before truncate/drop. UPDATE/DELETE without WHERE prompts for
review on production or protected connections; protected execution still
rejects writes. MongoDB and Redis use [structured editors](structured-editors.md).

The query menu's SQL file library saves and opens plain `.sql` files in a
chosen folder, with favourites in `.dbx-favorites.json`. Cmd/Ctrl-O and
**File → Open SQL File…** open a file as a draft on the active SQL connection.
The command palette also offers SQL file browsing and folder selection.
Folder scans run in the background when quick open opens or you choose
**Refresh files**; rendering reads the last completed snapshot. Use the library
for queries you intend to share through git; encrypted workspace recovery
remains separate.

## Change structure

The Structure page keeps the table name and **Design table…** action visible
while the columns and metadata scroll. In narrow windows, scroll the column
grid horizontally to see every field. Expand **Foreign keys**, **Indexes**,
**Check constraints**, or **Definition** to inspect those details; counts show
how many entries are available. Each structure tab retains its scroll position
and expanded sections when switching tabs.

In Structure, click **Design table…** beside the table name. Choose an action
under Columns, Indexes or Constraints. Column and type shortcuts sit beneath
their fields; **Allow NULL values** and **Unique index** use checkboxes.
Select existing columns to populate properties, queue alterations with
**Add to plan**, then **Review SQL plan** in a new query tab. **Close designer**
hides the form while retaining its fields and queued changes. PostgreSQL/MySQL
support complete type/default/nullability definitions and PK/FK/CHECK drafts.
MySQL drafts preserve metadata-backed auto-increment, collation, comments and
ON UPDATE attributes; generated or unknown attributes require manual SQL. DDL may commit independently
on MySQL; back up affected data. Unsupported SQLite alterations require an
explicit table rebuild. Generated SQL is never executed automatically.

## Move and compare data

**Export loaded rows** saves only the current grid and identifies truncated
results. **Export full query** reruns one read query in a fresh read-only snapshot,
streams CSV/TSV/typed JSONL and atomically replaces the output only on success.
It cannot share an open transaction and is not a native backup.

Right-click a table and **Import data** for CSV/TSV/JSON/JSONL preview/mapping.
Unquoted CSV empty is NULL; quoted empty is empty text; JSON null is NULL.
JSON decimals and integers beyond 64 bits retain their original digits as bound
text for the destination's typed conversion; previews also preserve nested JSON
number digits. The destination type controls conversion and storage.
Review the destination types and first values, omit fields to use database defaults,
then append. A failed row rolls back the whole append. Preview/copy budgets are
100,000 rows and 64 MiB; SQL/gzip streaming remains available for larger inputs.

To copy between connections, **Capture table for cross-connection copy**, then
right-click the destination and **Append captured data**. It uses the same
mapping and rollback flow and does not replace existing keys. **Compare with
captured data** reports whole bounded snapshot counts by destination primary
key, with exact typed-value comparison. Capture times differ; no synchronization
writes are generated. XLSX viewing/import and automatic synchronization remain
outside this implementation.

## Back up and restore

**Export database…** uses DBX's exporter to write selected tables as SQL, CSV or
TSV, with gzip and schema-only SQL options. Use it for data transfers and
table-level exports. **Native backup…** uses the database vendor's tools for
recovery backups, including database details that DBX's export metadata does not
represent, such as grants and generated column expressions.

Use the database menu's native backup actions with installed `pg_dump` /
`pg_restore` or `mysqldump` / `mysql`. PostgreSQL uses a custom archive; MySQL uses
SQL. The job detects tools, reports progress/logs, supports cancellation and
keeps previous backups intact on failure. Confirm the exact restore database
and use a new empty target. PostgreSQL restore is transactional; MySQL DDL can
already be committed if cancellation or an error occurs. A backup can contain
executable database functions and triggers: restore only trusted input. Use
**View native job log** for diagnostics; credentials are delivered privately.
PostgreSQL's native tools preserve TLS identity over TCP SSH forwarding. Local
socket backup/restore supports password authentication; a local socket profile
that requires TLS is rejected because libpq ignores that requirement on sockets.
Use verified TCP or SSH for that native job. Native
MySQL clients skip TLS on Unix sockets, so a required/verified TLS native job
needs a direct TCP profile; DBX query sessions still verify TLS over SSH.

## Local agents

Use the database menu to share one database read-only through MCP, copy its
private configuration and inspect activity. Stop sharing to revoke it. See
[local MCP](local-mcp.md); today's draft-only query assistant stays available.
