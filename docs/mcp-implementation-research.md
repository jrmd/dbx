# MCP implementation research: paired, read-only DBX server

**Scope.** This design is for a user-enabled local MCP surface owned by a running DBX UI. It is intentionally limited to one user-selected connection, metadata, and bounded table reads. It excludes arbitrary SQL, mutations, connection creation, credentials, exports, filesystem access, prompts, resources, sampling, and MCP extensions. This is research and an interoperable contract, not an implementation.

## Recommendation

Ship a **dual-era**, read-only MCP server with both standard transports:

* Streamable HTTP at one ephemeral `http://127.0.0.1:<port>/mcp` endpoint. This is the direct option for clients that support local HTTP.
* A DBX stdio proxy executable for clients that require a child-process server. It receives JSON-RPC on stdin/stdout and forwards authenticated requests to the running UI's loopback endpoint. Its pair secret and endpoint must arrive through inherited environment variables, never command-line arguments.

Support current MCP `2026-07-28` first, plus legacy `2025-11-25` for clients that use the `initialize` handshake. Current MCP has no initialize handshake: each request carries its protocol version and capabilities in `_meta`, and a current server must implement `server/discover`. The legacy revision uses `initialize`, a version response, and `notifications/initialized`. Supporting both is the smallest practical interoperability choice; do not implement deprecated HTTP+SSE just to support older clients.

