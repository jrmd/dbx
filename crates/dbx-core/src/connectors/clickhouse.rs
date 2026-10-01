//! ClickHouse's HTTP SQL interface. Sorting keys never imply unique row identity.
use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, EntityKind,
    QueryOptions, QueryResult, Result, RowData, SqlStatement, TableInfo, TableRef,
};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use url::Url;

pub(super) struct ClickHouseEngine {
    client: Client,
    endpoint: Url,
    username: String,
    password: String,
    database: RwLock<String>,
}

fn invalid(message: &str) -> DbxError {
    DbxError::InvalidConfig(message.into())
}
fn decode_error() -> DbxError {
    DbxError::Decode("Invalid ClickHouse JSONCompact response; omit explicit FORMAT clauses".into())
}

impl ClickHouseEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        config.validate()?;
        let raw = if let Some(rest) = config.url.strip_prefix("clickhouse://") {
            format!("http://{rest}")
        } else {
            config.url.clone()
        };
        let mut endpoint = Url::parse(&raw).map_err(|_| invalid("Invalid ClickHouse URL"))?;
        let host = endpoint
            .host_str()
            .ok_or_else(|| invalid("ClickHouse host is required"))?;
        let username = super::decode(endpoint.username())?;
        let password = endpoint
            .password()
            .map(super::decode)
            .transpose()?
            .unwrap_or_default();
        if endpoint.scheme() == "http"
            && (!username.is_empty() || !password.is_empty())
            && !matches!(host, "localhost" | "127.0.0.1" | "[::1]")
        {
            return Err(invalid("Use HTTPS when sending database credentials"));
        }
        let mut database = super::decode(endpoint.path().trim_matches('/'))?;
        for (key, value) in endpoint.query_pairs() {
            if key != "database" {
                return Err(invalid(
                    "ClickHouse URL accepts only the database query option",
                ));
            }
            database = value.into_owned();
        }
        if database.is_empty() {
            database = "default".into();
        }
        if config.url.starts_with("clickhouse://") && endpoint.port().is_none() {
            endpoint
                .set_port(Some(8123))
                .map_err(|_| invalid("Invalid ClickHouse port"))?;
        }
        let _ = endpoint.set_username("");
        let _ = endpoint.set_password(None);
        endpoint.set_path("/");
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        let client = Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| invalid("Cannot create ClickHouse HTTP client"))?;
        let engine = Self {
            client,
            endpoint,
            username: if username.is_empty() {
                "default".into()
            } else {
                username
            },
            password,
            database: RwLock::new(database),
        };
        engine.query("SELECT 1", QueryOptions::default()).await?;
        Ok(engine)
    }

    async fn run(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
        database: &str,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        let (sql, params) = bind(statement)?;
        let mut endpoint = self.endpoint.clone();
        {
            let mut query = endpoint.query_pairs_mut();
            query
                .append_pair("database", database)
                .append_pair("default_format", "JSONCompact")
                .append_pair("output_format_json_quote_decimals", "1")
                .append_pair("output_format_json_quote_64bit_integers", "1")
                .append_pair("wait_end_of_query", "1");
            if let Some(limit) = crate::engine::row_limit(options) {
                query
                    .append_pair("max_result_rows", &limit.saturating_add(1).to_string())
                    .append_pair("result_overflow_mode", "break");
            }
            for (key, value) in params {
                query.append_pair(&key, &value);
            }
        }
        let mut response = self
            .client
            .post(endpoint)
            .basic_auth(&self.username, Some(&self.password))
            .body(sql)
            .send()
            .await
            .map_err(|_| DbxError::Connection("ClickHouse request failed or timed out".into()))?;
        let status = response.status();
        let exception = response
            .headers()
            .get("x-clickhouse-exception-code")
            .is_some();
        let exception_code = response
            .headers()
            .get("x-clickhouse-exception-code")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u32>().ok());
        let affected = response
            .headers()
            .get("x-clickhouse-summary")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| serde_json::from_str::<Value>(v).ok())
            .and_then(|v| {
                v["written_rows"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
            });
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| DbxError::Connection("ClickHouse response interrupted".into()))?
        {
            if bytes.len() + chunk.len() > 64 * 1024 * 1024 {
                return Err(DbxError::Query(
                    "ClickHouse response exceeds 64 MiB; narrow the query".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() || exception {
            // Server errors may repeat SQL literals or credentials. Do not expose the body.
            let code = exception_code
                .map(|code| format!(" (ClickHouse error {code})"))
                .unwrap_or_default();
            return Err(DbxError::Query(format!(
                "ClickHouse returned HTTP {}{code}; check query, credentials and permissions",
                status.as_u16()
            )));
        }
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(crate::engine::query_result(
                Vec::new(),
                Vec::new(),
                affected,
                false,
                started,
            ));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| decode_error())?;
        let meta = value["meta"].as_array().ok_or_else(decode_error)?;
        let columns = meta
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let name = m["name"].as_str().ok_or_else(decode_error)?;
                let data_type = m["type"].as_str().ok_or_else(decode_error)?;
                let mut column = ColumnInfo::result(name, i, data_type);
                column.nullable = data_type.contains("Nullable(");
                Ok(column)
            })
            .collect::<Result<Vec<_>>>()?;
        let data = value["data"].as_array().ok_or_else(decode_error)?;
        let limit = crate::engine::row_limit(options).unwrap_or(usize::MAX);
        let truncated = data.len() > limit
            || value["rows_before_limit_at_least"]
                .as_u64()
                .is_some_and(|n| n > limit as u64);
        let rows = data
            .iter()
            .take(limit)
            .map(|row| {
                let values = row.as_array().ok_or_else(decode_error)?;
                if values.len() != columns.len() {
                    return Err(decode_error());
                }
                Ok(RowData::new(
                    values
                        .iter()
                        .zip(&columns)
                        .map(|(v, c)| typed_cell(v, &c.data_type))
                        .collect(),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(crate::engine::query_result(
            columns, rows, None, truncated, started,
        ))
    }
}

fn typed_cell(value: &Value, data_type: &str) -> CellValue {
    let data_type = data_type
        .strip_prefix("Nullable(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(data_type);
    if let Some(text) = value.as_str() {
        if matches!(data_type, "Int8" | "Int16" | "Int32" | "Int64") {
            if let Ok(v) = text.parse() {
                return CellValue::Integer(v);
            }
        } else if matches!(data_type, "UInt8" | "UInt16" | "UInt32" | "UInt64")
            && let Ok(v) = text.parse()
        {
            return CellValue::Unsigned(v);
        }
    }
    super::cell(value)
}

#[async_trait]
impl Engine for ClickHouseEngine {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::ClickHouse
    }
    async fn current_database(&self) -> Result<String> {
        Ok(self.database.read().await.clone())
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(self
            .query("SHOW DATABASES", QueryOptions { max_rows: None })
            .await?
            .rows
            .iter()
            .map(|r| super::text(r, 0))
            .collect())
    }
    async fn use_database(&self, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            return Err(invalid("Database name cannot be empty"));
        }
        self.run(
            &SqlStatement::new("SELECT 1", vec![]),
            QueryOptions::default(),
            name,
        )
        .await?;
        *self.database.write().await = name.into();
        Ok(())
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let result = self.query("SELECT name, engine FROM system.tables WHERE database = currentDatabase() ORDER BY name", QueryOptions { max_rows: None }).await?;
        Ok(result
            .rows
            .iter()
            .map(|r| TableInfo {
                name: super::text(r, 0),
                schema: None,
                kind: if super::text(r, 1).contains("View") {
                    EntityKind::View
                } else {
                    EntityKind::Table
                },
            })
            .collect())
    }
    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        let database = table
            .schema
            .clone()
            .unwrap_or(self.current_database().await?);
        let result = self.query_statement(&SqlStatement::new(
            "SELECT name, type, position FROM system.columns WHERE database = ? AND table = ? ORDER BY position",
            vec![CellValue::Text(database), CellValue::Text(table.name.clone())]), QueryOptions { max_rows: None }).await?;
        Ok(result
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let data_type = super::text(r, 1);
                let mut column = ColumnInfo::result(super::text(r, 0), i, &data_type);
                column.nullable = data_type.contains("Nullable(");
                column
            })
            .collect())
    }
    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        self.query_statement(&SqlStatement::new(sql, vec![]), options)
            .await
    }
    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        self.run(statement, options, &self.current_database().await?)
            .await
    }
}

