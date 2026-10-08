# Explicit local MCP pairing

Unlock DBX, connect a native PostgreSQL/MySQL/SQLite/CockroachDB profile and use
**Share this database read-only via MCP** in the database menu. Sharing starts
one ephemeral loopback endpoint and an in-memory random token. **Copy private
MCP pairing configuration** copies a host configuration using DBX's
`--mcp-stdio` mode and child environment variables `DBX_MCP_URL` and
`DBX_MCP_TOKEN`. Never put the token in command arguments, issues or logs.
Keep copied host configuration private and remove it after use.

Direct HTTP clients can POST JSON-RPC to the copied loopback URL with a Bearer
token. Origin-bearing requests, incorrect Host/token, oversized bodies and
batches are rejected. Legacy `2025-11-25` initialize and current `2026-07-28`
per-request metadata are supported. Current requests require matching
`MCP-Protocol-Version` and `Mcp-Method` headers and client-capability metadata.

Tools expose connection information, namespaces, tables, column metadata and
first-page table rows. There are no connection-selection, raw-SQL, write,
filesystem, export or credential tools. Names must match the paired connection's
metadata. Limits: one active read, 100 rows, 200 columns, 1 MiB result, 10-second
deadline, 16 KiB request and 120 requests per minute. Table listing is capped at
1000 and namespace listing at 200. There is no unbounded traversal/cursor API.

**View MCP activity** shows bounded timestamp/tool/outcome metadata without
queries, values or tokens. Legacy cancellation notifications stop a matching
request; the stdio proxy can forward cancellation while a request is running.
Stop sharing, close the connection, lock the vault or quit DBX to revoke access.
Changing the selected database makes tools unavailable until the user pairs
again. The original CLI query assistant remains draft-only and separate.

Pairing is a local application capability, not remote OAuth. DBX does not listen
on a network interface or prompt external agents for credentials. Use database
accounts with the intended permissions, particularly for database functions.
