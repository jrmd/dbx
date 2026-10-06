//! Snowflake SQL API with separate bindings, bounded partitions and polling.
use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, EntityKind,
    QueryOptions, QueryResult, Result, RowData, SqlStatement, TableInfo, TableRef, TableStructure,
};
use async_trait::async_trait;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct SnowflakeEngine {
    client: Client,
    endpoint: url::Url,
    token: String,
    auth: &'static str,
    database: RwLock<String>,
    schema: String,
    warehouse: String,
    role: String,
    timeout: Duration,
    read_only: bool,
}

// Dropping an in-flight query (Cancel, lock, tab closure) requests server-side
// cancellation once the API has supplied a handle. A transport failure before
// the handle arrives still has an unknown outcome and is never replayed.
struct PendingStatement {
    client: Client,
    url: url::Url,
    token: String,
    auth: &'static str,
    armed: bool,
}
impl Drop for PendingStatement {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (client, url, token, auth) = (
            self.client.clone(),
            self.url.clone(),
            self.token.clone(),
            self.auth,
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = client
                    .post(url)
                    .bearer_auth(token)
                    .header("X-Snowflake-Authorization-Token-Type", auth)
                    .send()
                    .await;
            });
        }
    }
}
fn retained_bytes(result: &QueryResult) -> usize {
    result
        .rows
        .iter()
        .flat_map(|row| &row.values)
        .map(|value| match value {
            CellValue::Text(value) => value.len(),
            CellValue::Bytes(value) => value.len(),
            CellValue::Json(value) => value.to_string().len(),
            _ => 16,
        })
        .sum()
}

