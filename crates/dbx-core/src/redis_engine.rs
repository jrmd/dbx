use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use redis::{Client, Cmd, Value, aio::MultiplexedConnection};

use crate::engine::{exec_result, query_result, row_limit};
use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, EntityKind, ExecResult,
    QueryOptions, QueryResult, Result, RowData, SqlStatement, TableInfo, TableRef, TableStructure,
};

/// Redis-backed engine. Redis has no tables, so DBX presents a virtual
/// `keys` collection with key/type/ttl columns while retaining a raw command
/// editor for the full Redis command set.
pub struct RedisEngine {
    client: Client,
    connection: MultiplexedConnection,
    /// Logical database currently selected. Clones of the multiplexed
    /// connection share one socket, so `SELECT` through any clone moves every
    /// subsequent command to that index.
    database: AtomicUsize,
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
        let client = Client::open(config.url.as_str()).map_err(|error| {
            DbxError::Connection(crate::error::connection_message(&config.url, error))
        })?;
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        let connection = tokio::time::timeout(timeout, client.get_multiplexed_async_connection())
            .await
            .map_err(|_| DbxError::Connection("Redis connection timed out".into()))?
            .map_err(|error| {
                DbxError::Connection(crate::error::connection_message(&config.url, error))
            })?;
        // The URL path selects the initial logical database, defaulting to 0.
        let database = config
            .url
            .split_once("://")
            .and_then(|(_, rest)| rest.rsplit_once('/'))
            .and_then(|(_, path)| path.split(['?', '#']).next())
            .and_then(|path| path.parse::<usize>().ok())
            .unwrap_or(0);
        Ok(Self {
            client,
            connection,
            database: AtomicUsize::new(database),
        })
    }

    pub fn kind(&self) -> DatabaseKind {
        DatabaseKind::Redis
    }

    pub fn client(&self) -> &Client {
        &self.client
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

    async fn send_command(&self, command: &str, params: &[CellValue]) -> Result<Value> {
        let words = parse_command(command)?;
        if words.is_empty() {
            return Err(DbxError::Parse("Redis command cannot be empty".into()));
        }
        let mut cmd = Cmd::new();
        for word in words {
            cmd.arg(word);
        }
        for value in params {
            cmd.arg(redis_argument(value)?);
        }
        let mut connection = self.connection.clone();
        Ok(cmd.query_async(&mut connection).await?)
    }

    async fn query_command(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        let command = parse_command(&statement.sql)?;
        let command_name = command.first().map(String::as_str).unwrap_or_default();
        let value = self.send_command(&statement.sql, &statement.params).await?;
        if command_name.eq_ignore_ascii_case("SCAN") {
            return self.query_scan(value, options, started).await;
        }

        let (columns, rows) = redis_value_rows(command_name, value)?;
        let rows = limit_rows(rows, options);
        Ok(query_result(columns, rows, None, started))
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
        let keys = redis_scan_keys(value)?;
        let keys = limit_values(keys, options);
        let mut rows = Vec::with_capacity(keys.len());

        if !keys.is_empty() {
            let mut pipeline = redis::pipe();
            for key in &keys {
                let argument = redis_argument(&cell_value(key.clone()))?;
                pipeline.cmd("TYPE").arg(&argument);
                pipeline.cmd("TTL").arg(&argument);
            }
            let mut connection = self.connection.clone();
            let metadata: Vec<Value> = pipeline.query_async(&mut connection).await?;
            if metadata.len() != keys.len() * 2 {
                return Err(DbxError::Decode(format!(
                    "Redis SCAN metadata returned {} values for {} keys",
                    metadata.len(),
                    keys.len()
                )));
            }

            for (key, metadata) in keys.into_iter().zip(metadata.chunks_exact(2)) {
                rows.push(RowData::new(vec![
                    cell_value(key),
                    cell_value(metadata[0].clone()),
                    cell_value(metadata[1].clone()),
                ]));
            }
        }

        Ok(query_result(redis_scan_columns(), rows, None, started))
    }

    async fn execute_command(&self, statement: &SqlStatement) -> Result<ExecResult> {
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
        let mut connection = self.connection.clone();
        tokio::time::timeout(
            Duration::from_secs(5),
            cmd.query_async::<()>(&mut connection),
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
                nullable: false,
                ordinal: 0,
                primary_key: true,
            },
            ColumnInfo {
                name: "type".to_owned(),
                data_type: "string".to_owned(),
                nullable: true,
                ordinal: 1,
                primary_key: false,
            },
            ColumnInfo {
                name: "ttl".to_owned(),
                data_type: "integer".to_owned(),
                nullable: true,
                ordinal: 2,
                primary_key: false,
            },
        ])
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        Ok(TableStructure {
            columns: self.describe_table(table).await?,
            foreign_keys: Vec::new(),
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

fn redis_value_rows(command: &str, value: Value) -> Result<(Vec<ColumnInfo>, Vec<RowData>)> {
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
                let rows = values
                    .chunks_exact(2)
                    .map(|pair| {
                        RowData::new(vec![
                            cell_value(pair[0].clone()),
                            cell_value(pair[1].clone()),
                        ])
                    })
                    .collect();
                return Ok((columns, rows));
            }
            let columns = vec![ColumnInfo::result("value", 0, "value")];
            let rows = values
                .into_iter()
                .map(|value| RowData::new(vec![cell_value(value)]))
                .collect();
            Ok((columns, rows))
        }
        Value::Map(entries) => {
            let columns = vec![
                ColumnInfo::result("key", 0, "string"),
                ColumnInfo::result("value", 1, "value"),
            ];
            let rows = entries
                .into_iter()
                .map(|(key, value)| RowData::new(vec![cell_value(key), cell_value(value)]))
                .collect();
            Ok((columns, rows))
        }
        Value::Set(values) => Ok((
            vec![ColumnInfo::result("value", 0, "value")],
            values
                .into_iter()
                .map(|value| RowData::new(vec![cell_value(value)]))
                .collect(),
        )),
        Value::Attribute { data, .. } => redis_value_rows(command, *data),
        Value::Push { data, .. } => redis_value_rows(command, Value::Array(data)),
        value => Ok((
            vec![ColumnInfo::result("value", 0, "value")],
            vec![RowData::new(vec![cell_value(value)])],
        )),
    }
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

