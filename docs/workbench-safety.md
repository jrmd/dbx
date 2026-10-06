# Workbench safety and limits

## Queries and transactions

PostgreSQL, MySQL, SQLite and CockroachDB query tabs retain their own SQL
connection. Temporary tables and session settings therefore survive later runs
in the same tab. Begin, Commit and Rollback controls apply to that tab. Closing
it, disconnecting, or locking the vault discards unfinished native transactions.
SQLite `:memory:` uses its sole shared connection; other operations may wait
while a tab holds a transaction. Other connectors reject interactive transaction
commands because DBX cannot promise a persistent provider session.

Run all returns a separate selectable result for each statement, including empty
results and errors, and stops at the first error. Native SQL results obey the
row limit and a 64 MiB retained-data bound, also enforced across a script.
Reaching a limit closes the connection, rolls back an open transaction and skips
remaining statements. Earlier statements already committed remain committed.
Provider implementations have their own response bounds; the legacy pooled core
query API is separate from this bounded interactive console.

The query options menu offers deadlines of 5, 30, 60 or 300 seconds. PostgreSQL
uses `pg_cancel_backend` and MySQL uses `KILL QUERY` through a separate control
connection. Status distinguishes server acknowledgment from unknown server
outcome. Acknowledgment means the cancellation request was accepted; it does
not undo writes committed before cancellation. SQLite checks cancellation and
deadlines through its execution progress handler. Other connectors stop waiting
locally and report that server execution may continue.

## Row edits and protected profiles

Double-click a table cell to edit it. Enter or leaving the cell stages the
typed value; Escape cancels the current editor. Staged cells are highlighted.
Save changes applies checked updates, and Discard removes pending changes.
Sorting, paging, refreshing, closing a table/connection and switching databases
are blocked while cell edits remain. Vault locking asks before discarding edits.
PostgreSQL, MySQL/InnoDB, SQLite and CockroachDB batches use a transaction;
other writable connectors apply rows sequentially and report partial success.

Query tabs prompt for `:name` and `$1` placeholders outside strings/comments.
Values are bound separately through the driver/API: text, numbers, booleans,
null and JSON are supported. Quotes are literal text, not SQL quoting syntax.
Confirmations retain the exact statement and bound-value snapshot. Find and
replace works in the editor; Results mode finds matching cells in displayed order.
Results can be saved as CSV, TSV, JSON or INSERT statements with a chosen target table.

The encrypted workspace recovers query, data, structure and diagram tabs in
their saved order after reconnecting, retaining the selected tab. Missing tables
are skipped. After vault unlock, recovery reopens saved connections and their selected databases.
Each connection instance retains its own tab layout. Recovery never executes saved queries.
Pending cell edits, query results and transactions are not persisted.

Grid updates and deletes match the primary key plus the original displayed
values. A changed or deleted row reports a conflict rather than overwriting the
newer values. Exactly one affected row is required. Nullable primary-key values
cannot identify a row safely and are rejected.

Protected profiles disable grid writes and imports and conservatively restrict
console commands. PostgreSQL/MySQL/SQLite also set their session read-only
mode. Provider guards reject known writing commands and stages. These are
accidental-write safeguards: database permissions remain the authority,
particularly for stored functions with side effects. Use a database account
with read-only privileges when that boundary matters.

## File transfers

Exports stream bounded pages to a temporary file, optionally through gzip, then
flush and replace the destination only after success. A failed or cancelled file
preserves its previous destination. A database CSV/TSV export completes each
table file independently; already completed files remain if a later file fails.

PostgreSQL, CockroachDB, SQLite and MySQL/InnoDB exports read all data pages and
selected tables in one transaction snapshot. Rows use stable key ordering, or
all-column ordering when there is no key. MySQL non-InnoDB tables and other
connectors are explicitly reported as live reads. Schema metadata is gathered
before the data snapshot; concurrent DDL is not a full schema-and-data backup
guarantee. SQL dumps include the columns, defaults, primary keys, check
constraints, secondary indexes and foreign keys represented by DBX metadata.
PostgreSQL serial columns are recreated as serial types and their sequences are
advanced past the imported rows. Available standalone sequences, routines,
triggers and view definitions are also dumped. SQLite CHECK expressions are
captured from its CREATE statement. MySQL 8 functional index expressions are captured and rendered. Generated
column expressions and grants require native tools. Use native backup tools for
full-fidelity backups.

File imports stream records/statements and insert bounded batches inside one
transaction on PostgreSQL, CockroachDB, SQLite and MySQL/InnoDB. Parse failures,
database errors and cancellation roll back earlier batches. CSV/TSV headers map
to real columns with typed parameter binding. An unquoted empty field is NULL;
`""` is empty text; binary values use hex. Records/statements are limited to
64 MiB and batches to 500 rows or 4 MiB, further reduced by parameter limits.

MySQL SQL imports accept data statements and reject DDL, transaction/session
commands and executable version comments because these can bypass atomicity.
Imports check the active database's storage engines and reject non-InnoDB
targets. Keep imported SQL within that database and transactional tables;
cross-database SQL and side effects of triggers/functions require separate
review. A generated MySQL schema dump must be reviewed/applied separately from
its data import. Other connectors reject atomic file imports instead of
replaying them without a transaction. Transfer progress and Cancel appear in
the status bar.

## Saved work and diagnostics

Query names, unfinished SQL drafts, saved queries and schema baselines live in
the encrypted vault, keyed to the connection identity. Editor changes autosave
after a short debounce; closing or locking flushes current drafts. Documents are
bounded to 2 MiB and writes are revision ordered. Saved query names are unique
within a connection; saving the same name replaces its SQL. A save toast appears
only after persistence succeeds. Normal query history retains its existing
local storage contract.

