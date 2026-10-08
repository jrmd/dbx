# DBX product gap audit — 8 October 2026

## Verdict

DBX has a credible everyday database workbench foundation. Its biggest remaining gaps are workflow completeness, consistent connector behavior, and evidence of dependable desktop operation. Adding more connector names would have less value than closing those gaps.

The competition has moved beyond connecting, querying, and editing rows. TablePlus provides integrated native backup/restore and substantial schema tools. TablePro advertises staged data and structure edits, rich file workflows, comparison/copy tools, cloud authentication, and a local MCP server. TablePlus also documents [AI](https://tableplus.com/docs/llm-plugin) and [MCP](https://tableplus.com/docs/preferences/mcp). See the [first-party competitor research](competitor-research-2026-10-08.md) for dated sources and commercial/platform qualifications. These are advertised competitor capabilities, not hands-on competitor test results.

“Native,” “open source,” and “bring your own AI” are useful attributes but do not independently distinguish DBX from TablePro, whose [FAQ](https://tablepro.app/faq) and [pricing](https://tablepro.app/pricing) describe a native, open-source client with a free core. The most credible positioning to develop is a dependable, free developer workbench for Linux and macOS, with explicit write safety and portable local work. Validate that positioning with switching users before expanding scope.

## Scope and evidence

- Checkout: `2d8a241`, version `0.5.0`. The worktree was clean at the start. Its parent, `87109a53f67ae62c8e2dc32e973dbfc1ad5db224`, contains the released application code; HEAD changes the website. No application, dependency, script, or workflow differences exist between those two commits.
- Inspected current source through CodeGraph, followed by targeted reads for uncovered details; read current product, connector, safety, release, and QA documentation.
- Inspected the live [DBX website](https://dbx.jrmd.dev/) in the collaborative browser. It serves v0.5.0 Mac and Linux asset links and identifies DBX as a preview.
- Verified [hosted Linux and macOS build checks](https://github.com/jrmd/dbx/actions/runs/37760517184): **414 passed tests per platform**, with 22 Linux / 23 macOS ignored checks. The workflow also passed formatting, strict release Clippy, packaging-script checks, and the release build.
- Verified [hosted database integration](https://github.com/jrmd/dbx/actions/runs/37765187554): **15 passed checks**, spanning native CRUD, PostgreSQL/MySQL workbench safety, and six server connector checks. This is disposable-server evidence, not paid cloud-account evidence.
- Verified [v0.5.0 release](https://github.com/jrmd/dbx/actions/runs/37760563415), including successful signed Mac packaging, signed updater installation checks, and publication.
- Reproduced the dotted-identifier failure below using the current Rust quoting functions in an isolated harness and a disposable SQLite database.
- A fresh local development-suite run was started, then stopped during DuckDB compilation once current hosted evidence for the identical application code was retrieved. No new local workspace-suite pass is claimed.
- No hands-on native desktop session, physical Mac accessibility test, competitor application trial, or paid provider account test was performed in this audit. Visual polish, accessibility, latency, and provider onboarding remain validation questions rather than established defects.

The October 6 audit documented real implementation work, but its “hosted CI unverified” boundary is now stale. Keep completed features and current external verification boundaries separate.

## Existing strengths: do not rebuild these

Current code already supplies:

- Multiple connections and independent data/query/structure/diagram documents; encrypted query drafts and workspace recovery.
- Staged **inserts, updates, and deletes**, changeset SQL review, bulk actions, row duplication, TSV paste, primary-key and original-value conflict checks.
- Server sorting, bounded pages, single-primary-key keyset browsing, estimated/on-demand exact counts, resizable/hidden/pinned columns, persisted table layouts, saved filters, and fuzzy quick open.
- Schema-aware SQL completion including alias/CTE inference, current-statement/selection execution, separate statement results, parameter binding, find/replace, named queries, query history, deadlines, and native SQL transaction sessions.
- JSON/binary viewers, FK navigation, ER diagrams with arrangement and SVG/PNG export, schema baselines, conservative migration drafts, and a limited table designer.
- Streaming table/database transfers, native snapshot exports and atomic imports on supported engines, protected profiles, SSH passwords/jump hosts/reconnect, BigQuery ADC, and AWS/Azure CLI token helpers.
- A local CLI query assistant, MIT licensing, Mac notarization/updater checks, and Linux AppImage distribution.

Evidence: [workbench contracts](workbench-safety.md), [connector contracts](database-connectors.md), `app/cell_edits.rs`, `app/data_clipboard.rs`, `app/table_layout.rs`, `app/quick_open.rs`, `app/row_count.rs`, `app/sql_completion.rs`, and the hosted checks above. Code presence does not establish equivalent depth across all fifteen connectors.

## Prioritized gaps

Priority is based on daily developer use and switching from TablePlus/TablePro. P1 should precede a broad production-ready claim; P2 follows a dependable core; P3 requires demonstrated demand. Effort is relative scope, not a delivery estimate.

| ID | Priority / effort | Gap and customer consequence | Current evidence | Recommended completion criterion |
| --- | --- | --- | --- | --- |
| G01 | P1 / small | **Valid names containing dots fail.** An existing table named `invoices.v2` or column `amount.net` cannot be addressed correctly by generated SQL. | `dbx-core/src/sql.rs:27–98`: `quote_identifier` splits every dotted name; `quote_table` concatenates schema and table before calling it. Isolated reproduction below. | Quote each already-separated schema/table/column component literally. Preserve a separate qualified-name API where needed. Regressions cover dots, quotes, spaces, Unicode, reserved words, and actual table reads/writes. |
| G02 | P1 / small–medium | **Query-result exports have weaker safety than table exports.** They serialize the loaded grid and directly overwrite a destination file. A failed write can damage an existing file; a query returning over 10,000 rows exports only its bounded loaded result. | `dbx-ui/src/app/query_actions.rs:449–476,548–609`; `QueryOptions::default()` in `dbx-core/src/engine.rs:22–25`. There is no export-specific truncation check in that path. Table transfers use temporary-file replacement. | First add atomic destination replacement and an explicit “export loaded rows”/truncation notice. Then add a separate cancellable streaming full-query export with a defined consistency contract and bounded memory. Do not blindly rerun arbitrary SQL with side effects. |
| G03 | P1 / small–medium | **Saved connection management is incomplete.** Users can accumulate profiles but have no exposed delete action; duplication, profile portability, and competitor import are missing from the inspected UI. This is a switching barrier. | `ProfileStore::delete` in `dbx-ui/src/profiles.rs:653–684` exists, is marked for future UI, and has no application caller. Quick open opens profiles but is not management. No profile import/export workflow was found. | Add rename/duplicate/delete with clear credential handling; import compatible TablePlus/TablePro exports where their formats permit; offer password-free connection export by default and an explicit encrypted portable bundle. Verify tags, SSH settings, TLS options, credentials, and open-session behavior. |
| G04 | P1 / medium–large | **No integrated full-fidelity backup and restore workflow.** DBX’s portable SQL export is not a native database backup; MySQL atomic SQL import deliberately rejects DDL. Users still leave the app for routine recovery. | [Transfer limits](workbench-safety.md#file-transfers): generated expressions/grants require native tools, metadata precedes the data snapshot; `DumpFormat` is SQL/CSV/TSV. No `pg_dump`/`pg_restore`/`mysqldump` execution workflow was found. TablePlus documents [integrated backup/restore](https://tableplus.com/docs/gui-tools/backup-and-restore). | Wrap native PostgreSQL/MySQL tools with version detection, progress/logs, cancellation, safe credential delivery, restore-target confirmation, and a tested backup-to-new-database round trip. Keep “export” and “backup” distinct. Account for SSH and native dump-format compatibility. |
| G05 | P1 / medium–large | **The visual schema designer is shallow.** It adds/renames/drops columns and adds/drops indexes, but cannot visually edit an existing column’s type/default/nullability or author FKs/CHECKs/PK changes. Metadata viewing is richer than editing. | `TableAlteration` in `dbx-core/src/designer.rs:6–28`; UI action list in `dbx-ui/src/app/designer.rs:123–147`. Schema comparison generates a conservative baseline draft, not a complete schema editor or synchronization tool. | Start with PostgreSQL/MySQL: edit existing column properties, constraint/FK creation, metadata-backed selectors and type suggestions, review the combined SQL, and refresh all affected tabs after execution. Unsupported SQLite alterations should use a reviewed rebuild or a clear limitation. |
| G06 | P1 / medium–large | **Connector depth varies substantially.** SQL Server tabs share one connection and staged writes can partially succeed; MongoDB/Elasticsearch grids are read-only; some engines cannot import or maintain interactive transactions. “Supported” needs to convey those distinctions before users hit an error. | [Connector contracts](database-connectors.md#scope-and-verification); `DatabaseEngine::query_table_with_columns` rejects structured non-SQL filters. `Engine` has no comprehensive capability descriptor. Some differences reflect database semantics and should remain. | Show connection/table-specific capabilities in the product. Separate intentional read-only analytics behavior from missing implementation. Give SQL Server independent sessions and atomic supported batches; add document-aware MongoDB editing/filtering if demand justifies it. Never claim transaction semantics for stateless APIs. |
| G07 | P1 / medium | **Strict TLS hostname verification cannot be combined with SSH forwarding.** Secure remote setups using `verify-full` / `verify_identity` are refused. Cloud tokens also need manually refreshing for new connections. | [README transport limitation](../README.md#unix-sockets-and-ssh-tunnels); [cloud token behavior](workbench-safety.md#connection-recovery-and-cloud-tokens). SQL Server has no Entra/integrated-auth helper; Snowflake takes user-minted tokens/JWTs. | Preserve the original TLS server identity independently of the local tunnel endpoint and test wrong-host/certificate rejection. Add token acquisition/refresh on connect/reconnect for the supported cloud flows, without replaying submitted writes. Rank broader auth methods by actual customer environments. |
| G08 | P1 / medium | **Staged data work is lost on process failure/restart.** Query drafts recover, but pending cell edits/inserts/deletes do not. A user can lose a substantial uncommitted changeset. | [Workspace recovery contract](workbench-safety.md#row-edits-and-protected-profiles), explicitly states pending edits/results/transactions are not persisted. Navigation protection already exists. | Encrypt and recover draft changesets with profile/database/table identity, original values, and schema fingerprint. On reopening, require review and fresh conflict validation. Never execute recovered mutations or restore transaction state automatically. |
| G09 | P1 validation / medium | **Desktop quality and large-schema performance are not yet established by reproducible acceptance evidence.** GPU rendering, virtualized pages, and keyset support are mechanisms, not comparative benchmarks. Composite keys and non-key sorts retain OFFSET paging. | `app.rs:245–282`; [desktop QA](desktop-qa.md); no benchmark harness/report found in the inspected repo. Hosted tests demonstrate correctness gates, not physical input/IME/screen-reader behavior. | Publish measurements on fixed Mac/Linux hardware for cold start, large schema discovery, deep sorted paging, wide/large-value rows, concurrent tabs, export memory, and reconnect latency. Run keyboard-only and screen-reader checks, IME, scaling, light/dark, and app upgrade/recovery journeys. Use absolute acceptance budgets before claiming an advantage over competitors. |
| G10 | P2 / small–medium | **Query history is hard to retrieve and privacy controls are limited.** The query menu displays ten recent entries; the store retains 100 per connection. No searchable history UI or opt-out setting was found. History is local JSON, distinct from the encrypted workspace. | `app/view/query.rs:361`; `query_history.rs:26–34`; a clear-history action already exists. Credential-shaped SQL is skipped, but that does not encrypt arbitrary business literals or personal data in other queries. | Add history search, date/outcome/connection filters, open-without-run, configurable retention and per-profile recording control. Either encrypt history or explain the storage distinction clearly. Preserve existing clear-history and secret rejection. |
| G11 | P2 / medium | **Data interchange is narrow.** File import is SQL/CSV/TSV; JSON/JSONL/XLSX import, a mapping/preview wizard, direct cross-connection copy, and data diff are not available in the inspected workflow. | `transfer.rs` formats and connector import limits; [TablePro features](https://tablepro.app/features) advertise broader file formats and database copy/compare. Query-result JSON export already exists and must not be counted as missing. | Start with previewed CSV/JSON mapping, explicit NULL/date/binary semantics, error-row handling and rollback guarantees. Add copy between supported relational connections with type-conversion warnings. Validate demand before a full cross-engine sync system or spreadsheet file viewer. |
| G12 | P2 / medium–large | **No local DBX MCP server or external automation surface.** The CLI assistant drafts queries but does not expose saved DBX connections to external agents. Both competitors advertise MCP. | [DBX assistant contract](query-agents.md) gives the CLI no execution tools; [TablePro AI/MCP](https://tablepro.app/features/ai-mcp) and [TablePlus MCP](https://tableplus.com/docs/preferences/mcp) document servers. MCP-related strings in DBX restrict third-party CLI permissions; they are not a DBX server. | Add explicit local pairing, scoped connection access, metadata/read tools first, bounded results, cancellation, and an activity log. Treat write tools as a separate reviewed capability. Preserve today’s draft-only CLI workflow. |
| G13 | P2 / small–medium | **Switching/onboarding and public documentation undersell working features and blur limits.** The live site emphasizes native/GPU/free and shows basic workflows, while staged bulk changes, quick open, saved filters, recovery and designer tools are largely absent. It links Architecture rather than a task-oriented user guide. | Live website inspection; README and docs are available in GitHub. `docs/macos-release.md:86` incorrectly says queries are not saved/restored; portions of architecture still describe keyset paging as future work. October 6 verification notes predate successful hosted runs. | Add a “switch from TablePlus/TablePro” guide, demo/sample connection, task-based docs, concise capability matrix, release changelog, and current screenshots/workflow clips. Correct stale docs. Make the Linux/Mac use case and production-preview boundary explicit. |
| G14 | P3 / large | **Broader administration/platform coverage is incomplete.** No dedicated users/roles/grants UI was found; distribution is Apple Silicon Mac and Linux x86_64, with no Windows or Intel Mac packages. | Current module inventory, [README release platforms](../README.md#unix-sockets-and-ssh-tunnels), and release workflow matrix. TablePlus has broader OS reach; TablePro’s desktop focus is Mac. | Add only when target users require it. Track Windows demand, role/grant workflows, and any missing engine such as Oracle separately. Avoid a mobile client or connector-count race before daily desktop workflows are dependable. |

## Concrete defect reproduction

The reproduction extracts the actual `quote_identifier` and `quote_table` functions from current `crates/dbx-core/src/sql.rs`, compiles them in an isolated Rust harness with stand-in surrounding types, then runs the emitted statements against Python’s SQLite connection. It does not launch the full DBX UI or compile the application.

Given an existing SQLite table and column:

```sql
CREATE TABLE "invoices.v2" ("amount.net" INTEGER);
INSERT INTO "invoices.v2" VALUES (42);
```

The current functions emit:

```text
SELECT * FROM "invoices"."v2"
=> no such table: invoices.v2

SELECT "amount"."net" FROM "invoices.v2"
=> no such column: amount.net

SELECT "amount.net" FROM "invoices.v2"
=> [(42,)]
```

This is a quoting correctness defect, not an SQL injection claim. Other dialects share the component-splitting design, but their live reproduction was not performed here. Fixing it requires auditing qualified-name callers rather than globally removing all qualification support.

G02’s direct destination write is source-confirmed; a disk-full or interrupted-write fault was not injected during this audit. The 10,000-row result cap is an intentional guard. The missing capability is explicit partial-export handling and a separate full-result streaming workflow, not removal of that guard.

## Recommended build order

1. **Dependable daily use:** G01, G02 atomic/partial export handling, G03 connection management, G10 history retrieval, and G13 stale docs. These have visible value without introducing a new subsystem.
2. **Complete PostgreSQL/MySQL workflows:** native backup/restore, richer schema designer, strict TLS through SSH, and recovered draft changesets. Prove errors/cancellation/restart behavior alongside happy paths.
3. **Make support claims precise:** capability-driven UI, SQL Server session/atomicity improvements, real cloud-account onboarding, and physical desktop/performance evidence. Existing provider read-only guards should remain until safe mutations exist.
4. **Lower switching costs and extend workflows:** competitor import, JSON mapping and cross-connection copy, then read-first MCP integration.
5. **Expand only with demand:** data synchronization, role management, Windows/Intel Mac, more engines, and mobile.

Use switching sessions to decide sequencing within those groups: ask five regular TablePlus/TablePro users to connect an existing non-production database, find a table, edit several rows, run a parameterized query, alter a column, export a large query, and restore a backup. Record completion, time, confusing states, and every occasion they return to their old client. This is a proposed study, not customer evidence gathered in this audit.

## What this audit does not establish

No application code was changed. Competitor capabilities are based on official documentation, and DBX capability findings are source-backed except where explicitly reproduced or hosted-verified. This report does not assert full parity, a measured speed advantage, accessibility conformance, or paid cloud/provider authentication. Those need their own acceptance evidence.