fn redis_scan_keys(value: Value) -> Result<Vec<Value>> {
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

    // The first element is the next cursor. It is deliberately discarded;
    // showing it as a row makes the keyspace browser look as if a cursor were
    // an actual key.
    let _cursor = values.remove(0);
    match unwrap_redis_container(values.remove(0)) {
        Value::Array(keys) | Value::Set(keys) => Ok(keys),
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

fn limit_rows(rows: Vec<RowData>, options: QueryOptions) -> Vec<RowData> {
    rows.into_iter()
        .take(row_limit(options).unwrap_or(usize::MAX))
        .collect()
}

fn limit_values(values: Vec<Value>, options: QueryOptions) -> Vec<Value> {
    values
        .into_iter()
        .take(row_limit(options).unwrap_or(usize::MAX))
        .collect()
}

fn is_nested_value(value: &Value) -> bool {
    matches!(value, Value::Array(_) | Value::Map(_) | Value::Set(_))
}

fn cell_value(value: Value) -> CellValue {
    match value {
        Value::Nil => CellValue::Null,
        Value::Int(value) => CellValue::Integer(value),
        Value::BulkString(value) => match String::from_utf8(value.clone()) {
            Ok(value) => CellValue::Text(value),
            Err(_) => CellValue::Bytes(value),
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
        let (columns, rows) = redis_value_rows(
            "HGETALL",
            Value::Array(vec![
                Value::BulkString(b"name".to_vec()),
                Value::BulkString(b"Ada".to_vec()),
                Value::BulkString(b"active".to_vec()),
                Value::Int(1),
            ]),
        )
        .unwrap();
        assert_eq!(columns.len(), 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values[0], CellValue::Text("name".into()));
        assert_eq!(rows[1].values[1], CellValue::Integer(1));
    }

    #[test]
    fn even_length_list_replies_are_not_assumed_to_be_pairs() {
        let (columns, rows) = redis_value_rows(
            "MGET",
            Value::Array(vec![
                Value::BulkString(b"Ada".to_vec()),
                Value::BulkString(b"Grace".to_vec()),
            ]),
        )
        .unwrap();
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
        let keys = redis_scan_keys(Value::Array(vec![
            Value::BulkString(b"42".to_vec()),
            Value::Array(vec![
                Value::BulkString(b"users:1".to_vec()),
                Value::BulkString(b"users:2".to_vec()),
            ]),
        ]))
        .unwrap();
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
        let (columns, rows) = redis_value_rows(
            "HSCAN",
            Value::Array(vec![
                Value::BulkString(b"0".to_vec()),
                Value::Array(vec![
                    Value::BulkString(b"field".to_vec()),
                    Value::BulkString(b"value".to_vec()),
                ]),
            ]),
        )
        .unwrap();
        assert_eq!(columns.len(), 1);
        assert_eq!(rows.len(), 2);
    }
}
