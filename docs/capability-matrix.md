# Connector workflow capabilities

This matrix describes implementation scope. Local disposable fixtures and CI
prove individual checks, not authenticated paid-provider accounts or physical
Mac behavior. Protected profiles disable every write workflow. Account
permissions remain authoritative, including function side effects.

| Connector | Grid changes | Independent query transactions | Full-query stream export / previewed atomic import | Native backup | Notes |
| --- | --- | --- | --- | --- | --- |
| PostgreSQL, CockroachDB | Atomic checked changesets | Yes | Yes / yes | PostgreSQL only | Strict TLS over SSH uses Unix forwarding with original TLS identity. AWS IAM / Azure PostgreSQL tokens refresh automatically through installed CLIs. |
| MySQL | Atomic checked changesets on InnoDB | Yes | Yes / yes on InnoDB | Yes | DDL can commit independently; native SQL restore is not atomic. Native verified-TLS jobs require direct TCP because the MySQL CLI skips TLS on sockets. |
| SQLite | Atomic checked changesets | Yes | Yes / yes | File/database export | Schema alterations beyond SQLite syntax require a reviewed rebuild. |
| SQL Server | Atomic checked changesets | Yes, isolated TDS clients | Loaded-result export / no wizard import | No | Password authentication; Entra/integrated authentication is not implemented. |
| DuckDB | Checked edits, partial-success reporting | No tab-owned transaction contract | Loaded-result export / no wizard import | No | File/in-memory analytics. |
| Turso, Cloudflare D1 | Checked edits, partial-success reporting | No persistent provider transaction | Loaded-result export / no wizard import | No | API tokens; SQL editing uses provider APIs. |
| BigQuery, ClickHouse, Snowflake | Read-only grid | No tab-owned transaction contract | Loaded-result export / no wizard import | No | SQL consoles retain provider-specific commands; ADC for BigQuery, explicit Snowflake token/JWT modes. |
| MongoDB, Elasticsearch | Read-only document grid | No | Loaded-result export / no wizard import | No | Native JSON/HTTP commands; no relational structured-filter editor. |
| Redis, Kafka | Provider-specific console | No SQL transaction contract | Loaded-result export / no wizard import | No | Redis incremental key browsing; Kafka metadata and bounded record inspection. |

Relational table/database exports use bounded pages; native SQL engines obtain
consistent snapshots where available. MCP pairing and bounded cross-connection
copy currently support PostgreSQL, MySQL, SQLite and CockroachDB. The designer
creates SQL drafts and never automatically executes them.

Linux x86_64 and Apple Silicon macOS are the published platforms. Intel macOS
build/test and signed-package jobs are added to the release matrix, with a matching
updater asset selector; a published Intel release and physical Mac acceptance
still require release evidence. Windows is tracked separately: dependencies,
packaging, signing, updater validation and physical QA are not established.
