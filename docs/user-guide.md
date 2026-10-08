# Daily work and switching to DBX

DBX is a preview workbench for Linux x86_64 and Apple Silicon macOS. Begin with
non-production data and a database account whose privileges match your work.
The connection form shows the connector's workflow capabilities. See the
[capability matrix](capability-matrix.md) before migrating engine-specific tasks.

## Try a sample

Unlock the vault and choose **Try a new demo database** in Connections. Each
click creates a separate local SQLite file with teams and 250 projects; existing
files are preserved. Browse projects, open a row's team link, filter budgets,
stage a name change, review the SQL and commit or discard. The demo profile
persists normally and can be deleted through the connection form. Deleting a
profile removes its saved credentials, not its database file.

## Switch from TablePlus or TablePro

For TablePro, export connections and choose **Import profiles** in DBX. Plaintext
JSON and encrypted `TPRO` v1 bundles are accepted; review every host/database,
TLS/SSH setting and warning. Imported profiles start protected and never connect
or execute startup commands. Key/certificate paths are references; copy those
files yourself. Unsupported custom credentials and tunnel/startup settings need
manual review. TablePlus's proprietary `.tableplusconnection` format is not
parsed: copy its connection URL into DBX and review TLS/SSH details in the form.

Rename a selected profile and Save; Duplicate makes a new identity with its own
credential entry; Delete is refused while that profile has open sessions. Export
password-free metadata by default. Use an encrypted DBX bundle only when moving
credentials, with a separate 12-character-or-longer passphrase. Keep both exports
private; neither includes key/certificate file contents.

## Find and query

Quick open searches tables, open documents and saved queries. The query menu's
history search accepts `history: invoice success:true after:2026-10-01
before:2026-10-09 connection:local`. Dates are UTC; history for currently open
connections is searchable. Selecting a result opens a new draft without running
it. History is plaintext local JSON; pause recording or clear it when SQL contains
private business data. Encrypted workspace recovery is a separate store.

Run a selection or the current statement, use parameters, and inspect each
statement's result. Native PostgreSQL/MySQL/SQLite/CockroachDB and SQL Server
query tabs own independent sessions. An interrupted transaction is not replayed.

## Edit safely and resume work

Stage inserts, edits and deletes; inspect their combined SQL before committing.
Primary keys and original values detect concurrent changes. Protected profiles
block writes; provider permissions remain the final boundary. Encrypted staged
changes recover after reopening with matching database and column metadata.
Review recovered drafts explicitly; they never auto-apply. Query results and
open transactions are not restored. If the recovery budget is exceeded, save
large work separately before exiting.

## Change structure

Open Structure and the designer. Select existing columns to populate properties,
queue alterations and **Review SQL plan** in a new query tab. PostgreSQL/MySQL
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

Use the database menu's native backup actions with installed `pg_dump` /
`pg_restore` or `mysqldump` / `mysql`. PostgreSQL uses a custom archive; MySQL uses
SQL. The job detects tools, reports progress/logs, supports cancellation and
keeps previous backups intact on failure. Confirm the exact restore database
and use a new empty target. PostgreSQL restore is transactional; MySQL DDL can
already be committed if cancellation or an error occurs. A backup can contain
executable database functions and triggers: restore only trusted input. Use
**View native job log** for diagnostics; credentials are delivered privately.
PostgreSQL's native tools preserve TLS identity over TCP SSH forwarding. Native
MySQL clients skip TLS on Unix sockets, so a required/verified TLS native job
needs a direct TCP profile; DBX query sessions still verify TLS over SSH.

## Local agents

Use the database menu to share one database read-only through MCP, copy its
private configuration and inspect activity. Stop sharing to revoke it. See
[local MCP](local-mcp.md); today's draft-only query assistant stays available.
