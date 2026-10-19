use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use redis::{Client, Cmd, Value, aio::MultiplexedConnection};

use crate::RedisCommandCatalog;
use crate::engine::{exec_result, query_result, row_limit};
use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, EntityKind, ExecResult,
    QueryOptions, QueryResult, Result, RowData, SqlStatement, TableInfo, TableRef, TableStructure,
};

/// Redis-backed engine. Redis has no tables, so DBX presents a virtual
/// `keys` collection with key/type/ttl columns while retaining a raw command
/// editor for the full Redis command set.
pub struct RedisEngine {
    read_only: bool,
    client: Client,
    connection: tokio::sync::Mutex<MultiplexedConnection>,
    /// Logical database currently selected. Clones of the multiplexed
    /// connection share one socket, so `SELECT` through any clone moves every
    /// subsequent command to that index.
    database: AtomicUsize,
    _tunnel: Option<crate::transport::Tunnel>,
}

impl std::fmt::Debug for RedisEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RedisEngine")
            .field("kind", &DatabaseKind::Redis)
            .finish_non_exhaustive()
    }
}

impl RedisEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        if config.kind != DatabaseKind::Redis {
            return Err(DbxError::InvalidConfig(format!(
                "RedisEngine cannot open a {} connection",
                config.kind
            )));
        }
        config.validate()?;
        let (config, tunnel) = crate::transport::prepare(config).await?;
        let mut info = redis::IntoConnectionInfo::into_connection_info(config.url.as_str())
            .map_err(|error| {
                DbxError::Connection(crate::error::connection_message(&config.url, error))
            })?;
        if let Some(socket) = &config.socket {
            info.addr = redis::ConnectionAddr::Unix(socket.clone());
        }
        let database = info.redis.db as usize;
        let client = Client::open(info).map_err(|error| {
            DbxError::Connection(crate::error::connection_message(&config.url, error))
        })?;
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        let connection = tokio::time::timeout(timeout, client.get_multiplexed_async_connection())
            .await
            .map_err(|_| DbxError::Connection("Redis connection timed out".into()))?
            .map_err(|error| {
                DbxError::Connection(crate::error::connection_message(&config.url, error))
            })?;
        Ok(Self {
            read_only: config.read_only,
            client,
            connection: tokio::sync::Mutex::new(connection),
            database: AtomicUsize::new(database),
            _tunnel: tunnel,
        })
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn kind(&self) -> DatabaseKind {
        DatabaseKind::Redis
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Discover this server's commands from its own runtime metadata.
    ///
    /// Redis 7+ documents command arguments through `COMMAND DOCS`. Older
    /// servers (and deployments which restrict DOCS) still expose names via
    /// `COMMAND`, so callers retain server-advertised command-name completion.
    pub async fn command_catalog(&self) -> Result<RedisCommandCatalog> {
        match self.send_command("COMMAND DOCS", &[]).await {
            Ok(value) => match RedisCommandCatalog::from_command_docs(&value) {
                Ok(catalog) => Ok(catalog),
                Err(_) => self.command_catalog_fallback().await,
            },
            Err(_) => self.command_catalog_fallback().await,
        }
    }

    async fn command_catalog_fallback(&self) -> Result<RedisCommandCatalog> {
        let value = self.send_command("COMMAND", &[]).await?;
        RedisCommandCatalog::from_command_metadata(&value)
    }

    /// Number of logical databases the server exposes. Falls back to the
    /// stock 16 when `CONFIG GET` is disabled (for example on managed Redis).
    async fn database_count(&self) -> Result<usize> {
        let value = self.send_command("CONFIG GET databases", &[]).await?;
        if let Value::Array(entries) = &value
            && entries.len() == 2
            && let Ok(text) = String::from_utf8(redis_argument(&cell_value(entries[1].clone()))?)
            && let Ok(count) = text.trim().parse::<usize>()
        {
            return Ok(count.max(1));
        }
        Ok(16)
    }

    /// Probe before sending a command. Only this harmless probe is retried;
    /// a failed user command is never replayed because it may have committed.
    async fn live_connection(&self) -> Result<tokio::sync::MutexGuard<'_, MultiplexedConnection>> {
        let mut connection = self.connection.lock().await;
        let alive = tokio::time::timeout(
            Duration::from_secs(5),
            redis::cmd("PING").query_async::<Value>(&mut *connection),
        )
        .await
        .is_ok_and(|result| result.is_ok());
        if !alive {
            let mut replacement = tokio::time::timeout(
                Duration::from_secs(10),
                self.client.get_multiplexed_async_connection(),
            )
            .await
            .map_err(|_| DbxError::Connection("Redis reconnect timed out".into()))??;
            redis::cmd("SELECT")
                .arg(self.database.load(Ordering::SeqCst))
                .query_async::<()>(&mut replacement)
                .await?;
            *connection = replacement;
        }
        Ok(connection)
    }

    async fn send_command(&self, command: &str, params: &[CellValue]) -> Result<Value> {
        let words = parse_command(command)?;
        if words.is_empty() {
            return Err(DbxError::Parse("Redis command cannot be empty".into()));
        }
        let mut cmd = Cmd::new();
        for word in &words {
            cmd.arg(word);
        }
        for value in params {
            cmd.arg(redis_argument(value)?);
        }
        let mut connection = self.live_connection().await?;
        let result = cmd.query_async(&mut *connection).await?;
        if words[0].eq_ignore_ascii_case("SELECT") {
            let database = words
                .get(1)
                .cloned()
                .or_else(|| params.first().map(ToString::to_string))
                .and_then(|value| value.parse().ok());
            if let Some(database) = database {
                self.database.store(database, Ordering::SeqCst);
            }
        }
        Ok(result)
    }

    async fn query_command(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        if self.read_only {
            crate::protected::ensure_query(DatabaseKind::Redis, &statement.sql)?;
        }
        let started = Instant::now();
        let command = parse_command(&statement.sql)?;
        let command_name = command.first().map(String::as_str).unwrap_or_default();
        let value = self.send_command(&statement.sql, &statement.params).await?;
        if command_name.eq_ignore_ascii_case("SCAN") {
            return self.query_scan(value, options, started).await;
        }

        let (columns, rows, truncated) = redis_value_rows(command_name, value, options)?;
        Ok(query_result(columns, rows, None, truncated, started))
    }

    /// Convert a SCAN reply into the stable keyspace grid shape. Redis SCAN
    /// replies are `[cursor, [keys...]]`; the cursor is pagination state and
    /// must never be rendered as a key row. TYPE and TTL are fetched in one
    /// pipeline so a browser refresh does not perform two round trips per key.
    async fn query_scan(
        &self,
        value: Value,
        options: QueryOptions,
        started: Instant,
    ) -> Result<QueryResult> {
        let (_, keys) = redis_scan_reply(value)?;
        let (keys, truncated) = limit_values(keys, options);
        self.key_rows(keys, truncated, started).await
    }

    /// Browse one page of the keyspace. A single `SCAN` step examines only
    /// about `COUNT` keys, so a selective pattern can return nothing even when
    /// matches exist; this keeps following the cursor until `target` keys are
    /// found, the keyspace is exhausted, or the step budget runs out. The
    /// returned cursor resumes the next page; `0` means the scan is complete.
    pub async fn scan_page(
        &self,
        pattern: &str,
        cursor: u64,
        target: usize,
    ) -> Result<(QueryResult, u64)> {
        const STEP_COUNT: usize = 1_000;
        const MAX_STEPS: usize = 256;
        let started = Instant::now();
        let mut cursor = cursor;
        let mut seen = std::collections::HashSet::new();
        let mut keys = Vec::new();
        for _ in 0..MAX_STEPS {
            let value = {
                let mut connection = self.live_connection().await?;
                redis::cmd("SCAN")
                    .arg(cursor)
                    .arg("MATCH")
                    .arg(pattern)
                    .arg("COUNT")
                    .arg(STEP_COUNT)
                    .query_async::<Value>(&mut *connection)
                    .await?
            };
            let (next, batch) = redis_scan_reply(value)?;
            for key in batch {
                // SCAN may return a key more than once while the table rehashes.
                if seen.insert(redis_argument(&cell_value(key.clone()))?) {
                    keys.push(key);
                }
            }
            cursor = next;
            if cursor == 0 || keys.len() >= target {
                break;
            }
        }
        let result = self.key_rows(keys, false, started).await?;
        Ok((result, cursor))
    }

    /// Fetch TYPE and TTL for scanned keys in one pipeline.
    async fn key_rows(
        &self,
        keys: Vec<Value>,
        truncated: bool,
        started: Instant,
    ) -> Result<QueryResult> {
        let mut rows = Vec::with_capacity(keys.len());

        if !keys.is_empty() {
            let mut pipeline = redis::pipe();
            for key in &keys {
                let argument = redis_argument(&cell_value(key.clone()))?;
                pipeline.cmd("TYPE").arg(&argument);
                pipeline.cmd("TTL").arg(&argument);
            }
            let mut connection = self.live_connection().await?;
            let metadata: Vec<Value> = pipeline.query_async(&mut *connection).await?;
            if metadata.len() != keys.len() * 2 {
                return Err(DbxError::Decode(format!(
                    "Redis SCAN metadata returned {} values for {} keys",
                    metadata.len(),
                    keys.len()
                )));
            }

            for (key, metadata) in keys.into_iter().zip(metadata.as_chunks::<2>().0.iter()) {
                rows.push(RowData::new(vec![
                    cell_value(key),
                    cell_value(metadata[0].clone()),
                    cell_value(metadata[1].clone()),
                ]));
            }
        }

        Ok(query_result(
            redis_scan_columns(),
            rows,
            None,
            truncated,
            started,
        ))
    }

    async fn execute_command(&self, statement: &SqlStatement) -> Result<ExecResult> {
        if self.read_only {
            return Err(DbxError::Query(
                "Protected connection: writes are disabled".into(),
            ));
        }
        let started = Instant::now();
        let value = self.send_command(&statement.sql, &statement.params).await?;
        let rows_affected = match value {
            Value::Int(value) if value >= 0 => value as u64,
            Value::Okay | Value::SimpleString(_) => 1,
            _ => 0,
        };
        Ok(exec_result(rows_affected, None, started))
    }
}

