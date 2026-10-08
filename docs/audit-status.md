# DBX audit status

The October 8 competitor gap work is recorded in [product gap audit](product-gap-audit-2026-10-08.md) and [gap fix verification](gap-fix-verification-2026-10-08.md). The October 6 evidence below is historical; its hosted CI boundary was subsequently verified for v0.5.0, as recorded in the newer audit.

## 6 October 2026 implementation

The interrupted implementation has been completed and checked locally. This
matrix records the feature scope and the remaining external verification
boundaries; it is not a claim of authenticated access to every cloud provider.

| Audit item | Implemented behavior | Evidence |
| --- | --- | --- |
| Header sorting | Ascending, descending, none; table sorting runs on the server and resets paging; query results sort locally | Native GPUI tests and connector integration |
| Inline edits | Double-click, typed staged values, highlights, save/discard, original-value conflict checks; navigation guards retain pending work | UI persistence and conflict tests; live native databases, D1 and Turso |
| Schema metadata | Defaults, indexes including MySQL functional expressions, unique/primary keys, CHECK constraints, view definitions; supported triggers, routines and standalone sequences in the explorer | Native catalog tests; PostgreSQL and SQLite dump/restore with working triggers/views; MySQL functional index recreation |
| Schema compare | Available detailed metadata is compared; ancillary object/view changes become commented review drafts | Core migration tests and encrypted snapshot tests |
| Table designer | Add/rename/drop columns and add/drop indexes generate a quoted SQL draft; engine-specific unsupported operations report a clear error | Core dialect tests; GPUI test proves drafting leaves the database unchanged |
| Parameters | `:name` and `$1` prompt for typed values, bound separately through the driver/API; confirmations retain the exact statement/value snapshot | Injection-shaped value tests; native session transaction test; real D1/Turso bindings |
| Find/replace | Editor find/replace and result-cell search in displayed order | Editor and result grid tests; query result replacement is intentionally unavailable because arbitrary query rows lack a safe mutation target |
| Restart recovery | Vault-encrypted saved connections, selected databases, per-instance ordered query/data/structure/diagram tabs and selected tabs | Vault relaunch, mixed-layout recovery, duplicate-profile layout tests; recovery never executes saved SQL |
| Result export | CSV, TSV, JSON files and clipboard; INSERT statements with an explicit target table | Typed export tests, native query-table exports and quoted INSERT generation |
| Value viewers | Pretty JSON and validated multiline JSON editing; complete-value copy, binary hex and bounded image previews in table/query inspectors | GPUI large-document persistence/invalid-JSON tests and value-preview tests |
| Reconnect | Native SQL session/pooled connection health checks; Redis and SQL Server probes; SSH supervisor reopens its original port | Lost native connection tests; real SSH process-kill/reconnect/cleanup test; submitted writes are never replayed |
| Cloud authentication | AWS RDS IAM and Azure PostgreSQL Entra token buttons use existing authenticated local CLIs | Command-construction tests; token issuance needs a provider account; fetch again on expiry |
| SSH passwords and jump hosts | Masked password, encrypted profile persistence, private askpass file; validated jump-hop configuration | Real disposable password-authenticated SSH; vault relaunch and transport tests |
| SQL Server | TDS connector, TLS options, metadata, bindings, database switching, sorted/filtered browsing and checked row changes | Disposable SQL Server 2022 connector suite |
| Snowflake | SQL API connector with PAT/OAuth/JWT inputs, typed bindings, polling, partitions, limits, cancellation requests, database/table/view metadata | Local protocol fixture; authenticated Snowflake account not provisioned |
| PR and live CI | Build checks on PRs; database integration on relevant PRs, main, nightly and manual dispatch | Workflow configuration plus local execution of the same integration script; hosted runs are not claimed |
| Module size | Session state, tabs and query actions extracted from app; syntax/formatting/completion analysis extracted from editor | Workspace tests and Clippy |
| Stale files and product docs | Stale local files moved to ignored `target/local-backups/20261006-214857`; PRODUCT and connector/safety docs now describe all 15 engines | Local file and documentation checks |

## Verification

- Both final development and optimized (`--release`) workspace suites passed
  406 tests each (including the PostgreSQL password regressions). The default suite explicitly ignores 22 live database,
  provider and platform checks; the applicable local and cloud checks were also
  run separately below. Formatting, diff checks and Clippy with warnings denied
  passed. These local results do not claim hosted CI or physical macOS proof.
- The disposable local database suite has 15 passing checks across PostgreSQL,
  MySQL, SQLite, Redis, ClickHouse, CockroachDB, MongoDB, Elasticsearch, Kafka,
  and SQL Server. PostgreSQL checks include restoring nested view dependencies,
  functions, triggers, a standalone sequence, serial identity, indexes and checks.
- Local socket/SSH checks cover nine forwarding combinations, password
  authentication, reconnect after killing the SSH client, and listener cleanup.
- Live D1 and Turso both passed bound-value, default/index, checked-update,
  stale-conflict and persisted-readback contracts. Their audit tables are removed
  by the tests. D1 uses the isolated `dbx-audit-20261006` database.
- ClickHouse HTTPS has local certificate-validation and authentication proof.
  BigQuery has local API/token/pagination fixtures; Snowflake has local
  authentication/binding/polling/partition fixtures. These do not prove provider
  account permissions.
- Paid ClickHouse Cloud, BigQuery, Snowflake, AWS RDS and Azure databases were
  not provisioned, following the cost constraint. Actual IAM/Entra token
  issuance, those paid live accounts, physical macOS testing and hosted CI
  remain external verification boundaries.

## PostgreSQL connection-string regression

Saved profile URLs omit passwords by design, but connection-string mode now
combines them with the credential restored from the vault. Switching to Details
preserves that credential and URL options. A newly pasted database URL password
takes precedence over a stale hidden Details password when connecting or saving.
Regression checks cover the UI handoff, encrypted save/load, reserved characters
and the password actually received by the PostgreSQL protocol fixture.

## Explicit engine limits

Catalog support follows engine capabilities and account permissions. Grants and
full generated-column expressions are not a portable backup contract. Native
backup tools remain appropriate for full-fidelity recovery. SQL Server and
ClickHouse currently export CSV/TSV rather than generic SQL dumps. MySQL atomic
imports accept data-only SQL because DDL implicitly commits. Interactive query
transactions remain native SQL-session features; Snowflake/BigQuery/ClickHouse
row grids are read-only where safe row identity is unavailable. Other writable
connector batches can partially succeed and report that outcome.

See [workbench safety](workbench-safety.md) and
[connector capabilities](database-connectors.md) for the precise contracts.
