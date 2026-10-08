//! Explicit, ephemeral, single-connection pairing. No SQL or write tools.
use crate::{DatabaseEngine, DbxError, Result, TableRef};
use axum::{
    Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

pub struct Pairing {
    pub url: String,
    token: Zeroizing<String>,
    state: Arc<ServerState>,
    task: tokio::task::JoinHandle<()>,
}
impl Pairing {
    pub async fn start(engine: Arc<DatabaseEngine>) -> Result<Self> {
        if !matches!(engine.as_ref(), DatabaseEngine::Sql(_)) {
            return Err(DbxError::Query(
                "MCP reads currently require PostgreSQL, MySQL, SQLite or CockroachDB".into(),
            ));
        }
        let source = engine;
        let DatabaseEngine::Sql(sql) = source.as_ref() else {
            unreachable!("native engine checked")
        };
        let engine = Arc::new(DatabaseEngine::Sql(sql.frozen_pool().await?));
        let database = engine.current_database().await?;
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy)
            .map_err(|_| DbxError::Io("Could not generate a pairing secret".into()))?;
        let token = Zeroizing::new(
            entropy
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| DbxError::Io(e.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|e| DbxError::Io(e.to_string()))?;
        let state = Arc::new(ServerState {
            source,
            engine,
            database,
            token: token.clone(),
            authority: address.to_string(),
            revoked: CancellationToken::new(),
            active: Mutex::new(HashMap::new()),
            gate: tokio::sync::Semaphore::new(1),
            rate: Mutex::new((Instant::now(), 0)),
            activity: Mutex::new(VecDeque::new()),
        });
        let router = Router::new()
            .route("/mcp", post(handle))
            .layer(DefaultBodyLimit::max(16 * 1024))
            .with_state(state.clone());
        let cancel = state.revoked.clone();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(cancel.cancelled_owned())
                .await;
        });
        Ok(Self {
            url: format!("http://{address}/mcp"),
            token,
            state,
            task,
        })
    }
    pub fn recipe(&self, executable: &std::path::Path) -> Value {
        json!({"mcpServers":{"dbx":{"command":executable,"args":["--mcp-stdio"],"env":{"DBX_MCP_URL":self.url,"DBX_MCP_TOKEN":self.token.as_str()}}}})
    }
    pub fn activity(&self) -> Vec<String> {
        self.state
            .activity
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}
impl Drop for Pairing {
    fn drop(&mut self) {
        self.state.revoked.cancel();
        self.task.abort();
    }
}
struct ServerState {
    source: Arc<DatabaseEngine>,
    engine: Arc<DatabaseEngine>,
    database: String,
    token: Zeroizing<String>,
    authority: String,
    revoked: CancellationToken,
    active: Mutex<HashMap<String, CancellationToken>>,
    gate: tokio::sync::Semaphore,
    rate: Mutex<(Instant, u32)>,
    activity: Mutex<VecDeque<String>>,
}
fn response(status: StatusCode, value: Value) -> Response {
    (status, [("cache-control", "no-store")], axum::Json(value)).into_response()
}
fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}
async fn handle(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if state.revoked.is_cancelled() {
        return StatusCode::GONE.into_response();
    }
    if headers.contains_key("origin")
        || headers.get("host").and_then(|h| h.to_str().ok()) != Some(state.authority.as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    {
        let mut rate = state.rate.lock().unwrap_or_else(|p| p.into_inner());
        if rate.0.elapsed() > Duration::from_secs(60) {
            *rate = (Instant::now(), 0);
        }
        rate.1 += 1;
        if rate.1 > 120 {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
    }
    let authorization = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_equal(authorization.as_bytes(), state.token.as_bytes()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !headers
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v.split(';').next() == Some("application/json"))
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let message: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return response(StatusCode::OK, error(Value::Null, -32700, "Invalid JSON")),
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !message.is_object()
        || message["jsonrpc"] != "2.0"
        || method.is_empty()
        || !(message.get("id").is_none()
            || id.is_i64()
            || id.is_u64()
            || id.as_str().is_some_and(|id| id.len() <= 64))
    {
        return response(
            StatusCode::OK,
            error(id, -32600, "Invalid JSON-RPC request"),
        );
    }
    let modern = message["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str();
    if let Some(version) = modern {
        if !message["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"].is_object() {
            return response(
                StatusCode::BAD_REQUEST,
                error(id, -32602, "Missing clientCapabilities metadata"),
            );
        }

        if version != "2026-07-28" {
            return response(
                StatusCode::BAD_REQUEST,
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2026-07-28","2025-11-25"],"requested":version}}}),
            );
        }
        if headers
            .get("mcp-protocol-version")
            .and_then(|h| h.to_str().ok())
            != Some(version)
            || headers.get("mcp-method").and_then(|h| h.to_str().ok()) != Some(method)
        {
            return response(
                StatusCode::BAD_REQUEST,
                error(id, -32020, "MCP header mismatch"),
            );
        }
    } else if method != "initialize"
        && headers
            .get("mcp-protocol-version")
            .and_then(|h| h.to_str().ok())
            != Some("2025-11-25")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if method == "notifications/cancelled" {
        let key = message["params"]["requestId"].to_string();
        if let Some(cancel) = state
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
        {
            cancel.cancel();
        }
        return StatusCode::ACCEPTED.into_response();
    }
    if !message.as_object().unwrap().contains_key("id") {
        return StatusCode::ACCEPTED.into_response();
    }
    let mut result = match method {
        "initialize" => {
            json!({"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"DBX","version":env!("CARGO_PKG_VERSION")},"instructions":"Read-only access to one explicitly paired database. No SQL or writes. Table rows are capped at 100 and 1 MiB."})
        }
        "server/discover" => {
            json!({"supportedVersions":["2026-07-28","2025-11-25"],"capabilities":{"tools":{}},"instructions":"One paired database; metadata and first 100 rows only; no SQL or writes."})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools":tools()}),
        "tools/call" => {
            let name = message["params"]["name"].as_str().unwrap_or_default();
            let arguments = message["params"]
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !tools().iter().any(|tool| tool["name"] == name) {
                return response(StatusCode::OK, error(id, -32602, "Unknown read tool"));
            }
            if !valid_arguments(name, &arguments) {
                return response(StatusCode::OK, error(id, -32602, "Invalid tool arguments"));
            }
            let Ok(_permit) = state.gate.try_acquire() else {
                return response(
                    StatusCode::OK,
                    error(
                        id,
                        -32000,
                        "One read is already running; retry after it finishes",
                    ),
                );
            };
            let key = id.to_string();
            let cancel = CancellationToken::new();
            {
                let mut active = state.active.lock().unwrap_or_else(|p| p.into_inner());
                if active.contains_key(&key) {
                    return response(StatusCode::OK, error(id, -32600, "Duplicate request id"));
                }
                active.insert(key.clone(), cancel.clone());
            }
            let read = tokio::select! {
                _ = state.revoked.cancelled() => Err(DbxError::Query("Pairing revoked".into())),
                _ = cancel.cancelled() => Err(DbxError::Query("Read cancelled".into())),
                result = tokio::time::timeout(Duration::from_secs(10), read_tool(&state, name, arguments)) => result.unwrap_or_else(|_| Err(DbxError::Query("Read deadline exceeded".into()))),
            };
            state
                .active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&key);
            let mut activity = state.activity.lock().unwrap_or_else(|p| p.into_inner());
            if activity.len() >= 100 {
                activity.pop_front();
            }
            activity.push_back(format!(
                "{} · {} · {}",
                chrono::Utc::now().format("%H:%M:%S UTC"),
                name,
                if read.is_ok() {
                    "success"
                } else {
                    "failed or cancelled"
                }
            ));
            if cancel.is_cancelled() {
                return StatusCode::ACCEPTED.into_response();
            }
            match read {
                Ok(value) => {
                    json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
                }
                Err(_) => {
                    json!({"content":[{"type":"text","text":"Read unavailable, cancelled, or outside the 100-row/1 MiB/10-second budget. Check DBX activity and retry."}],"isError":true})
                }
            }
        }
        _ => return response(StatusCode::OK, error(id, -32601, "Method not found")),
    };
    if modern.is_some() {
        result["resultType"] = "complete".into();
        result["_meta"] = json!({"io.modelcontextprotocol/serverInfo":{"name":"DBX","version":env!("CARGO_PKG_VERSION")}});
    }
    response(
        StatusCode::OK,
        json!({"jsonrpc":"2.0","id":id,"result":result}),
    )
}
fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn tools() -> Vec<Value> {
    [
        ("dbx_list_namespaces", "First 200 namespaces in the paired database", json!({})),
        ("dbx_connection_info", "Selected database and read-only scope", json!({})),
        ("dbx_list_tables", "First 1000 tables and views in the paired database", json!({})),
        ("dbx_describe_table", "Column metadata, capped at 200 columns", json!({"table":{"type":"string","minLength":1},"schema":{"type":"string"}})),
        ("dbx_read_rows", "First 100 rows from a known table; no raw SQL, filters or writes", json!({"table":{"type":"string","minLength":1},"schema":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}})),
    ].into_iter().map(|(name, description, properties)| json!({"name":name,"description":description,"annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false},"inputSchema":{"type":"object","properties":properties,"required":if name.ends_with("table") || name == "dbx_read_rows" { vec!["table"] } else { vec![] },"additionalProperties":false},"outputSchema":{"type":"object","additionalProperties":true}})).collect()
}
fn valid_arguments(name: &str, arguments: &Value) -> bool {
    let Some(object) = arguments.as_object() else {
        return false;
    };
    let table_tool = matches!(name, "dbx_describe_table" | "dbx_read_rows");
    object.iter().all(|(key, value)| match key.as_str() {
        "table" | "schema" if table_tool => value
            .as_str()
            .is_some_and(|v| !v.is_empty() && v.len() <= 256),
        "limit" if name == "dbx_read_rows" => {
            value.as_u64().is_some_and(|v| (1..=100).contains(&v))
        }
        _ => false,
    }) && (!table_tool || object.contains_key("table"))
}
async fn read_tool(state: &ServerState, name: &str, arguments: Value) -> Result<Value> {
    if state.source.current_database().await? != state.database {
        return Err(DbxError::Query(
            "Selected database changed; re-pair in DBX".into(),
        ));
    }
    if name == "dbx_connection_info" {
        return Ok(
            json!({"database":state.database,"kind":state.engine.kind(),"readOnly":true,"maxRows":100,"maxBytes":1048576}),
        );
    }
    let mut tables = state.engine.list_tables().await?;
    if name == "dbx_list_namespaces" {
        let namespaces = tables
            .iter()
            .filter_map(|table| table.schema.clone())
            .collect::<std::collections::BTreeSet<_>>();
        return bounded(
            json!({"namespaces":namespaces.iter().take(200).collect::<Vec<_>>(),"truncated":namespaces.len()>200}),
        );
    }
    if name == "dbx_list_tables" {
        let truncated = tables.len() > 1000;
        tables.truncate(1000);
        return bounded(json!({"tables":tables,"truncated":truncated}));
    }
    let table = TableRef {
        name: arguments["table"].as_str().unwrap().into(),
        schema: arguments["schema"].as_str().map(str::to_owned),
    };
    if !tables.iter().any(|info| {
        info.name == table.name
            && info.schema == table.schema
            && info.kind == crate::EntityKind::Table
    }) {
        return Err(DbxError::Query("Unknown base table".into()));
    }
    let columns = state.engine.describe_table(&table).await?;
    if columns.len() > 200 {
        return Err(DbxError::Query("Table exceeds 200-column budget".into()));
    }
    if name == "dbx_describe_table" {
        return bounded(
            json!({"columns":columns.iter().map(|column| json!({"name":column.name,"type":column.data_type,"nullable":column.nullable,"primaryKey":column.primary_key})).collect::<Vec<_>>()}),
        );
    }
    let limit = arguments["limit"].as_u64().unwrap_or(100) as usize;
    let names = columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    let order = columns
        .iter()
        .filter(|c| c.primary_key)
        .map(|c| crate::Order {
            column: c.name.clone(),
            direction: crate::OrderDirection::Ascending,
        })
        .collect::<Vec<_>>();
    let sql = crate::build_select_with_columns(
        state.engine.kind(),
        &table,
        &names,
        &[],
        &order,
        Some(crate::Page {
            limit: limit as u32 + 1,
            offset: 0,
        }),
        &columns,
    )?;
    let mut transaction = crate::console::SqlTransaction::begin(&state.engine, true).await?;
    let mut rows = Vec::new();
    let mut bytes = 0;
    transaction
        .stream_rows(&sql.sql, |_: &[crate::ColumnInfo], row| {
            if let Some(row) = row {
                bytes += serde_json::to_vec(&row)
                    .map_err(|e| DbxError::Parse(e.to_string()))?
                    .len();
                if bytes > 1024 * 1024 {
                    return Err(DbxError::Query("Row result exceeds 1 MiB".into()));
                }
                rows.push(row);
            }
            Ok(())
        })
        .await?;
    transaction.commit().await?;
    let truncated = rows.len() > limit;
    rows.truncate(limit);
    bounded(json!({"columns":names,"rows":rows,"truncated":truncated}))
}
fn bounded(value: Value) -> Result<Value> {
    if value.to_string().len() > 1024 * 1024 {
        Err(DbxError::Query("Result exceeds 1 MiB".into()))
    } else {
        Ok(value)
    }
}

/// Run before opening the GUI. Secrets arrive in the child environment only.
pub async fn stdio_proxy() -> Result<()> {
    use std::io::{BufRead, Read, Write};
    let endpoint = std::env::var("DBX_MCP_URL")
        .map_err(|_| DbxError::Parse("DBX_MCP_URL is required".into()))?;
    let token = Zeroizing::new(
        std::env::var("DBX_MCP_TOKEN")
            .map_err(|_| DbxError::Parse("DBX_MCP_TOKEN is required".into()))?,
    );
    let url =
        url::Url::parse(&endpoint).map_err(|_| DbxError::Parse("Invalid MCP endpoint".into()))?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.path() != "/mcp"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(DbxError::Parse(
            "MCP proxy requires the paired loopback endpoint".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|e| DbxError::Io(e.to_string()))?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<Result<Vec<u8>>>(4);
    tokio::task::spawn_blocking(move || {
        let input = std::io::stdin();
        let mut input = input.lock();
        loop {
            let mut bytes = Vec::new();
            match (&mut input)
                .take(16 * 1024 + 1)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) => break,
                Ok(_) if bytes.len() <= 16 * 1024 => {
                    if sender.blocking_send(Ok(bytes)).is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = sender.blocking_send(Err(DbxError::Parse(
                        "Invalid or oversized MCP message".into(),
                    )));
                    break;
                }
            }
        }
    });
    let mut pending = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            message = receiver.recv() => {
                let Some(bytes) = message else { pending.abort_all(); break; };
                let message: Value = serde_json::from_slice(&bytes?).map_err(|_| DbxError::Parse("Invalid MCP message".into()))?;
                if pending.len() >= 4 { return Err(DbxError::Parse("Too many concurrent MCP messages".into())); }
                let client = client.clone(); let url = url.clone(); let token = token.clone();
                pending.spawn(async move {
                    let method = message["method"].as_str().unwrap_or_default();
                    let version = message["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str().unwrap_or("2025-11-25");
                    let response = client.post(url).bearer_auth(token.as_str()).header("MCP-Protocol-Version", version).header("Mcp-Method", method).json(&message).send().await.map_err(|_| DbxError::Io("DBX pairing is unavailable".into()))?;
                    if response.status() == reqwest::StatusCode::ACCEPTED { return Ok(None); }
                    // Protocol errors can use HTTP 400 and still carry valid JSON-RPC.
                    if !response.status().is_success() && response.status() != reqwest::StatusCode::BAD_REQUEST { return Err(DbxError::Io(format!("DBX pairing rejected the request ({})", response.status()))); }
                    let value: Value = response.json().await.map_err(|_| DbxError::Parse("Invalid MCP response".into()))?;
                    Ok::<_, DbxError>(Some(value))
                });
            }
            result = pending.join_next(), if !pending.is_empty() => {
                if let Some(value) = result.unwrap().map_err(|_| DbxError::Io("MCP proxy request failed".into()))?? {
                    writeln!(std::io::stdout(), "{value}").map_err(|e| DbxError::Io(e.to_string()))?;
                }
            }
        }
    }
    Ok(())
}
