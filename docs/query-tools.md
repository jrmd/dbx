# Query tools

Open the query tab's **Query options** menu for plans, schema comparisons,
watched queries, exports, and statistics.
Menus hide actions the active driver does not support. Supported actions can
remain disabled while busy, without a query or result, or when protection blocks writes.

## Execution plans

**Explain query** plans one selected or current read statement without running
`ANALYZE`. PostgreSQL and MySQL JSON plans open as an indented plan tree; **Show
grid** switches to the same data in a table. Other supported connectors keep
native plan rows in the result grid. The tree shows operation, estimated rows,
planner cost, and relation/index/filter details. Planner cost is not elapsed time.

**EXPLAIN ANALYZE… (executes query)** supports native PostgreSQL, MySQL
8.0.18+, MariaDB, and CockroachDB connections. The review dialog displays the SQL before
execution. Each analysis opens a separate connection and a server-enforced
read-only transaction, then rolls it back. Your query tab's connection and open
transaction stay separate; its uncommitted changes are not visible to analysis.
PostgreSQL returns its JSON plan with actual rows, time per loop, and loop counts.
MySQL `TREE` and CockroachDB `PLAN` output also open as indented trees. Explicit
actual metrics are shown where the native plan names them; all native node
attributes remain in the detail column. DBX detects MariaDB on the isolated
connection and uses its native `ANALYZE FORMAT=JSON`, including measured rows, table time, and loop counts.
Unsupported older server versions report their native error; DBX never falls
back to a writable transaction.
SQLite has no equivalent execution-plan ANALYZE command; its **Explain query**
uses non-executing `EXPLAIN QUERY PLAN`, and the ANALYZE action is hidden.

Analysis uses the tab's timeout and normal cancellation controls. PostgreSQL
cancels the backend query, MySQL uses `KILL QUERY`, and CockroachDB cancels queries
belonging to the isolated session ID. Errors or cancellation close the isolated
connection and its transaction. Database functions can still have external effects
that rollback cannot undo. Read-only transactions protect database tables; they
are not a sandbox for untrusted functions or external services.

Under **Diagnostics**, **Capture plan** stores the current plan in the tab;
explain another statement and choose **Compare plans** to compare steps. Actual
measurements exist only for steps obtained through ANALYZE.

## Compare open connections

**Make this connection’s schema match…** lists other open connections using the
same SQL dialect. Choose the connection containing the desired schema. DBX reads
both schemas and opens an editable migration draft on the current target
connection. The draft identifies **TARGET** and **DESIRED SCHEMA FROM** at the top.
No migration runs automatically.

Finish running queries and open transactions on both connections before
comparing. Changing either database or connection invalidates an outstanding
comparison. Review the SQL and warning comments; ambiguous or destructive
changes remain commented for review. Coverage depends on the connector's
captured metadata; grants are not captured. This creates a schema migration,
not a data migration.

**Capture schema baseline** and **Compare schema / draft migration** remain
available for comparing one connection against an earlier snapshot.

## Watch a read query

For native PostgreSQL, MySQL/MariaDB, CockroachDB, and file-backed SQLite,
**Watch read query** repeats the selected or current read statement every 5, 10, 30, or 60 seconds while its query tab is visible. Each
sample uses a fresh server-enforced read-only transaction and rolls it back.
SQLite opens the database file through an independent read-only connection with
`query_only=ON`; it never changes the tab or pool’s PRAGMAs. Private in-memory
SQLite databases cannot be reopened as an independent committed snapshot, so
watching them reports an unsupported-operation error. Community drivers and
remote SQLite services do not share this native isolation path.
Only one sample runs at a time; a slow sample makes the timer skip that tick.
Normal result limits and the query timeout apply.

**Stop watch** cancels an in-flight sample. Editing SQL, running a manual query,
a failed sample, changing databases, or closing the tab stops its watch. A hidden
tab pauses sampling. Watch state is not restored when reopening the application.
Finish an open transaction before starting: samples read committed database
state, independently of a query tab's temporary state or uncommitted writes.
Functions can still have external effects; use statements appropriate to repeat.
Watch samples do not fill the persistent query history.

