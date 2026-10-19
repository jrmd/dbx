# Community database drivers

DBX can run explicitly registered local driver programs. In **New connection → Community drivers**, choose **Register trusted driver…**, select a manifest, then select its displayed database name. Enter its connection URL and save/connect normally. The connection form shows the program name and path. Registration does not start the program; opening/testing a connection does.

Drivers run with your operating-system permissions and receive that connection's credentials. Only register programs from authors you trust. DBX copies manifests to its platform config directory under `dbx/drivers`; removing a manifest disables future connections through it. Executables stay at the absolute path provided by the author. Re-registering an existing ID requires removing its previous manifest first.

A driver declares a compatible database family to choose DBX's SQL dialect or document UI. Its own name and URL schemes can describe a new database. Native drivers remain available in the same picker. Community drivers currently use SQL-compatible or MongoDB-compatible families and URLs. Redis, search, and streaming families are unavailable because those workspaces require native interfaces. Native staged grid edits, atomic imports, streaming exports, backup, SSH/socket/cloud-token settings and persistent transaction sessions are unavailable. Each query or execute operation is explicit. Implement unsupported operations as an error response. Local driver registrations and executable paths are machine-specific.

A runnable stdlib-only SQLite adapter and manifest are in [`examples/drivers`](../examples/drivers/). Replace the Python/script absolute paths before registering it; connect with `example-sqlite:///absolute/path/to/database.sqlite`. The example supports metadata, queries and bound parameters and obeys protected SQLite file mode. It limits responses to 100,000 rows even when the caller requests unlimited rows.

## Manifest

```json
{
  "protocol": 1,
  "id": "example-community-db",
  "name": "Example Community Database",
  "family": "postgresql",
  "executable": "/absolute/path/to/example-driver",
  "arguments": [],
  "schemes": ["exampledb"]
}
```

`family` uses a SQL-compatible serialized `DatabaseKind` name (for example `postgresql`, `mysql` or `sqlite`) or `mongodb`. Redis, Elasticsearch and Kafka families are rejected in this protocol version. A manifest is limited to 64 KiB, the program path must be absolute, and the family and URL scheme must match the selected connection. URL query parameters and fragments are rejected; use the authority password field for credentials so profile storage encrypts it. Imported profile bundles cannot select a machine-specific driver program.

## Protocol version 1

DBX starts one process per operation. The process reads one JSON object from stdin until EOF, writes one JSON object to stdout, then exits. Credentials are sent only on stdin, never as arguments. DBX discards child stderr and does not display arbitrary driver error text, which could contain credentials.

```json
{
  "protocol": 1,
  "operation": "query_statement",
  "connection": {
    "url": "exampledb://user:password@host/database",
    "read_only": false,
    "database": "database"
  },
  "parameters": {
    "statement": {
      "sql": "SELECT name FROM users WHERE id = $1",
      "params": [{"type": "integer", "value": 42}]
    },
    "options": {"max_rows": 10000}
  }
}
```

A successful response is `{"protocol":1,"ok":true,"result":...}`. A failure is `{"protocol":1,"ok":false}` and a nonzero exit also fails. Query results use DBX's public serialized `QueryResult`/`ColumnInfo`/`RowData`/`CellValue` models. Parameter values must remain bound through the destination driver's API.

| Operation | Parameters | Result |
| --- | --- | --- |
| `connect` | `{}` | Current database name as a string |
| `list_tables` | `{}` | Array of `TableInfo` |
| `list_databases` | `{}` | Array of names |
| `use_database` | `{"name":"..."}` | `null`; the acknowledged name is passed in subsequent requests |
| `describe_table` | `{"table":{"name":"...","schema":null}}` | Array of `ColumnInfo` |
| `table_structure` | Same table object | `TableStructure` |
| `query` | `{"query":"...","options":{"max_rows":10000}}` | `QueryResult` |
| `query_statement` | `statement` and `options` as above | `QueryResult` |
| `execute` | `{"statement":{"sql":"...","params":[]}}` | `ExecResult` |
| `document_for_edit` | `table` and `identity` | Complete canonical Extended JSON document; document-compatible drivers only |

Requests and responses are limited to 16 MiB. Every operation must exit before the connection timeout (maximum five minutes); timed-out, cancelled, and oversized-response processes are killed. Unix cancellation terminates the process group, including helper processes. On other platforms only the direct process is guaranteed to terminate. DBX verifies result row widths and enforces the requested row cap. Drivers should stream/limit upstream reads rather than materializing unlimited data before replying.

Protected connections reject mutation statements in DBX before invoking the program. The program must also enforce read-only credentials and server permissions: an external driver is trusted code, not an operating-system sandbox. Persistent transaction statements (`BEGIN`, `COMMIT`, `ROLLBACK`, savepoints) are rejected by this version of the protocol. Normalized relational snapshots mark additional schema objects as uncaptured instead of issuing native system-catalog SQL through the driver.