impl SnowflakeEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        config.validate()?;
        let url = url::Url::parse(&config.url)
            .map_err(|_| DbxError::InvalidConfig("Invalid Snowflake URL".into()))?;
        let token = url
            .password()
            .map(super::decode)
            .transpose()?
            .unwrap_or_default();
        if token.is_empty() {
            return Err(DbxError::InvalidConfig(
                "Enter a Snowflake PAT, OAuth token or key-pair JWT in the password field".into(),
            ));
        }
        let options = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        let host = url
            .host_str()
            .ok_or_else(|| DbxError::InvalidConfig("Snowflake account host is required".into()))?;
        let loopback = host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        let protocol = if loopback && options.get("protocol").is_some_and(|value| value == "http") {
            "http"
        } else {
            "https"
        };
        let mut endpoint = url::Url::parse(&format!("{protocol}://{host}/api/v2/statements"))
            .map_err(|_| DbxError::InvalidConfig("Invalid Snowflake host".into()))?;
        if let Some(port) = url.port() {
            endpoint
                .set_port(Some(port))
                .map_err(|_| DbxError::InvalidConfig("Invalid port".into()))?;
        }
        let auth = match options
            .get("auth")
            .map(|value| value.as_ref())
            .unwrap_or("pat")
        {
            "pat" => "PROGRAMMATIC_ACCESS_TOKEN",
            "oauth" => "OAUTH",
            "jwt" => "KEYPAIR_JWT",
            _ => {
                return Err(DbxError::InvalidConfig(
                    "Snowflake auth must be pat, oauth or jwt".into(),
                ));
            }
        };
        let timeout = Duration::from_millis(config.connect_timeout_ms.max(30_000));
        let engine = Self {
            client: Client::builder()
                .timeout(timeout)
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("DBX/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| DbxError::Connection("Cannot create Snowflake client".into()))?,
            endpoint,
            token,
            auth,
            database: RwLock::new(super::decode(url.path().trim_matches('/'))?),
            schema: options
                .get("schema")
                .map(ToString::to_string)
                .unwrap_or_else(|| "PUBLIC".into()),
            warehouse: options
                .get("warehouse")
                .map(ToString::to_string)
                .unwrap_or_default(),
            role: options
                .get("role")
                .map(ToString::to_string)
                .unwrap_or_default(),
            timeout,
            read_only: config.read_only,
        };
        engine.query("SELECT 1", QueryOptions::default()).await?;
        Ok(engine)
    }

    async fn request(
        &self,
        method: Method,
        url: url::Url,
        body: Option<Value>,
    ) -> Result<(StatusCode, Value)> {
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(&self.token)
            .header("X-Snowflake-Authorization-Token-Type", self.auth)
            .header("Accept", "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| DbxError::Connection("Snowflake request failed; a submitted write may have completed. Check its outcome before retrying".into()))?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| DbxError::Connection("Snowflake response interrupted".into()))?
        {
            if bytes.len().saturating_add(chunk.len()) > 64 * 1024 * 1024 {
                return Err(DbxError::Query(
                    "Snowflake response exceeds 64 MiB; narrow the query".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(DbxError::Query(format!(
                "Snowflake returned HTTP {}; check credentials, permissions and SQL",
                status.as_u16()
            )));
        }
        let response = serde_json::from_slice(&bytes)
            .map_err(|_| DbxError::Query("Invalid Snowflake response".into()))?;
        Ok((status, response))
    }

    fn handle_url(&self, handle: &str) -> Result<url::Url> {
        if !handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || handle.is_empty()
        {
            return Err(DbxError::Query("Invalid Snowflake statement handle".into()));
        }
        let mut url = self.endpoint.clone();
        url.set_path(&format!("/api/v2/statements/{handle}"));
        Ok(url)
    }

    async fn run(&self, statement: &SqlStatement, options: QueryOptions) -> Result<QueryResult> {
        if self.read_only {
            crate::protected::ensure_query(DatabaseKind::Snowflake, &statement.sql)?;
        }
        if crate::checked_split_sql_for(Some(DatabaseKind::Snowflake), &statement.sql)?.len() > 1 {
            return Err(DbxError::Parse(
                "Run Snowflake statements separately through a query tab".into(),
            ));
        }
        let started = Instant::now();
        let mut body = json!({"statement":statement.sql,"timeout":self.timeout.as_secs(),"parameters":{"MULTI_STATEMENT_COUNT":"1"}});
        let database = self.database.read().await.clone();
        for (key, value) in [
            ("database", &database),
            ("schema", &self.schema),
            ("warehouse", &self.warehouse),
            ("role", &self.role),
        ] {
            if !value.is_empty() {
                body[key] = json!(value);
            }
        }
        if !statement.params.is_empty() {
            let bindings = statement
                .params
                .iter()
                .enumerate()
                .map(|(index, value)| Ok(((index + 1).to_string(), binding(value)?)))
                .collect::<Result<serde_json::Map<String, Value>>>()?;
            body["bindings"] = Value::Object(bindings);
        }
        let (mut status, mut response) = self
            .request(Method::POST, self.endpoint.clone(), Some(body))
            .await?;
        let handle = response["statementHandle"].as_str().map(str::to_owned);
        let mut pending = handle
            .as_ref()
            .map(|handle| {
                let mut url = self.handle_url(handle)?;
                url.set_path(&format!("{}/cancel", url.path()));
                Ok::<_, DbxError>(PendingStatement {
                    client: self.client.clone(),
                    url,
                    token: self.token.clone(),
                    auth: self.auth,
                    armed: true,
                })
            })
            .transpose()?;
        while status == StatusCode::ACCEPTED {
            let handle = handle
                .as_ref()
                .ok_or_else(|| DbxError::Query("Snowflake omitted the statement handle".into()))?;
            if started.elapsed() >= self.timeout {
                let mut cancel = self.handle_url(handle)?;
                cancel.set_path(&format!("{}/cancel", cancel.path()));
                let _ = self.request(Method::POST, cancel, None).await;
                return Err(DbxError::Interrupted(format!(
                    "Snowflake statement {handle} timed out; verify its outcome before retrying"
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            (status, response) = self
                .request(Method::GET, self.handle_url(handle)?, None)
                .await?;
        }
        let mut result = decode_result(&response, options)?;
        let mut retained = retained_bytes(&result);
        if retained > 64 * 1024 * 1024 {
            return Err(DbxError::Query(
                "Snowflake results exceed 64 MiB; narrow the query".into(),
            ));
        }
        let partitions = response["resultSetMetaData"]["partitionInfo"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(1);
        let limit = crate::engine::row_limit(options);
        for partition in 1..partitions {
            if result.truncated {
                break;
            }
            let handle = handle
                .as_ref()
                .ok_or_else(|| DbxError::Query("Snowflake omitted the statement handle".into()))?;
            let mut url = self.handle_url(handle)?;
            url.query_pairs_mut()
                .append_pair("partition", &partition.to_string());
            let (_, response) = self.request(Method::GET, url, None).await?;
            let rows = response["data"]
                .as_array()
                .ok_or_else(|| DbxError::Query("Snowflake omitted partition data".into()))?;
            for row in rows {
                if limit.is_some_and(|limit| result.rows.len() >= limit) {
                    result.truncated = true;
                    break;
                }
                let row = decode_row(row, &result.columns)?;
                retained = retained.saturating_add(
                    row.values
                        .iter()
                        .map(|value| match value {
                            CellValue::Text(value) => value.len(),
                            CellValue::Bytes(value) => value.len(),
                            CellValue::Json(value) => value.to_string().len(),
                            _ => 16,
                        })
                        .sum::<usize>(),
                );
                if retained > 64 * 1024 * 1024 {
                    return Err(DbxError::Query(
                        "Snowflake results exceed 64 MiB; narrow the query".into(),
                    ));
                }
                result.rows.push(row);
            }
        }
        if let Some(pending) = &mut pending {
            pending.armed = false;
        }
        result.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(result)
    }
}

fn binding(value: &CellValue) -> Result<Value> {
    let (kind, value) = match value {
        CellValue::Null => ("TEXT", Value::Null),
        CellValue::Boolean(value) => ("BOOLEAN", json!(value.to_string())),
        CellValue::Integer(value) => ("FIXED", json!(value.to_string())),
        CellValue::Unsigned(value) => ("FIXED", json!(value.to_string())),
        CellValue::Real(value) if value.is_finite() => ("REAL", json!(value.to_string())),
        CellValue::Text(value) => ("TEXT", json!(value)),
        CellValue::Json(value) => ("TEXT", json!(value.to_string())),
        CellValue::Bytes(value) => (
            "BINARY",
            json!(
                value
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ),
        ),
        _ => {
            return Err(DbxError::Parse(
                "Cannot bind non-finite Snowflake number".into(),
            ));
        }
    };
    Ok(json!({"type":kind,"value":value}))
}

fn decode_row(row: &Value, columns: &[ColumnInfo]) -> Result<RowData> {
    let values = row
        .as_array()
        .ok_or_else(|| DbxError::Query("Invalid Snowflake row".into()))?;
    if values.len() != columns.len() {
        return Err(DbxError::Query("Snowflake row width mismatch".into()));
    }
    let values = values
        .iter()
        .zip(columns)
        .map(|(value, column)| {
            if value.is_null() {
                return CellValue::Null;
            }
            let text = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            match column.data_type.as_str() {
                "fixed" if !text.contains('.') => text
                    .parse()
                    .map(CellValue::Integer)
                    .unwrap_or(CellValue::Text(text)),
                "real" => text
                    .parse()
                    .map(CellValue::Real)
                    .unwrap_or(CellValue::Text(text)),
                "boolean" => match text.as_str() {
                    "true" | "1" => CellValue::Boolean(true),
                    "false" | "0" => CellValue::Boolean(false),
                    _ => CellValue::Text(text),
                },
                "variant" | "object" | "array" => serde_json::from_str(&text)
                    .map(CellValue::Json)
                    .unwrap_or(CellValue::Text(text)),
                _ => CellValue::Text(text),
            }
        })
        .collect();
    Ok(RowData::new(values))
}

fn decode_result(response: &Value, options: QueryOptions) -> Result<QueryResult> {
    let metadata = response["resultSetMetaData"]["rowType"]
        .as_array()
        .ok_or_else(|| DbxError::Query("Snowflake omitted result metadata".into()))?;
    let mut result = QueryResult::empty(None, 0);
    result.columns = metadata
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let mut info = ColumnInfo::result(
                column["name"].as_str().unwrap_or("column"),
                index,
                column["type"].as_str().unwrap_or("text"),
            );
            info.nullable = column["nullable"].as_bool().unwrap_or(true);
            info
        })
        .collect();
    let rows = response["data"]
        .as_array()
        .ok_or_else(|| DbxError::Query("Snowflake omitted result data".into()))?;
    let limit = crate::engine::row_limit(options);
    for row in rows {
        if limit.is_some_and(|limit| result.rows.len() >= limit) {
            result.truncated = true;
            break;
        }
        result.rows.push(decode_row(row, &result.columns)?);
    }
    Ok(result)
}

#[async_trait]
impl Engine for SnowflakeEngine {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::Snowflake
    }
    fn is_read_only(&self) -> bool {
        self.read_only
    }
    async fn current_database(&self) -> Result<String> {
        Ok(self.database.read().await.clone())
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        let result = self
            .query("SHOW DATABASES", QueryOptions::default())
            .await?;
        let index = result
            .columns
            .iter()
            .position(|column| column.name.eq_ignore_ascii_case("name"))
            .ok_or_else(|| DbxError::Query("Missing database names".into()))?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| row.values.get(index))
            .map(ToString::to_string)
            .collect())
    }
    async fn use_database(&self, name: &str) -> Result<()> {
        crate::quote_identifier(self.kind(), name)?;
        let statement = format!(
            "USE DATABASE {}",
            crate::quote_identifier(self.kind(), name)?
        );
        self.query(&statement, QueryOptions::default()).await?;
        *self.database.write().await = name.to_owned();
        Ok(())
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let result = self.query("SELECT TABLE_NAME, TABLE_SCHEMA, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA <> 'INFORMATION_SCHEMA' ORDER BY TABLE_SCHEMA,TABLE_NAME", QueryOptions { max_rows: None }).await?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| {
                Some(TableInfo {
                    name: row.values.first()?.to_string(),
                    schema: Some(row.values.get(1)?.to_string()),
                    kind: if row.values.get(2)?.to_string() == "VIEW" {
                        EntityKind::View
                    } else {
                        EntityKind::Table
                    },
                })
            })
            .collect())
    }
    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        let statement = SqlStatement::new(
            "SELECT COLUMN_NAME, DATA_TYPE, IS_NULLABLE, COLUMN_DEFAULT FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_NAME=? AND TABLE_SCHEMA=? ORDER BY ORDINAL_POSITION",
            vec![
                CellValue::Text(table.name.clone()),
                CellValue::Text(table.schema.clone().unwrap_or_else(|| self.schema.clone())),
            ],
        );
        let result = self
            .run(&statement, QueryOptions { max_rows: None })
            .await?;
        Ok(result
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let mut column =
                    ColumnInfo::result(row.values[0].to_string(), index, row.values[1].to_string());
                column.nullable = row.values[2].to_string() == "YES";
                column.default_value =
                    (!matches!(row.values[3], CellValue::Null)).then(|| row.values[3].to_string());
                column
            })
            .collect())
    }
    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let columns = self.describe_table(table).await?;
        let result = self
            .run(
                &SqlStatement::new(
                    "SELECT GET_DDL('TABLE', ?)",
                    vec![CellValue::Text(crate::sql::quote_table(
                        self.kind(),
                        table,
                    )?)],
                ),
                QueryOptions::default(),
            )
            .await?;
        let definition = result
            .rows
            .first()
            .and_then(|row| row.values.first())
            .map(ToString::to_string);
        Ok(TableStructure {
            columns,
            definition,
            ..Default::default()
        })
    }
    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        self.run(&SqlStatement::new(sql, Vec::new()), options).await
    }
    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        self.run(statement, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn sql_api_binds_polls_fetches_partitions_and_keeps_credentials_on_origin() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, reply) in [
                (
                    "200 OK",
                    json!({"resultSetMetaData":{"rowType":[{"name":"ready","type":"fixed"}]},"data":[["1"]]}),
                ),
                ("202 Accepted", json!({"statementHandle":"fixture-handle"})),
                (
                    "200 OK",
                    json!({"statementHandle":"fixture-handle","resultSetMetaData":{"rowType":[{"name":"value","type":"text"}],"partitionInfo":[{},{}]},"data":[["first"]]}),
                ),
                ("200 OK", json!({"data":[["second"]]})),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 4096];
                let (header_end, length) = loop {
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length: ")
                                    .and_then(|value| value.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_end + length {
                    let count = socket.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                }
                let header = String::from_utf8_lossy(&bytes[..header_end]).to_string();
                assert!(
                    header
                        .to_ascii_lowercase()
                        .contains("authorization: bearer fixture-secret")
                );
                assert!(
                    header.to_ascii_lowercase().contains(
                        "x-snowflake-authorization-token-type: programmatic_access_token"
                    )
                );
                let body = if length > 0 {
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
                } else {
                    Value::Null
                };
                requests.push((header, body));
                let reply = reply.to_string();
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",reply.len()).as_bytes()).await.unwrap();
            }
            requests
        });
        let engine = SnowflakeEngine::connect(ConnectionConfig::new(
            DatabaseKind::Snowflake,
            format!(
                "snowflake://user:fixture-secret@127.0.0.1:{port}/DB?protocol=http&warehouse=WH"
            ),
        ))
        .await
        .unwrap();
        let result = engine
            .query_statement(
                &SqlStatement::new(
                    "SELECT ?",
                    vec![CellValue::Text("'; DROP TABLE private_data; --".into())],
                ),
                QueryOptions { max_rows: None },
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[1].values[0], CellValue::Text("second".into()));
        let requests = task.await.unwrap();
        assert_eq!(requests[1].1["statement"], "SELECT ?");
        assert_eq!(
            requests[1].1["bindings"]["1"]["value"],
            "'; DROP TABLE private_data; --"
        );
        assert_eq!(requests[1].1["database"], "DB");
        assert!(
            requests[2]
                .0
                .starts_with("GET /api/v2/statements/fixture-handle ")
        );
        assert!(
            requests[3]
                .0
                .starts_with("GET /api/v2/statements/fixture-handle?partition=1 ")
        );
        assert!(engine.handle_url("https://another-host").is_err());
    }

    #[test]
    fn binds_sql_looking_text_and_preserves_large_decimal_values() {
        assert_eq!(
            binding(&CellValue::Text("1; DROP TABLE x".into())).unwrap(),
            json!({"type":"TEXT","value":"1; DROP TABLE x"})
        );
        let response = json!({"resultSetMetaData":{"rowType":[{"name":"n","type":"fixed"}]},"data":[["999999999999999999999999999"],[null]]});
        let result = decode_result(&response, QueryOptions::default()).unwrap();
        assert_eq!(
            result.rows[0].values[0],
            CellValue::Text("999999999999999999999999999".into())
        );
        assert_eq!(result.rows[1].values[0], CellValue::Null);
    }
}