## Statistics for loaded rows

**Column statistics (loaded rows)** is available from query options and the
loaded table's menu. It opens a separate statistics tab with loaded row count,
null count, distinct non-null client values, and numeric count/minimum/maximum/
mean. It performs no database query or full-table scan. A table page or limited
query result describes only the rows already loaded; the status repeats that
scope and notes a limited source result.

Distinct counts use typed client values rather than database collation. Numeric
summaries marked `~` use floating-point approximations; nonnumeric columns have
empty numeric summaries. These are exploratory summaries, not database-wide
statistics or exact decimal accounting.

## Full-table statistics

The loaded table’s menu also offers **Column statistics (full table, scans all
rows)** for native PostgreSQL, MySQL/MariaDB, CockroachDB, and file-backed SQLite.
This explicit action queries all rows visible to the database user in one
isolated read-only aggregate statement, ignoring the loaded page and grid
filters. It opens a separate query tab showing the SQL; use its normal **Stop**
and timeout controls. It may scan the table and use server memory for exact
`COUNT(DISTINCT ...)`, so run it when that cost is appropriate.

Results include full-table row counts, nulls, distinct non-null counts, and
minimum/maximum values. The server computes extrema using each column’s native
type and collation. Large integers and exact decimals retain their decoded values;
DBX does not convert them through floating point. Floating-point source columns
retain their native floating-point precision. Complex/custom types without
supported aggregate operators show counts/nulls and an explicit unsupported
marker for distinct/min/max; booleans and UUIDs have distinct counts but no range
where native range operators are unavailable. SQLite uses its native dynamic
storage-class comparison order. Server privileges, row-level security, and
statement-width limits still apply. These statistics are separate from the
approximate, client-only **loaded rows** statistics.

## Foreign-key JOIN completion

Immediately after `JOIN`, table completions can include **FK JOIN** snippets
from the background schema snapshot. A snippet inserts the related table and
its `ON` predicate, including every column in a composite foreign key. Existing
aliases and provider identifier quoting are respected. Incoming and outgoing
relationships are offered; ambiguous unqualified source names are skipped.
The snippet is inserted into the editor for review and does not execute SQL.

Engine references: [MySQL read-only transactions](https://dev.mysql.com/doc/refman/8.4/en/commit.html),
[SQLite query_only](https://www.sqlite.org/pragma.html#pragma_query_only), and
[CockroachDB cancellation](https://docs.cockroachlabs.com/docs/stable/cancel-query).

## Excel and Markdown exports

**Copy as Markdown** and loaded-row **Excel XLSX** export use the rows already
in the grid. They exclude staged edits and do not silently fetch missing pages.

For native PostgreSQL, MySQL/MariaDB, CockroachDB and SQLite connections, choose
**Export → Full query · fresh read-only rerun → Excel XLSX** to execute one read
statement again without the grid row limit. Finish the tab's transaction and stop
its watch first. Parameters must be replaced with reviewed literal values before
export; mutation statements and multiple statements are rejected.

Right-click the table grid for **Copy as Markdown** (selected rows),
**Copy all loaded rows as Markdown**, or **Export XLSX**.
The **Export XLSX** submenu also offers **All table rows**, which exports the
whole table and explicitly excludes grid filters and staged edits. Full exports
use a separate read-only transaction; snapshot consistency depends on the
server and table engine. They do not include a tab's uncommitted changes.

Rows spool to private temporary files instead of accumulating in memory.
Progress reports rows and output bytes; **Cancel** stops streaming or workbook
finalization. The destination is replaced atomically only after success. Failure,
cancellation, or an Excel format limit leaves an existing destination intact.
Excel permits 1,048,575 data rows plus the header, 16,384 columns and 32,767
characters per cell; exceeding a limit produces an error instead of truncating.

Formula-looking values and URLs remain literal text. SQL NULL is blank, empty
text uses the fixed formula `=""`, and integers beyond 15 digits are stored as
exact text. JSON and hexadecimal bytes are text. A **Column types** sheet records
the export scope and database types. Other full-result formats remain available
through the existing CSV/TSV/SQL export tools.