#[async_trait]
impl crate::Engine for RedisEngine {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Redis
    }

    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        Ok(vec![TableInfo {
            name: "keys".to_owned(),
            schema: None,
            kind: EntityKind::Collection,
        }])
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        let count = self.database_count().await?;
        Ok((0..count).map(|index| index.to_string()).collect())
    }

    async fn current_database(&self) -> Result<String> {
        Ok(self.database.load(Ordering::SeqCst).to_string())
    }

    async fn use_database(&self, name: &str) -> Result<()> {
        let index = name.trim().parse::<usize>().map_err(|_| {
            DbxError::InvalidConfig(format!("`{name}` is not a valid Redis database index"))
        })?;
        let mut cmd = Cmd::new();
        cmd.arg("SELECT").arg(index);
        let mut connection = self.live_connection().await?;
        tokio::time::timeout(
            Duration::from_secs(5),
            cmd.query_async::<()>(&mut *connection),
        )
        .await
        .map_err(|_| DbxError::Connection("Redis SELECT timed out".into()))?
        .map_err(|error| DbxError::Connection(error.to_string()))?;
        self.database.store(index, Ordering::SeqCst);
        Ok(())
    }

    async fn describe_table(&self, _table: &TableRef) -> Result<Vec<ColumnInfo>> {
        Ok(vec![
            ColumnInfo {
                name: "key".to_owned(),
                data_type: "string".to_owned(),
                enum_values: Vec::new(),
                nullable: false,
                ordinal: 0,
                primary_key: true,
                default_value: None,
            },
            ColumnInfo {
                name: "type".to_owned(),
                data_type: "string".to_owned(),
                enum_values: Vec::new(),
                nullable: true,
                ordinal: 1,
                primary_key: false,
                default_value: None,
            },
            ColumnInfo {
                name: "ttl".to_owned(),
                data_type: "integer".to_owned(),
                enum_values: Vec::new(),
                nullable: true,
                ordinal: 2,
                primary_key: false,
                default_value: None,
            },
        ])
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        Ok(TableStructure {
            columns: self.describe_table(table).await?,
            foreign_keys: Vec::new(),
            ..Default::default()
        })
    }

    async fn relational_schema(&self) -> Result<crate::RelationalSchema> {
        Err(DbxError::Unsupported {
            operation: "relational_schema".into(),
            kind: self.kind(),
        })
    }

    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        self.query_command(&SqlStatement::new(sql, Vec::new()), options)
            .await
    }

    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        self.query_command(statement, options).await
    }

    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        self.execute_command(statement).await
    }
}