fn parameter(value: &CellValue) -> Result<(&'static str, String)> {
    Ok(match value {
        CellValue::Null => ("Nullable(String)", "\\N".into()),
        CellValue::Boolean(v) => ("Bool", u8::from(*v).to_string()),
        CellValue::Integer(v) => ("Int64", v.to_string()),
        CellValue::Unsigned(v) => ("UInt64", v.to_string()),
        CellValue::Real(v) if v.is_finite() => ("Float64", v.to_string()),
        CellValue::Text(v) => ("String", escape_parameter(v)),
        CellValue::Json(v) => ("String", escape_parameter(&v.to_string())),
        _ => {
            return Err(DbxError::Unsupported {
                operation: "binary or non-finite ClickHouse parameters".into(),
                kind: DatabaseKind::ClickHouse,
            });
        }
    })
}

fn escape_parameter(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\0', "\\0")
}

/// Translate DBX's anonymous placeholders without touching quoted SQL or comments.
fn bind(statement: &SqlStatement) -> Result<(String, Vec<(String, String)>)> {
    if statement.params.is_empty() {
        return Ok((statement.sql.clone(), Vec::new()));
    }
    let mut sql = String::new();
    let mut params = Vec::new();
    let mut chars = statement.sql.chars().peekable();
    let mut quote = None;
    let mut line = false;
    let mut block = false;
    while let Some(c) = chars.next() {
        if line {
            sql.push(c);
            if c == '\n' {
                line = false;
            }
            continue;
        }
        if block {
            sql.push(c);
            if c == '*' && chars.peek() == Some(&'/') {
                sql.push(chars.next().unwrap());
                block = false;
            }
            continue;
        }
        if let Some(q) = quote {
            sql.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    sql.push(next);
                }
            } else if c == q {
                if chars.peek() == Some(&q) {
                    sql.push(chars.next().unwrap());
                } else {
                    quote = None;
                }
            }
            continue;
        }
        if (c == '-' && chars.peek() == Some(&'-')) || c == '#' {
            line = true;
            sql.push(c);
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            block = true;
            sql.push(c);
            sql.push(chars.next().unwrap());
            continue;
        }
        if matches!(c, '\'' | '"' | '`') {
            quote = Some(c);
            sql.push(c);
            continue;
        }
        if c == '?' {
            let index = params.len();
            let value = statement
                .params
                .get(index)
                .ok_or_else(|| DbxError::Parse("Too few bound parameters".into()))?;
            let (data_type, value) = parameter(value)?;
            sql.push_str(&format!("{{dbx_{index}:{data_type}}}"));
            params.push((format!("param_dbx_{index}"), value));
        } else {
            sql.push(c);
        }
    }
    if params.len() != statement.params.len() {
        return Err(DbxError::Parse("Too many bound parameters".into()));
    }
    Ok((sql, params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn http_contract_checks_auth_parameters_and_errors_in_success_responses() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for index in 0..3 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut chunk = [0; 4096];
                loop {
                    let n = stream.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|s| s.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if data.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(data).unwrap();
                assert!(
                    request.to_ascii_lowercase().contains(
                        "authorization: basic dXNlcjpmaXh0dXJlLXNlY3JldA=="
                            .to_ascii_lowercase()
                            .as_str()
                    )
                );
                assert!(!request.lines().next().unwrap().contains("fixture-secret"));
                requests.push(request);
                let (extra, body) = if index == 2 {
                    (
                        "X-ClickHouse-Exception-Code: 60\r\n",
                        "server error repeats fixture-secret",
                    )
                } else {
                    (
                        "",
                        r#"{"meta":[{"name":"id","type":"UInt64"}],"data":[["18446744073709551615"]],"rows":1}"#,
                    )
                };
                stream.write_all(format!("HTTP/1.1 200 OK\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        let engine = ClickHouseEngine::connect(ConnectionConfig::new(
            DatabaseKind::ClickHouse,
            format!("http://user:fixture-secret@127.0.0.1:{port}/default"),
        ))
        .await
        .unwrap();
        let result = engine
            .query_statement(
                &SqlStatement::new("SELECT ? AS id", vec![CellValue::Unsigned(u64::MAX)]),
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.rows[0].values[0], CellValue::Unsigned(u64::MAX));
        let error = engine
            .query("SELECT bad", QueryOptions::default())
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("fixture-secret"));
        let requests = server.await.unwrap();
        assert!(requests[1].contains("param_dbx_0=18446744073709551615"));
        assert!(requests[1].ends_with("SELECT {dbx_0:UInt64} AS id"));
        assert!(requests[1].contains("default_format=JSONCompact"));
    }
    #[test]
    fn parameters_preserve_literals_comments_unicode_and_exact_values() {
        let statement = SqlStatement::new(
            "SELECT '?', `?`, '\\'?', ? /* ? */ -- ?\n, ? AS café, ?",
            vec![
                CellValue::Text("O'Reilly\n\\N".into()),
                CellValue::Unsigned(u64::MAX),
                CellValue::Null,
            ],
        );
        let (sql, params) = bind(&statement).unwrap();
        assert!(sql.contains("{dbx_0:String}"));
        assert!(sql.contains("{dbx_1:UInt64}"));
        assert!(sql.contains("{dbx_2:Nullable(String)}"));
        assert!(sql.contains("/* ? */ -- ?"));
        assert!(!sql.contains("O'Reilly"));
        assert_eq!(params[0].1, "O'Reilly\\n\\\\N");
        assert_eq!(params[1].1, u64::MAX.to_string());
        assert_eq!(params[2].1, "\\N");
        assert!(bind(&SqlStatement::new("SELECT ?, ?", vec![CellValue::Null])).is_err());
        assert!(bind(&SqlStatement::new("SELECT 1", vec![CellValue::Null])).is_err());
    }
    #[test]
    fn decodes_wide_integer_strings_without_rounding_or_misreading_text() {
        assert_eq!(
            typed_cell(&Value::String(u64::MAX.to_string()), "UInt64"),
            CellValue::Unsigned(u64::MAX)
        );
        assert_eq!(
            typed_cell(
                &Value::String("-9007199254740993".into()),
                "Nullable(Int64)"
            ),
            CellValue::Integer(-9007199254740993)
        );
        assert_eq!(
            typed_cell(
                &Value::String("1234567890123456.1234".into()),
                "Decimal(20, 4)"
            ),
            CellValue::Text("1234567890123456.1234".into())
        );
        assert_eq!(
            typed_cell(&Value::String("123".into()), "String"),
            CellValue::Text("123".into())
        );
    }
    #[tokio::test]
    async fn rejects_plaintext_remote_credentials_and_native_port_schemes() {
        assert!(
            ClickHouseEngine::connect(ConnectionConfig::new(
                DatabaseKind::ClickHouse,
                "clickhouse://user:secret@remote.example/default"
            ))
            .await
            .is_err()
        );
        assert!(
            ConnectionConfig::new(DatabaseKind::ClickHouse, "tcp://localhost:9000/default")
                .validate()
                .is_err()
        );
    }
}
