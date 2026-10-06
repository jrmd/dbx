# Database connectors

DBX provides native connections for PostgreSQL, MySQL, SQLite, Redis, MongoDB,
CockroachDB, DuckDB, Elasticsearch, BigQuery, Kafka, Turso, Cloudflare D1 and ClickHouse.

| Provider | Connection | Query editor and explorer |
| --- | --- | --- |
| MongoDB | `mongodb://localhost:27017/app`, replica-set seed lists, or `mongodb+srv://user:password@cluster/app` | JSON database commands; collections and bounded document results |
| CockroachDB | PostgreSQL URL supplied by CockroachDB, including its TLS options | SQL, schemas, tables, structure, primary keys and foreign keys |
| DuckDB | `duckdb:///absolute/path.duckdb` or `duckdb::memory:`; native file chooser | Embedded DuckDB SQL, tables, views, structure and bound row mutations |
| Elasticsearch | HTTP(S) endpoint; username/password for Basic authentication or `https://:API_KEY@host` | `METHOD /path` followed by an optional JSON body; indices and search hits |
| BigQuery | `bigquery://project/dataset?location=US`; leave the token field blank for Google Application Default Credentials, or supply an OAuth access token | GoogleSQL, datasets, tables, typed results, job polling and result pagination |
| Kafka | `kafka://broker:9092`; optional `brokers=host:9092,other:9092` | Topics, JSON consume commands and acknowledged produce commands |
| Turso | `libsql://database-organization.turso.io`, `turso://...` or HTTPS, with a database token in the masked field | SQL over HTTP, tables, views, SQLite structure and bound row mutations |
| Cloudflare D1 | `d1://account-id/database-id`, with a Cloudflare API token in the masked field | REST SQL, tables, views, SQLite structure and bound row mutations |
| ClickHouse | `clickhouse://default@localhost:8123/default` for HTTP; select ClickHouse and use `https://user:password@host:8443/default` for TLS/Cloud | SQL, database switching, tables/views, typed structure, bound filters, sorting and paged results; read-only row grid |

API tokens can also be supplied as URL passwords by API callers. DBX stores
passwords and tokens in its encrypted vault, removes them from profile JSON,
and rejects credentials supplied in query parameters. HTTP authentication
requires HTTPS except on loopback endpoints used for local development.

Supabase uses the existing PostgreSQL connector. Copy its direct connection or
session-pooler URL from the Supabase dashboard. No Supabase-specific engine is
needed. See [Supabase's connection guide](https://supabase.com/docs/guides/database/connecting-to-postgres).

## Query examples

MongoDB commands are JSON, including extended JSON values where necessary:

```json
{"find":"users","filter":{"active":true},"limit":100}
```

```json
{"aggregate":"users","pipeline":[{"$match":{"active":true}}],"cursor":{}}
```

Elasticsearch requests stay on the configured endpoint:

```http
POST /users/_search
{"query":{"match_all":{}},"size":100}
```

Kafka queries:

```json
{"action":"topics"}
```

```json
{"action":"consume","topic":"events","partition":0,"offset":0,"limit":100}
```

```json
{"action":"produce","topic":"events","key":"example","value":"hello"}
```

Kafka browsing assigns partitions directly and disables automatic offset storage
and commits. `offset` is a broker offset in each selected partition. Binary
keys and payloads remain byte values. A produce result is returned after the
broker acknowledges delivery; a timeout reports unknown delivery status.

For SASL, put the username/password in the URL and specify `security.protocol`
and `sasl.mechanism` as needed. Credentials default to SASL_SSL and PLAIN.
Supported certificate-file options are `ssl.ca.location`,
`ssl.certificate.location` and `ssl.key.location`.

## Scope and verification

ClickHouse uses the [HTTP SQL interface](https://clickhouse.com/docs/interfaces/http),
not native TCP ports 9000/9440. Choose ClickHouse before entering a generic HTTPS
URL; generic HTTP URLs otherwise identify Elasticsearch. The database is the URL
path (or `?database=name`); other URL options are rejected. Use the database picker
to switch databases. SQL queries are stateless, so session commands such as `USE`
and `SET` do not affect later requests. Proxy path prefixes are not supported.

ClickHouse results use JSONCompact metadata and positional rows. Int64/UInt64
retain exact values; decimals and wider integers remain text, and arrays/tuples
remain JSON. Omit explicit output `FORMAT` clauses so DBX can decode the result.
Queries have row and 64 MiB response bounds. Row inserts, updates and deletes are
disabled because ClickHouse sorting/primary keys do not enforce uniqueness; use
SQL for writes. CSV/TSV exports are supported. SQL dumps and file imports are
unsupported because generic SQL dumps omit ClickHouse engine and sorting metadata.

Disposable ClickHouse 26.8 coverage checks authentication, SQL writes, table and
column metadata, exact UInt64/decimal values, bound equality/LIKE filters, empty
results, truncation, database switching and rejected row edits. HTTPS/ClickHouse
Cloud account verification remains outstanding. To rerun locally:

```sh
docker compose -f docker-compose.test.yml up -d --wait clickhouse
cargo test -p dbx-core --test connectors clickhouse_live -- --ignored
docker compose -f docker-compose.test.yml stop clickhouse
```

MongoDB and Elasticsearch collection schemas are sampled from documents;
their grids are read-only. Use their native commands for writes. Kafka has a
topic/message surface rather than relational row editing. BigQuery's grid is
read-only, with writes available through SQL. BigQuery uses refreshing Google
Application Default Credentials when its optional token field is blank. This
includes service-account credentials via `GOOGLE_APPLICATION_CREDENTIALS` and
local credentials created with `gcloud auth application-default login`. An
explicit access token still requires manual renewal. See the
[workbench guide](workbench-safety.md#bigquery-credentials) for setup and verification limits.

Atomic file imports are available on PostgreSQL, MySQL/InnoDB, SQLite and
CockroachDB. MySQL SQL imports accept data statements and reject DDL/session
commands that can commit implicitly. Other connectors reject atomic file imports.
Native SQL exports use one data snapshot across pages and selected tables;
MySQL non-InnoDB and other connectors report live reads. See
[workbench safety and limits](workbench-safety.md) for the complete contract.

Live disposable-server checks cover MongoDB 8, CockroachDB 25.1,
Elasticsearch 8.17 and Kafka 3.9. Embedded DuckDB checks cover actual
SQL execution, binding, structure, empty results and result limits.
BigQuery, Turso and D1 have local HTTP contract tests; these do not establish
live cloud-account verification. Vault tests cover all new credential types.

To rerun the disposable-server tests after creating the instances on the ports
listed in `crates/dbx-core/tests/connectors.rs`:

```sh
cargo test -p dbx-core --test connectors -- --ignored --test-threads=1
```

HTTP implementations follow the provider references:
[Turso SQL over HTTP](https://docs.turso.tech/sdk/http/reference),
[Cloudflare D1 query API](https://developers.cloudflare.com/api/resources/d1/subresources/database/methods/query/),
and [BigQuery jobs.query](https://docs.cloud.google.com/bigquery/docs/reference/rest/v2/jobs/query).