Sources: [current versioning and legacy compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning), [current transport overview](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports), [2025-11-25 release](https://blog.modelcontextprotocol.io/posts/2025-11-25-first-mcp-anniversary/).

## Explicit DBX pairing and connection scope

1. In the running DBX UI, the user chooses **Share one connection with MCP**, sees the exact profile display name and database/driver metadata, and confirms the read-only tool list and data limit.
2. DBX creates an in-memory, cryptographically random pairing secret and an ephemeral loopback listener. The secret is a capability for precisely one UI-selected profile handle; no MCP tool accepts a connection id, host, URL, vault key, SSH setting, or TLS path.
3. The UI displays a connection recipe once. For HTTP it contains the loopback URL and secret. For stdio it supplies only the command plus environment names; the host config supplies the secret as an environment variable. Do not log or persist either value by default.
4. The binding expires when the user stops sharing, DBX exits, the profile is removed, or its selected endpoint/identity changes. A disconnect, vault lock, or failed health check makes tools return a non-secret unavailable result; it never prompts an MCP client for credentials or silently switches connections.

The pairing secret is application authorization, distinct from MCP's optional OAuth authorization. A local, single-user capability does not need an OAuth authorization server. If DBX later exposes a non-loopback endpoint, implement the MCP HTTP authorization specification rather than extending this pairing token into a remote auth design.

Sources: [MCP authorization scope](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization), [MCP security and consent principles](https://modelcontextprotocol.io/specification/2026-07-28).

## Protocol contract

### Modern `2026-07-28`

* Require every request's `params._meta.io.modelcontextprotocol/protocolVersion` to be `2026-07-28`, validate the accompanying client-capability object, and reply with MCP `UnsupportedProtocolVersionError` (`-32022`) listing supported versions for an unsupported value.
* Implement `server/discover`; advertise only the tools capability and stable `serverInfo` / instructions. Do not advertise resources, prompts, logging, roots, sampling, elicitation, subscriptions, or extensions.
* For `tools/list` and `tools/call`, require the current pairing secret on every request. Return the fixed tool list in deterministic order. The pairing is application state, but the MCP tools must not differ per client connection; they vary only by the explicit authorization capability.
* Return `structuredContent` conforming to each advertised output schema and also a JSON `TextContent` representation for older host compatibility. Tool validation / unknown tool names are JSON-RPC errors; an expected execution failure such as the paired connection becoming unavailable is a `tools/call` result with `isError: true`.

### Legacy `2025-11-25`

* First request is `initialize`; negotiate only `2025-11-25` (counter-offer it if needed), return `serverInfo` and `{ "tools": {} }`, then wait for `notifications/initialized` before normal requests.
* Keep the pairing capability per legacy transport session. A legacy HTTP response can use `Mcp-Session-Id`; require it on all subsequent requests if issued. A session end, pair expiry, or re-pair invalidates it.
* On subsequent HTTP requests require `MCP-Protocol-Version: 2025-11-25`. Unknown/unsupported header values receive HTTP `400` as required by the legacy Streamable HTTP binding.

Current MCP explicitly distinguishes the 2026 per-request metadata model from the pre-2026 `initialize` lifecycle. The current tools specification requires the tools capability, `tools/list`, valid JSON Schemas, and deterministic tool ordering. The legacy lifecycle requires the client `initialized` notification after a successful initialize response.

Sources: [2026 version behavior](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning), [2026 tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools), [2025 lifecycle](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle), [2025 Streamable HTTP headers and sessions](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports).

## Transport requirements

### Streamable HTTP

* Bind only `127.0.0.1` (optionally a separately opt-in `::1` listener), use an unguessable high port, and expose only `POST /mcp`. A `GET /mcp` returns `405` because DBX needs no server-to-client SSE stream.
* Each POST contains one UTF-8 JSON-RPC message. Support `Accept: application/json, text/event-stream` and answer requests with `application/json`; answer accepted notifications with `202` and no body. Reject batches, wrong content types, malformed JSON-RPC, and bodies above a small fixed limit.
* Validate **every** `Origin` header: simplest safe policy is to reject any non-empty Origin because browser access is not a DBX feature. Do not emit CORS headers. Require `Authorization: Bearer <pair-secret>` in constant-time comparison and rate-limit failed authentication and tool calls.
* On modern HTTP, require and cross-check `MCP-Protocol-Version` and `Mcp-Method` against JSON body metadata/method. On legacy HTTP, apply the negotiated protocol header after initialize. Keep HTTP responses no-store and avoid logging authorization headers, request bodies, data rows, or secrets.

The Streamable HTTP specification requires Origin validation, recommends loopback binding and authentication for local servers, uses one endpoint for POST and GET, and permits a JSON response rather than SSE. It also says that client disconnection is not a cancellation in legacy Streamable HTTP.

Sources: [Streamable HTTP security and message rules](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports), [2026 request metadata model](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports).

### Stdio proxy

* The host launches `dbx-mcp-stdio` as a subprocess. It reads exactly one UTF-8 JSON-RPC message per newline from stdin and writes exactly one valid JSON-RPC message per newline to stdout. All diagnostics go only to stderr.
* The proxy reads `DBX_MCP_URL` and `DBX_MCP_TOKEN` from its inherited environment, does not print them, and refuses to start when either is missing. It must use the same protocol/version validation and bounded tool contract as HTTP.
* Close stdin to request clean shutdown; proxy waits for current DB work to stop, then exits. The host can terminate it after a bounded grace period. Limit one outstanding tool call per stdio process.

The standard stdio binding requires newline-delimited messages and forbids non-MCP stdout. The 2025 authorization specification says stdio implementations should retrieve credentials from the environment rather than follow its HTTP OAuth flow.

Sources: [stdio transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports), [authorization transport guidance](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization), [legacy shutdown](https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle).

## Minimal read-only tool set

All schemas set `additionalProperties: false`; all identifier arguments are matched against metadata from the paired connection before the driver builds a quoted query. No tool accepts raw SQL.

| Tool | Input | Bounded result |
| --- | --- | --- |
| `dbx_connection_info` | `{}` | Non-secret display metadata: driver family, server version if already known, selected database/catalog, read-only scope, and capabilities. Exclude host, port, user, URL, TLS/SSH paths, and all credentials. |
| `dbx_list_namespaces` | `{}` | Namespaces/schemas/catalogs capped at 200. |
| `dbx_list_tables` | `{ "namespace": "...", "cursor": "..." }` | Tables/views from a server-generated opaque cursor, page size ≤100, capped total traversal per call. |
| `dbx_describe_table` | `{ "namespace": "...", "table": "..." }` | Column names/types/nullability, keys, and indexes; cap 200 columns and omit generated DDL, comments above a configured size, permissions, and trigger bodies. |
| `dbx_read_rows` | `{ "namespace": "...", "table": "...", "columns": ["..."], "cursor": "...", "limit": 1..100 }` | At most 100 rows, 1 MiB serialized result, and a short DB timeout. Columns must be known identifiers. Cursor is opaque, server-signed/expiring, and only issued for that table/profile. Return a truncation marker and next cursor where a stable keyset order is available. |

Use output schemas for every tool. A result contains `structuredContent` with `{ connection, rows, nextCursor, truncated, warnings }` or the appropriate metadata shape, plus a compact JSON text copy. Mark read operations read-only in the tool description; do not rely on annotations as an authorization boundary.

DBX should optionally show a compact activity strip in the UI with client name, tool name, row count, byte count, timestamp, and success/cancellation status. The user may stop sharing at any time. This fulfills MCP guidance that a human can see exposed tools and tool invocations while preserving the deliberately narrow server capability.

Sources: [tool schemas, results, and error behavior](https://modelcontextprotocol.io/specification/2026-07-28/server/tools), [tool interaction safety guidance](https://modelcontextprotocol.io/specification/2026-07-28/server/tools).

## Cancellation, timeouts, and limits

* Legacy clients cancel with `notifications/cancelled` carrying the original request ID. Maintain a request-id to DB cancellation token map, stop the query, free resources, and do not send a later response. Never cancel `initialize`.
* For modern Streamable HTTP, cancellation is closing the request response stream; stop the correlated DB work when DBX can prove the response stream is closed. For stdio, accept legacy `notifications/cancelled` as the byte-stream binding specifies.
* Enforce a hard per-tool deadline (for example 10 seconds), one active query per pairing, a concurrency queue of one, and rate limits for both success and error paths. Disconnecting a legacy HTTP client is not cancellation; do not assume it is.

Sources: [legacy cancellation behavior](https://modelcontextprotocol.io/specification/2025-06-18/basic/utilities/cancellation), [current transport cancellation](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports), [legacy HTTP disconnect semantics](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports).

## Acceptance evidence

1. A current client performs `server/discover`, then calls all five tools with `2026-07-28` metadata; a legacy client completes `initialize` / `initialized` at `2025-11-25` and performs the same calls.
2. `tools/list` is deterministic, only declares tools, and each input/output schema validates. Unknown tool and invalid input use protocol errors; unavailable paired connection uses `isError: true`.
3. An MCP caller cannot select another connection or send raw SQL. Attempts to pass host, URL, user, filters-as-SQL, oversized limit, unknown namespace/table/column, forged cursor, or DDL syntax are rejected before database execution.
4. Origin-bearing browser requests, missing/wrong tokens, non-loopback connections, oversized requests, requests after expiry, and replayed legacy session IDs fail without data disclosure. Logs and process listings contain no pair token or row values.
5. A 101-row table and a large-cell table stay within row/byte limits. Cancellation, timeout, DB disconnect, UI stop-sharing, and DBX exit end work and leave no listening socket or active DB query.
