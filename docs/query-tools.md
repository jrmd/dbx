# Query tools

Open the query tab's **Query options** menu for plans, schema comparisons,
watched queries, exports, and statistics.

## Execution plans

**Explain query** plans one selected or current read statement without running
`ANALYZE`. PostgreSQL and MySQL JSON plans open as an indented plan tree; **Show
grid** switches to the same data in a table. Other supported connectors keep
native plan rows in the result grid. The tree shows operation, estimated rows,
planner cost, and relation/index/filter details. Planner cost is not elapsed time.

**EXPLAIN ANALYZE… (executes query)** is available for PostgreSQL only. The review
dialog displays the SQL before execution. DBX opens a separate connection, runs
`BEGIN READ ONLY`, executes `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)`, and rolls
back. Your query tab's connection and any open transaction are kept separate;
uncommitted changes in that tab are not visible to this plan. Actual rows, time
per loop, and loop count appear alongside the estimates when PostgreSQL supplies
them. Database functions can have external effects that a rollback cannot undo.

Analysis uses the tab's timeout and normal cancellation controls. Errors or
cancellation close the isolated connection and its transaction. Other providers
keep non-executing EXPLAIN; DBX does not emulate transaction isolation for them.

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

For PostgreSQL, **Watch read query** repeats the selected or current read
statement every 5, 10, 30, or 60 seconds while its query tab is visible. Each
sample uses a fresh server-enforced read-only transaction and rolls it back.
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

## Foreign-key JOIN completion

Immediately after `JOIN`, table completions can include **FK JOIN** snippets
from the background schema snapshot. A snippet inserts the related table and
its `ON` predicate, including every column in a composite foreign key. Existing
aliases and provider identifier quoting are respected. Incoming and outgoing
relationships are offered; ambiguous unqualified source names are skipped.
The snippet is inserted into the editor for review and does not execute SQL.