fn parse_command(command: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in command.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(expected) = quote {
            if character == expected {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            character if character.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if escaped {
        current.push('\\');
    }
    if quote.is_some() {
        return Err(DbxError::Parse(
            "unterminated quote in Redis command".into(),
        ));
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

fn redis_argument(value: &CellValue) -> Result<Vec<u8>> {
    match value {
        CellValue::Null => Ok(b"".to_vec()),
        CellValue::Boolean(value) => Ok(if *value { b"1" } else { b"0" }.to_vec()),
        CellValue::Integer(value) => Ok(value.to_string().into_bytes()),
        CellValue::Unsigned(value) => Ok(value.to_string().into_bytes()),
        CellValue::Real(value) => Ok(value.to_string().into_bytes()),
        CellValue::Text(value) => Ok(value.as_bytes().to_vec()),
        CellValue::Bytes(value) => Ok(value.clone()),
        CellValue::Json(value) => Ok(value.to_string().into_bytes()),
    }
}

fn redis_value_rows(
    command: &str,
    value: Value,
    options: QueryOptions,
) -> Result<(Vec<ColumnInfo>, Vec<RowData>, bool)> {
    match value {
        Value::Array(values) => {
            // Only commands whose protocol explicitly defines alternating
            // pairs may use the two-column shape. An even-length MGET,
            // LRANGE, or SMEMBERS reply is still a list, not a hash.
            if command_returns_pairs(command)
                && values.len() >= 2
                && values.len() % 2 == 0
                && !values.iter().any(is_nested_value)
            {
                let columns = vec![
                    ColumnInfo::result("key", 0, "string"),
                    ColumnInfo::result("value", 1, "value"),
                ];
                let limit = row_limit(options);
                let mut rows = Vec::with_capacity(limit.unwrap_or(64).min(1024));
                let mut values = values.into_iter();
                let mut truncated = false;
                while let Some(key) = values.next() {
                    let Some(value) = values.next() else {
                        break;
                    };
                    if limit.is_some_and(|limit| rows.len() >= limit) {
                        truncated = true;
                        break;
                    }
                    rows.push(RowData::new(vec![cell_value(key), cell_value(value)]));
                }
                return Ok((columns, rows, truncated));
            }
            let columns = vec![ColumnInfo::result("value", 0, "value")];
            let (rows, truncated) = bounded_rows(values, options, |value| {
                RowData::new(vec![cell_value(value)])
            });
            Ok((columns, rows, truncated))
        }
        Value::Map(entries) => {
            let columns = vec![
                ColumnInfo::result("key", 0, "string"),
                ColumnInfo::result("value", 1, "value"),
            ];
            let (rows, truncated) = bounded_rows(entries, options, |(key, value)| {
                RowData::new(vec![cell_value(key), cell_value(value)])
            });
            Ok((columns, rows, truncated))
        }
        Value::Set(values) => {
            let (rows, truncated) = bounded_rows(values, options, |value| {
                RowData::new(vec![cell_value(value)])
            });
            Ok((
                vec![ColumnInfo::result("value", 0, "value")],
                rows,
                truncated,
            ))
        }
        Value::Attribute { data, .. } => redis_value_rows(command, *data, options),
        Value::Push { data, .. } => redis_value_rows(command, Value::Array(data), options),
        value => Ok((
            vec![ColumnInfo::result("value", 0, "value")],
            vec![RowData::new(vec![cell_value(value)])],
            false,
        )),
    }
}

/// Convert a Redis collection directly into the bounded grid shape. The
/// response owns all values already, so stopping the iterator after the first
/// omitted row releases the unrendered tail without allocating `RowData` for
/// it. This matters for commands such as HGETALL on a large hash.
fn bounded_rows<I, F>(values: I, options: QueryOptions, mut row: F) -> (Vec<RowData>, bool)
where
    I: IntoIterator,
    F: FnMut(I::Item) -> RowData,
{
    let limit = row_limit(options);
    let mut rows = Vec::with_capacity(limit.unwrap_or(64).min(1024));
    let mut truncated = false;
    for value in values {
        if limit.is_some_and(|limit| rows.len() >= limit) {
            truncated = true;
            break;
        }
        rows.push(row(value));
    }
    (rows, truncated)
}

fn command_returns_pairs(command: &str) -> bool {
    command.eq_ignore_ascii_case("HGETALL")
}

fn redis_scan_columns() -> Vec<ColumnInfo> {
    vec![
        ColumnInfo::result("key", 0, "string"),
        ColumnInfo::result("type", 1, "string"),
        ColumnInfo::result("ttl", 2, "integer"),
    ]
}

fn redis_scan_reply(value: Value) -> Result<(u64, Vec<Value>)> {
    let Value::Array(mut values) = unwrap_redis_container(value) else {
        return Err(DbxError::Decode(
            "Redis SCAN response was not an array".into(),
        ));
    };
    if values.len() != 2 {
        return Err(DbxError::Decode(format!(
            "Redis SCAN response contained {} elements; expected cursor and key list",
            values.len()
        )));
    }

    // The first element is the next cursor. It is pagination state and must
    // never be shown as a key row.
    let cursor = match unwrap_redis_container(values.remove(0)) {
        Value::BulkString(bytes) => String::from_utf8_lossy(&bytes).parse().ok(),
        Value::SimpleString(text) => text.parse().ok(),
        Value::Int(value) => u64::try_from(value).ok(),
        _ => None,
    }
    .ok_or_else(|| DbxError::Decode("Redis SCAN cursor was not a number".into()))?;
    match unwrap_redis_container(values.remove(0)) {
        Value::Array(keys) | Value::Set(keys) => Ok((cursor, keys)),
        other => Err(DbxError::Decode(format!(
            "Redis SCAN key list was {:?}, expected an array",
            other
        ))),
    }
}

fn unwrap_redis_container(value: Value) -> Value {
    match value {
        Value::Attribute { data, .. } => unwrap_redis_container(*data),
        Value::Push { data, .. } => Value::Array(data),
        value => value,
    }
}

fn limit_values(values: Vec<Value>, options: QueryOptions) -> (Vec<Value>, bool) {
    let Some(limit) = row_limit(options) else {
        return (values, false);
    };
    let truncated = values.len() > limit;
    (values.into_iter().take(limit).collect(), truncated)
}

fn is_nested_value(value: &Value) -> bool {
    matches!(value, Value::Array(_) | Value::Map(_) | Value::Set(_))
}

fn cell_value(value: Value) -> CellValue {
    match value {
        Value::Nil => CellValue::Null,
        Value::Int(value) => CellValue::Integer(value),
        Value::BulkString(value) => match String::from_utf8(value) {
            Ok(value) => CellValue::Text(value),
            Err(error) => CellValue::Bytes(error.into_bytes()),
        },
        Value::SimpleString(value) | Value::VerbatimString { text: value, .. } => {
            CellValue::Text(value)
        }
        Value::Okay => CellValue::Text("OK".to_owned()),
        Value::Double(value) => CellValue::Real(value),
        Value::Boolean(value) => CellValue::Boolean(value),
        Value::BigNumber(value) => CellValue::Text(value.to_string()),
        Value::Array(values) => CellValue::Json(
            values
                .into_iter()
                .map(|value| cell_value(value).to_string())
                .collect::<Vec<_>>()
                .into(),
        ),
        Value::Map(values) => {
            let mut object = serde_json::Map::new();
            for (key, value) in values {
                object.insert(
                    cell_value(key).to_string(),
                    serde_json::Value::String(cell_value(value).to_string()),
                );
            }
            CellValue::Json(serde_json::Value::Object(object))
        }
        Value::Set(values) | Value::Push { data: values, .. } => CellValue::Json(
            values
                .into_iter()
                .map(|value| cell_value(value).to_string())
                .collect::<Vec<_>>()
                .into(),
        ),
        Value::Attribute { data, .. } => cell_value(*data),
        Value::ServerError(error) => CellValue::Text(format!("{error:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_parser_preserves_quoted_and_escaped_arguments() {
        let words = parse_command(r#"SET greeting "hello world" NX"#).unwrap();
        assert_eq!(words, vec!["SET", "greeting", "hello world", "NX"]);
        assert!(parse_command("SET 'unterminated").is_err());
    }

    #[test]
    fn redis_hash_replies_are_tabular() {
        let (columns, rows, truncated) = redis_value_rows(
            "HGETALL",
            Value::Array(vec![
                Value::BulkString(b"name".to_vec()),
                Value::BulkString(b"Ada".to_vec()),
                Value::BulkString(b"active".to_vec()),
                Value::Int(1),
            ]),
            QueryOptions::default(),
        )
        .unwrap();
        assert!(!truncated);
        assert_eq!(columns.len(), 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values[0], CellValue::Text("name".into()));
        assert_eq!(rows[1].values[1], CellValue::Integer(1));
    }

    #[test]
    fn even_length_list_replies_are_not_assumed_to_be_pairs() {
        let (columns, rows, truncated) = redis_value_rows(
            "MGET",
            Value::Array(vec![
                Value::BulkString(b"Ada".to_vec()),
                Value::BulkString(b"Grace".to_vec()),
            ]),
            QueryOptions::default(),
        )
        .unwrap();
        assert!(!truncated);
        assert_eq!(
            columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["value"]
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].values, vec![CellValue::Text("Grace".into())]);
    }

    #[test]
    fn scan_discards_cursor_and_keeps_key_values() {
        let (cursor, keys) = redis_scan_reply(Value::Array(vec![
            Value::BulkString(b"42".to_vec()),
            Value::Array(vec![
                Value::BulkString(b"users:1".to_vec()),
                Value::BulkString(b"users:2".to_vec()),
            ]),
        ]))
        .unwrap();
        assert_eq!(cursor, 42);
        assert_eq!(keys.len(), 2);
        assert_eq!(
            cell_value(keys[0].clone()),
            CellValue::Text("users:1".into())
        );
        assert_eq!(
            redis_scan_columns()
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["key", "type", "ttl"]
        );
    }

    #[test]
    fn nested_replies_remain_single_raw_values() {
        let (columns, rows, truncated) = redis_value_rows(
            "HSCAN",
            Value::Array(vec![
                Value::BulkString(b"0".to_vec()),
                Value::Array(vec![
                    Value::BulkString(b"field".to_vec()),
                    Value::BulkString(b"value".to_vec()),
                ]),
            ]),
            QueryOptions::default(),
        )
        .unwrap();
        assert!(!truncated);
        assert_eq!(columns.len(), 1);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn redis_collection_rows_are_bounded_before_grid_conversion() {
        let values = (0..100)
            .map(|value| Value::BulkString(value.to_string().into_bytes()))
            .collect();
        let (columns, rows, truncated) = redis_value_rows(
            "LRANGE",
            Value::Array(values),
            QueryOptions { max_rows: Some(2) },
        )
        .unwrap();

        assert_eq!(columns.len(), 1);
        assert_eq!(rows.len(), 2);
        assert!(truncated);
        assert_eq!(rows[1].values, vec![CellValue::Text("1".into())]);
    }

    #[test]
    fn redis_limits_report_omitted_rows() {
        let rows = vec![RowData::default(), RowData::default()];
        let (rows, truncated) = bounded_rows(rows, QueryOptions { max_rows: Some(1) }, |row| row);
        assert_eq!(rows.len(), 1);
        assert!(truncated);

        let values = vec![Value::Int(1), Value::Int(2)];
        let (values, truncated) = limit_values(values, QueryOptions { max_rows: Some(2) });
        assert_eq!(values.len(), 2);
        assert!(!truncated);
    }
}

/// Snapshot for a structured editor. The binary revision token never enters
/// user JSON. A save checks the token and selected logical database atomically.
#[derive(Clone)]
pub struct RedisEditSnapshot {
    pub key: String,
    pub document: serde_json::Value,
    dump: Vec<u8>,
    database: usize,
}
const REDIS_EDIT_READ: &str = r#"
local t=redis.call('TYPE',KEYS[1]).ok
local memory=redis.call('MEMORY','USAGE',KEYS[1])
if memory and memory>4194304 then return redis.error_reply('Key exceeds editor memory limit (4 MiB)') end
local d=redis.call('DUMP',KEYS[1])
if not d then return redis.error_reply('Key no longer exists') end
local v
if t=='string' then
 if redis.call('STRLEN',KEYS[1])>1048576 then return redis.error_reply('Value exceeds editor limit (1 MiB)') end
 v=redis.call('GET',KEYS[1])
elseif t=='hash' then
 if redis.call('HLEN',KEYS[1])>10000 then return redis.error_reply('Hash exceeds editor limit (10000 fields)') end
 v=redis.call('HGETALL',KEYS[1])
elseif t=='list' then
 if redis.call('LLEN',KEYS[1])>10000 then return redis.error_reply('List exceeds editor limit (10000 entries)') end
 v=redis.call('LRANGE',KEYS[1],0,-1)
elseif t=='set' then
 if redis.call('SCARD',KEYS[1])>10000 then return redis.error_reply('Set exceeds editor limit (10000 members)') end
 v=redis.call('SMEMBERS',KEYS[1])
elseif t=='zset' then
 if redis.call('ZCARD',KEYS[1])>10000 then return redis.error_reply('Sorted set exceeds editor limit (10000 members)') end
 v=redis.call('ZRANGE',KEYS[1],0,-1,'WITHSCORES')
elseif t=='stream' then v=redis.call('XRANGE',KEYS[1],'-','+','COUNT',1000)
else return redis.error_reply('This key type has no structured editor') end
return {d,t,v}
"#;
const REDIS_EDIT_WRITE: &str = r#"
if redis.call('DUMP',KEYS[1])~=ARGV[1] then return redis.error_reply('Key changed or expired; reload before saving') end
local doc=cjson.decode(ARGV[2])
local t=doc.type
local ttl=redis.call('PTTL',KEYS[1])
if ttl==-2 then return redis.error_reply('Key expired; reload before saving') end
local v=doc.value
if t=='stream' then
 for _,entry in ipairs(doc.append) do
  local args={KEYS[1],'*'}
  for _,pair in ipairs(entry) do table.insert(args,pair[1]); table.insert(args,pair[2]) end
  redis.call('XADD',unpack(args))
 end
 for _,id in ipairs(doc.delete) do redis.call('XDEL',KEYS[1],id) end
 return 1
end
redis.call('DEL',KEYS[1])
if t=='string' then redis.call('SET',KEYS[1],v)
elseif t=='hash' then for _,pair in ipairs(v) do redis.call('HSET',KEYS[1],pair[1],pair[2]) end
elseif t=='list' then for _,item in ipairs(v) do redis.call('RPUSH',KEYS[1],item) end
elseif t=='set' then for _,item in ipairs(v) do redis.call('SADD',KEYS[1],item) end
elseif t=='zset' then for _,pair in ipairs(v) do redis.call('ZADD',KEYS[1],pair[2],pair[1]) end end
if ttl>=0 then redis.call('PEXPIRE',KEYS[1],ttl) end
return 1
"#;
impl RedisEngine {
    pub async fn read_structured(&self, key: &str) -> Result<RedisEditSnapshot> {
        let mut connection = self.live_connection().await?;
        let database = self.database.load(Ordering::SeqCst);
        let (dump, kind, content): (Vec<u8>, String, Value) = redis::Script::new(REDIS_EDIT_READ)
            .key(key)
            .invoke_async(&mut *connection)
            .await?;
        let value = match kind.as_str() {
            "string" => serde_json::json!(redis::from_redis_value::<String>(&content)?),
            "stream" => {
                let entries: Vec<(String, Vec<String>)> = redis::from_redis_value(&content)?;
                let entries = entries.into_iter().map(|(id, values)| serde_json::json!({"id":id,"fields":values.chunks(2).map(|pair| pair.to_vec()).collect::<Vec<_>>()})).collect::<Vec<_>>();
                return Ok(RedisEditSnapshot {
                    key: key.into(),
                    document: serde_json::json!({"type":"stream","entries":entries,"append":[],"delete":[]}),
                    dump,
                    database,
                });
            }
            "hash" | "zset" => {
                let values: Vec<String> = redis::from_redis_value(&content)?;
                serde_json::json!(
                    values
                        .chunks(2)
                        .map(|pair| pair.to_vec())
                        .collect::<Vec<_>>()
                )
            }
            _ => serde_json::json!(redis::from_redis_value::<Vec<String>>(&content)?),
        };
        Ok(RedisEditSnapshot {
            key: key.into(),
            document: serde_json::json!({"type":kind,"value":value}),
            dump,
            database,
        })
    }
    pub async fn save_structured(
        &self,
        snapshot: &RedisEditSnapshot,
        document: &serde_json::Value,
    ) -> Result<()> {
        if self.read_only {
            return Err(DbxError::Query(
                "Protected connection: writes are disabled".into(),
            ));
        }
        validate_redis_edit(&snapshot.document, document)?;
        let mut connection = self.live_connection().await?;
        if self.database.load(Ordering::SeqCst) != snapshot.database {
            return Err(DbxError::Query(
                "Selected Redis database changed; reload the key".into(),
            ));
        }
        redis::Script::new(REDIS_EDIT_WRITE)
            .key(&snapshot.key)
            .arg(&snapshot.dump)
            .arg(document.to_string())
            .invoke_async::<i64>(&mut *connection)
            .await?;
        Ok(())
    }
}
/// Validate every argument before the atomic script can change the key.
pub fn validate_redis_edit(
    original: &serde_json::Value,
    document: &serde_json::Value,
) -> Result<()> {
    use serde_json::Value as Json;
    let invalid = || {
        DbxError::Parse("Invalid structured Redis value. Keep the type; use strings for values, [field,value] pairs for hashes, and [member,score] pairs for sorted sets.".into())
    };
    let kind = original["type"].as_str().ok_or_else(invalid)?;
    if document["type"].as_str() != Some(kind) || document.to_string().len() > 4 * 1024 * 1024 {
        return Err(invalid());
    }
    let strings = |value: &Json| {
        value
            .as_array()
            .is_some_and(|values| values.len() <= 10000 && values.iter().all(Json::is_string))
    };
    let pairs = |value: &Json, scores: bool| {
        value.as_array().is_some_and(|values| {
            values.len() <= 10000
                && values.iter().all(|value| {
                    value.as_array().is_some_and(|pair| {
                        pair.len() == 2
                            && pair[0].is_string()
                            && pair[1].as_str().is_some_and(|value| {
                                !scores || value.parse::<f64>().is_ok_and(f64::is_finite)
                            })
                    })
                })
        })
    };
    let valid = match kind {
        "string" => document["value"]
            .as_str()
            .is_some_and(|value| value.len() <= 1024 * 1024),
        "hash" => pairs(&document["value"], false),
        "list" | "set" => strings(&document["value"]),
        "zset" => pairs(&document["value"], true),
        "stream" => {
            document["entries"] == original["entries"]
                && document["append"].as_array().is_some_and(|entries| {
                    entries.len() <= 1000
                        && entries.iter().all(|entry| {
                            pairs(entry, false)
                                && entry.as_array().is_some_and(|fields| {
                                    !fields.is_empty() && fields.len() <= 1000
                                })
                        })
                })
                && strings(&document["delete"])
                && document["delete"].as_array().is_some_and(|ids| {
                    ids.iter().all(|id| {
                        original["entries"]
                            .as_array()
                            .is_some_and(|entries| entries.iter().any(|entry| entry["id"] == *id))
                    })
                })
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(invalid()) }
}
#[cfg(test)]
mod structured_edit_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn structured_values_reject_lossy_or_invalid_mutations_before_writing() {
        for document in [
            json!({"type":"string","value":"hello"}),
            json!({"type":"hash","value":[["field","value"]]}),
            json!({"type":"list","value":["a","b"]}),
            json!({"type":"set","value":["a"]}),
            json!({"type":"zset","value":[["a","1.25"]]}),
        ] {
            validate_redis_edit(&document, &document).unwrap();
        }
        let original = json!({"type":"zset","value":[["a","1"]]});
        assert!(
            validate_redis_edit(&original, &json!({"type":"zset","value":[["a","NaN"]]})).is_err()
        );
        assert!(validate_redis_edit(&original, &json!({"type":"string","value":"x"})).is_err());
        let stream = json!({"type":"stream","entries":[{"id":"1-0","fields":[["x","y"]]}],"append":[],"delete":[]});
        let mut oversized = stream.clone();
        oversized["append"] = json!([[["valid", "first"]], vec![json!(["field", "value"]); 1001]]);
        assert!(
            validate_redis_edit(&stream, &oversized).is_err(),
            "the entire batch must validate before any append"
        );
        let string = json!({"type":"string","value":"original"});
        assert!(
            validate_redis_edit(
                &string,
                &json!({"type":"string","value":"x".repeat(1024 * 1024 + 1)})
            )
            .is_err()
        );
        let mut edit = stream.clone();
        edit["append"] = json!([[["new", "value"]]]);
        edit["delete"] = json!(["1-0"]);
        validate_redis_edit(&stream, &edit).unwrap();
        edit["entries"][0]["fields"] = json!([["x", "changed"]]);
        assert!(validate_redis_edit(&stream, &edit).is_err());
    }
}
