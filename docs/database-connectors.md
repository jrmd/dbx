# Database connectors

DBX provides native connections for PostgreSQL, MySQL, SQLite, Redis, MongoDB,
CockroachDB, DuckDB, Elasticsearch, BigQuery, Kafka, Turso and Cloudflare D1.

| Provider | Connection | Query editor and explorer |
| --- | --- | --- |
| MongoDB | `mongodb://localhost:27017/app`, replica-set seed lists, or `mongodb+srv://user:password@cluster/app` | JSON database commands; collections and bounded document results |
| CockroachDB | PostgreSQL URL supplied by CockroachDB, including its TLS options | SQL, schemas, tables, structure, primary keys and foreign keys |
| DuckDB | `duckdb:///absolute/path.duckdb` or `duckdb::memory:`; native file chooser | Embedded DuckDB SQL, tables, views, structure and bound row mutations |
| Elasticsearch | HTTP(S) endpoint; username/password for Basic authentication or `https://:API_KEY@host` | `METHOD /path` followed by an optional JSON body; indices and search hits |
| BigQuery | `bigquery://project/dataset?location=US`, with an OAuth access token in the masked API-token field | GoogleSQL, datasets, tables, typed results, job polling and result pagination |
| Kafka | `kafka://broker:9092`; optional `brokers=host:9092,other:9092` | Topics, JSON consume commands and acknowledged produce commands |
| Turso | `libsql://database-organization.turso.io`, `turso://...` or HTTPS, with a database token in the masked field | SQL over HTTP, tables, views, SQLite structure and bound row mutations |
| Cloudflare D1 | `d1://account-id/database-id`, with a Cloudflare API token in the masked field | REST SQL, tables, views, SQLite structure and bound row mutations |

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

MongoDB and Elasticsearch collection schemas are sampled from documents;
their grids are read-only. Use their native commands for writes. Kafka has a
topic/message surface rather than relational row editing. BigQuery's grid is
read-only, with writes available through SQL. BigQuery currently takes an
OAuth access token; renew it when it expires. Service-account login and automatic
OAuth refresh are not implemented.

SQL dump imports that require a transaction remain available on the original
SQL engines and CockroachDB. The newer SQL connectors reject that operation
rather than replaying a dump without the promised transaction.

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