The query options menu can Explain a single read statement, pin its plan and
compare with a later plan. PostgreSQL and MySQL JSON plans become a hierarchy
grid with estimated rows/costs; other supported engines retain their native
plan shape. Explain never adds ANALYZE. Comparisons align displayed steps and
are aids for inspection, not semantic proof of equivalent plans.

PostgreSQL/MySQL session and lock monitors open provider queries in new tabs.
Visibility depends on the account's server permissions. References:
[PostgreSQL locks](https://www.postgresql.org/docs/current/view-pg-locks.html),
[MySQL lock waits](https://dev.mysql.com/doc/refman/8.4/en/performance-schema-data-lock-waits-table.html),
[PostgreSQL EXPLAIN](https://www.postgresql.org/docs/current/sql-explain.html),
and [SQLite query plans](https://www.sqlite.org/eqp.html).

Capture schema baseline stores normalized columns, defaults, primary keys,
check constraints, indexes and foreign keys. Compare schema opens a migration
draft in a new editor tab without running it. New tables, nullable or defaulted
column additions, default changes, index additions/removals and check
constraint additions/removals can be generated. Column drops remain commented;
changed types, required/key columns and changed foreign keys require manual
review. Baselines captured by earlier DBX versions lack defaults, indexes and
checks, so those are not compared until the baseline is recaptured. When both
snapshots capture ancillary objects, changes to views, triggers, routines and
sequences appear as commented review drafts. Generated columns and grants
remain outside automatic migration generation. This is a conservative drafting workflow, not a
complete schema synchronization or migration-history system.

Catalog read failures abort schema capture, structure loading and export rather
than treating unavailable indexes or constraints as absent. Unsupported server
catalog versions therefore produce an error requiring review.

## BigQuery credentials

Leave the optional OAuth token field blank to use Google Application Default
Credentials. For local user credentials, run `gcloud auth application-default
login` before launching DBX. For a service account, set
`GOOGLE_APPLICATION_CREDENTIALS` to its credential JSON file in DBX's launch
environment. The authentication library caches and refreshes tokens; each API
request obtains the current token. An explicitly supplied access token is still
supported but must be renewed manually. Credentials must have the required
BigQuery permissions and the cloud-platform scope. See
[gcp_auth credential discovery](https://docs.rs/gcp_auth/latest/gcp_auth/fn.provider.html)
and [Google service-account credentials](https://docs.cloud.google.com/iam/docs/service-account-creds).

## Verification

Core and native GPUI tests exercise conflict preservation, session transactions,
savepoints, separate results, cancellation, result limits, encrypted recovery,
streamed transfers, late-batch rollback and snapshot paging during concurrent
writes. Local HTTP tests cover refreshed-token requests; they do not establish
authenticated BigQuery or service-account access.

Disposable PostgreSQL 16 and MySQL 8.4 tests cover native transactions,
transaction chaining, protection, plans, cancellation acknowledgment, exports
and import rollback. Existing connector regressions also run against disposable
Redis 7 and ClickHouse 26.8. Live D1 and Turso checks passed on October 6, 2026, covering bound values,
defaults, unique indexes, checked updates, stale conflicts and persisted readback.
ClickHouse HTTPS certificate/authentication and Snowflake polling, bindings and
partitioning have local protocol coverage. Paid ClickHouse Cloud, BigQuery,
Snowflake, RDS IAM and Azure Entra accounts were not provisioned; local tests
do not establish account permissions or provider authentication. Physical macOS
behavior and hosted CI remain separate verification boundaries.

```sh
env CARGO_BUILD_JOBS=4 CXXFLAGS=-g0 cargo test --locked -p dbx-core
env CARGO_BUILD_JOBS=4 CXXFLAGS=-g0 cargo test --locked -p dbx-ui
env CARGO_BUILD_JOBS=4 CXXFLAGS=-g0 cargo clippy --locked --workspace --all-targets -- -D warnings

docker compose -f docker-compose.test.yml -p dbx-workbench-audit up -d --wait postgres mysql
DBX_TEST_POSTGRES_URL=postgres://dbx_test:dbx_test_password@127.0.0.1:55432/dbx_test \
DBX_TEST_MYSQL_URL=mysql://dbx_test:dbx_test_password@127.0.0.1:53306/dbx_test \
  cargo test --locked -p dbx-core --test workbench_safety -- --ignored --test-threads=1
docker compose -f docker-compose.test.yml -p dbx-workbench-audit down --remove-orphans
```

## Table designer and value viewers

Structure → Design table generates quoted, reviewable ALTER or index SQL in a
query tab. Creating a draft never executes it. JSON cells open a multiline
syntax-highlighted editor; Stage validates JSON before marking the cell pending.
Query cells open a dedicated pretty JSON/text, hex or image viewer on double-click.
Previews are bounded; Copy full value preserves the complete value.

## Connection recovery and cloud tokens

Native SQL sessions health-check idle connections and reconnect before running a
statement. Losing an open transaction fails it rather than replaying it. SQL Server
and Redis probe connections before commands. HTTP connectors issue independent
requests; MongoDB and Kafka retain their driver reconnection behavior. SSH
supervision re-establishes the same local forwarding port after process exit.
No submitted database write is automatically replayed.

AWS RDS IAM uses the existing `aws` CLI login, default profile and region. Azure
PostgreSQL Entra uses the existing `az` login and the `oss-rdbms` audience. Token
buttons populate the masked password field; fetch a new token when it expires.
See [AWS token command](https://docs.aws.amazon.com/cli/latest/reference/rds/generate-db-auth-token.html)
and [Azure PostgreSQL Entra authentication](https://learn.microsoft.com/en-us/azure/postgresql/security/security-entra-configure).
