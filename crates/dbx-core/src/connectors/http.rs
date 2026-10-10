use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, EntityKind,
    QueryOptions, QueryResult, Result, RowData, SqlStatement, TableInfo, TableRef,
};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use url::Url;

pub(super) struct HttpEngine {
    kind: DatabaseKind,
    client: Client,
    endpoint: Url,
    token: String,
    google_auth: Option<std::sync::Arc<dyn gcp_auth::TokenProvider>>,
    username: String,
    database: RwLock<String>,
    location: Option<String>,
    timeout: Duration,
}
fn invalid(message: &str) -> DbxError {
    DbxError::InvalidConfig(message.into())
}
fn decode_error() -> DbxError {
    DbxError::Decode("Invalid database API response".into())
}
fn array(value: &Value) -> Result<&Vec<Value>> {
    value.as_array().ok_or_else(decode_error)
}

impl HttpEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        let mut url = Url::parse(&config.url).map_err(|_| invalid("Invalid database URL"))?;
        let token = url
            .password()
            .map(super::decode)
            .transpose()?
            .unwrap_or_default();
        let username = super::decode(url.username())?;
        let location = url
            .query_pairs()
            .find(|(k, _)| k == "location")
            .map(|(_, v)| v.into_owned());
        let host = url
            .host_str()
            .ok_or_else(|| invalid("Database endpoint is required"))?
            .to_owned();
        let database = super::decode(url.path().trim_matches('/'))?;
        let _ = url.set_password(None);
        let _ = url.set_username("");
        let endpoint = match config.kind {
            DatabaseKind::CloudflareD1 => {
                if token.is_empty() || database.is_empty() {
                    return Err(invalid(
                        "D1 requires an account ID, database ID and API token in the URL password",
                    ));
                }
                let mut endpoint = Url::parse("https://api.cloudflare.com/client/v4/").unwrap();
                endpoint
                    .path_segments_mut()
                    .unwrap()
                    .extend(["accounts", &host, "d1", "database"]);
                endpoint
            }
            DatabaseKind::BigQuery => {
                let mut endpoint =
                    Url::parse("https://bigquery.googleapis.com/bigquery/v2/").unwrap();
                endpoint
                    .path_segments_mut()
                    .unwrap()
                    .extend(["projects", &host]);
                endpoint
            }
            DatabaseKind::Turso => {
                if matches!(url.scheme(), "libsql" | "turso") {
                    // Url refuses a non-special to special scheme change.
                    url = Url::parse(&format!(
                        "https://{}",
                        url.as_str().split_once("://").unwrap().1
                    ))
                    .map_err(|_| invalid("Invalid Turso URL"))?;
                }
                if url.path() != "/" && !url.path().is_empty() {
                    return Err(invalid("Turso URL must point to the database host"));
                }
                url.set_query(None);
                url
            }
            DatabaseKind::Elasticsearch => {
                url.set_query(None);
                url
            }
            _ => return Err(invalid("Not an HTTP database connector")),
        };
        if endpoint.scheme() == "http"
            && (!token.is_empty() || !username.is_empty())
            && !matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
        {
            return Err(invalid("Use HTTPS when sending database credentials"));
        }
        let timeout = Duration::from_millis(config.connect_timeout_ms);
        let google_auth = if config.kind == DatabaseKind::BigQuery && token.is_empty() {
            Some(tokio::time::timeout(timeout, gcp_auth::provider()).await
                .map_err(|_| invalid("Google credentials discovery timed out"))?
                .map_err(|_| invalid("Configure Google Application Default Credentials or provide an OAuth access token"))?)
        } else {
            None
        };
        let client = Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| invalid("Cannot create HTTP client"))?;
        let engine = Self {
            kind: config.kind,
            client,
            endpoint,
            token,
            google_auth,
            username,
            database: RwLock::new(database),
            location,
            timeout,
        };
        match engine.kind {
            DatabaseKind::Elasticsearch => {
                engine
                    .request(Method::GET, engine.endpoint.clone(), None)
                    .await?;
            }
            DatabaseKind::BigQuery => {
                engine
                    .request(Method::GET, engine.url(&["datasets"]), None)
                    .await?;
            }
            _ => {
                engine.query("SELECT 1", QueryOptions::default()).await?;
            }
        }
        Ok(engine)
    }
    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .extend(segments);
        url
    }
    async fn request(&self, method: Method, url: Url, body: Option<Value>) -> Result<Value> {
        let mut request = self.client.request(method, url);
        if self.kind == DatabaseKind::Elasticsearch {
            if !self.username.is_empty() {
                request = request.basic_auth(&self.username, Some(&self.token));
            } else if !self.token.is_empty() {
                request = request.header("Authorization", format!("ApiKey {}", self.token));
            }
        } else if let Some(provider) = &self.google_auth {
            let token = tokio::time::timeout(
                self.timeout,
                provider.token(&["https://www.googleapis.com/auth/cloud-platform"]),
            )
            .await
            .map_err(|_| DbxError::Connection("Google token refresh timed out".into()))?
            .map_err(|_| {
                DbxError::Connection(
                    "Google token refresh failed; check ADC credentials and permissions".into(),
                )
            })?;
            request = request.bearer_auth(token.as_str());
        } else if !self.token.is_empty() {
            request = request.bearer_auth(&self.token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(|_| {
            DbxError::Connection(format!("{} request failed or timed out", self.kind))
        })?;
        let status = response.status();
        // Bound the response before decoding. A provider may ignore maxResults.
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| DbxError::Connection("Database response was interrupted".into()))?
        {
            if bytes.len() + chunk.len() > 64 * 1024 * 1024 {
                return Err(DbxError::Query(
                    "Database response exceeds 64 MiB; narrow the query".into(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(DbxError::Query(format!(
                "{} API returned HTTP {}; check credentials, permissions and query",
                self.kind,
                status.as_u16()
            )));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| decode_error())?;
        if value["success"] == false
            || value.get("error").is_some()
            || value
                .get("errors")
                .is_some_and(|e| e.as_array().is_some_and(|a| !a.is_empty()))
        {
            return Err(DbxError::Query(format!(
                "{} rejected the request; check the query and permissions",
                self.kind
            )));
        }
        Ok(value)
    }
    async fn sql_query(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        if matches!(self.kind, DatabaseKind::Turso | DatabaseKind::CloudflareD1) {
            crate::script::ensure_single_statement(
                self.kind,
                &statement.sql,
                &self.kind.to_string(),
            )?;
        }
        match self.kind {
            DatabaseKind::Turso => {
                let args = statement
                    .params
                    .iter()
                    .map(turso_arg)
                    .collect::<Result<Vec<_>>>()?;
                let response=self.request(Method::POST,self.url(&["v2","pipeline"]),Some(json!({"requests":[{"type":"execute","stmt":{"sql":statement.sql,"args":args,"want_rows":true}},{"type":"close"}]}))).await?;
                let result = &response["results"][0];
                if result["type"] != "ok" {
                    return Err(DbxError::Query("Turso rejected the SQL statement".into()));
                }
                turso_result(&result["response"]["result"], options, started)
            }
            DatabaseKind::CloudflareD1 => {
                let database = self.database.read().await.clone();
                let (sql, params) = super::d1_binding::bind(statement)?;
                let response = self
                    .request(
                        Method::POST,
                        self.url(&[&database, "query"]),
                        Some(json!({"sql":sql,"params":params})),
                    )
                    .await?;
                let results = array(&response["result"])?;
                if results.len() != 1 {
                    return Err(DbxError::Query("Run one D1 SQL statement at a time".into()));
                }
                if results[0]["success"] == false {
                    return Err(DbxError::Query("D1 rejected the SQL statement".into()));
                }
                let mut result =
                    super::json_rows(array(&results[0]["results"])?.clone(), options, started);
                result.rows_affected = results[0]["meta"]["changes"].as_u64();
                Ok(result)
            }
            DatabaseKind::BigQuery => self.bigquery(statement, options, started).await,
            _ => unreachable!(),
        }
    }
    async fn bigquery(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
        started: Instant,
    ) -> Result<QueryResult> {
        let database = self.database.read().await.clone();
        let project = self
            .endpoint
            .path_segments()
            .unwrap()
            .next_back()
            .unwrap()
            .to_owned();
        let mut body = json!({"query":statement.sql,"useLegacySql":false,"timeoutMs":1000,"maxResults":crate::engine::row_limit(options).unwrap_or(10_000).saturating_add(1),"requestId":format!("dbx-{}",uuid_like())});
        if !database.is_empty() {
            body["defaultDataset"] = json!({"projectId":project,"datasetId":database});
        }
        if let Some(location) = &self.location {
            body["location"] = json!(location);
        }
        if !statement.params.is_empty() {
            body["parameterMode"] = json!("POSITIONAL");
            body["queryParameters"] = Value::Array(
                statement
                    .params
                    .iter()
                    .map(bq_arg)
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        let mut response = self
            .request(Method::POST, self.url(&["queries"]), Some(body))
            .await?;
        let job = response["jobReference"].clone();
        let job_id = job["jobId"].as_str().map(str::to_owned);
        let deadline = tokio::time::Instant::now() + self.timeout;
        while response["jobComplete"] != true {
            if tokio::time::Instant::now() >= deadline {
                return Err(DbxError::Query(format!(
                    "BigQuery job {} is still running; check its status in Google Cloud before retrying",
                    job_id.as_deref().unwrap_or("unknown")
                )));
            }
            let id = job_id.as_deref().ok_or_else(decode_error)?;
            let mut url = self.url(&["queries", id]);
            url.query_pairs_mut().append_pair("timeoutMs", "1000");
            if let Some(location) = job["location"].as_str().or(self.location.as_deref()) {
                url.query_pairs_mut().append_pair("location", location);
            }
            response = self.request(Method::GET, url, None).await?;
        }
        let fields = response["schema"]["fields"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let columns = fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut c = super::column(
                    f["name"].as_str().unwrap_or("value"),
                    f["type"].as_str().unwrap_or("JSON"),
                    i,
                );
                c.nullable = f["mode"] != "REQUIRED";
                c
            })
            .collect();
        let limit = crate::engine::row_limit(options).unwrap_or(usize::MAX);
        let mut rows = Vec::new();
        let mut truncated = false;
        loop {
            for row in response["rows"].as_array().into_iter().flatten() {
                if rows.len() >= limit {
                    truncated = true;
                    break;
                }
                let entries = array(&row["f"])?;
                if entries.len() != fields.len() {
                    return Err(decode_error());
                }
                rows.push(RowData::new(
                    entries
                        .iter()
                        .zip(&fields)
                        .map(|(entry, f)| bq_cell(&entry["v"], f))
                        .collect::<Result<Vec<_>>>()?,
                ));
            }
            let page = response["pageToken"].as_str().filter(|s| !s.is_empty());
            if rows.len() >= limit {
                truncated |= page.is_some();
                break;
            }
            let Some(page) = page else {
                break;
            };
            let id = job_id.as_deref().ok_or_else(decode_error)?;
            let mut url = self.url(&["queries", id]);
            url.query_pairs_mut()
                .append_pair("pageToken", page)
                .append_pair(
                    "maxResults",
                    &limit
                        .saturating_sub(rows.len())
                        .saturating_add(1)
                        .min(10_000)
                        .to_string(),
                );
            if let Some(location) = job["location"].as_str().or(self.location.as_deref()) {
                url.query_pairs_mut().append_pair("location", location);
            }
            response = self.request(Method::GET, url, None).await?;
        }
        Ok(crate::engine::query_result(
            columns,
            rows,
            response["numDmlAffectedRows"]
                .as_str()
                .and_then(|n| n.parse().ok()),
            truncated,
            started,
        ))
    }
}
fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}
fn turso_arg(value: &CellValue) -> Result<Value> {
    Ok(match value {
        CellValue::Null => json!({"type":"null"}),
        CellValue::Boolean(v) => json!({"type":"integer","value":if *v {"1"} else {"0"}}),
        CellValue::Integer(v) => json!({"type":"integer","value":v.to_string()}),
        CellValue::Unsigned(v) => {
            json!({"type":"integer","value":i64::try_from(*v).map_err(|_|invalid("Integer exceeds SQLite range"))?.to_string()})
        }
        CellValue::Real(v) if v.is_finite() => json!({"type":"float","value":v}),
        CellValue::Real(_) => return Err(invalid("Non-finite parameter")),
        CellValue::Text(v) => json!({"type":"text","value":v}),
        CellValue::Json(v) => json!({"type":"text","value":v.to_string()}),
        CellValue::Bytes(v) => json!({"type":"blob","base64":STANDARD.encode(v)}),
    })
}
fn turso_result(value: &Value, options: QueryOptions, started: Instant) -> Result<QueryResult> {
    let columns = array(&value["cols"])?
        .iter()
        .enumerate()
        .map(|(i, c)| {
            super::column(
                c["name"].as_str().unwrap_or("value"),
                c["decltype"].as_str().unwrap_or("unknown"),
                i,
            )
        })
        .collect::<Vec<_>>();
    let values = array(&value["rows"])?;
    let limit = crate::engine::row_limit(options).unwrap_or(usize::MAX);
    let rows = values
        .iter()
        .take(limit)
        .map(|row| {
            array(row)?
                .iter()
                .map(|v| {
                    Ok(match v["type"].as_str() {
                        Some("null") => CellValue::Null,
                        Some("integer") => CellValue::Integer(
                            v["value"]
                                .as_str()
                                .ok_or_else(decode_error)?
                                .parse()
                                .map_err(|_| decode_error())?,
                        ),
                        Some("float") => {
                            CellValue::Real(v["value"].as_f64().ok_or_else(decode_error)?)
                        }
                        Some("text") => {
                            CellValue::Text(v["value"].as_str().ok_or_else(decode_error)?.into())
                        }
                        Some("blob") => CellValue::Bytes(
                            STANDARD
                                .decode(v["base64"].as_str().ok_or_else(decode_error)?)
                                .map_err(|_| decode_error())?,
                        ),
                        _ => return Err(decode_error()),
                    })
                })
                .collect::<Result<Vec<_>>>()
                .map(RowData::new)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(crate::engine::query_result(
        columns,
        rows,
        value["affected_row_count"].as_u64(),
        values.len() > limit,
        started,
    ))
}
fn bq_arg(value: &CellValue) -> Result<Value> {
    let (kind, value) = match value {
        CellValue::Null => ("STRING", Value::Null),
        CellValue::Boolean(v) => ("BOOL", json!(v.to_string())),
        CellValue::Integer(v) => ("INT64", json!(v.to_string())),
        CellValue::Unsigned(v) => (
            "INT64",
            json!(
                i64::try_from(*v)
                    .map_err(|_| invalid("Integer exceeds BigQuery INT64 range"))?
                    .to_string()
            ),
        ),
        CellValue::Real(v) if v.is_finite() => ("FLOAT64", json!(v.to_string())),
        CellValue::Text(v) => ("STRING", json!(v)),
        CellValue::Bytes(v) => ("BYTES", json!(STANDARD.encode(v))),
        CellValue::Json(v) => ("JSON", json!(v.to_string())),
        _ => return Err(invalid("Non-finite parameter")),
    };
    Ok(json!({"parameterType":{"type":kind},"parameterValue":{"value":value}}))
}
fn bq_cell(value: &Value, field: &Value) -> Result<CellValue> {
    if value.is_null() {
        return Ok(CellValue::Null);
    }
    if field["mode"] == "REPEATED" || matches!(field["type"].as_str(), Some("RECORD" | "STRUCT")) {
        return Ok(CellValue::Json(value.clone()));
    }
    let text = value.as_str().ok_or_else(decode_error)?;
    Ok(match field["type"].as_str() {
        Some("INTEGER" | "INT64") => CellValue::Integer(text.parse().map_err(|_| decode_error())?),
        Some("FLOAT" | "FLOAT64") => CellValue::Real(text.parse().map_err(|_| decode_error())?),
        Some("BOOLEAN" | "BOOL") => CellValue::Boolean(text.parse().map_err(|_| decode_error())?),
        Some("BYTES") => CellValue::Bytes(STANDARD.decode(text).map_err(|_| decode_error())?),
        Some("JSON") => CellValue::Json(serde_json::from_str(text).map_err(|_| decode_error())?),
        _ => CellValue::Text(text.into()),
    })
}

#[async_trait]
impl Engine for HttpEngine {
    fn kind(&self) -> DatabaseKind {
        self.kind
    }
    async fn current_database(&self) -> Result<String> {
        Ok(self.database.read().await.clone())
    }
    async fn list_databases(&self) -> Result<Vec<String>> {
        if self.kind != DatabaseKind::BigQuery {
            return Ok(vec![self.current_database().await?]);
        }
        let mut databases = Vec::new();
        let mut token = String::new();
        loop {
            let mut url = self.url(&["datasets"]);
            if !token.is_empty() {
                url.query_pairs_mut().append_pair("pageToken", &token);
            }
            let response = self.request(Method::GET, url, None).await?;
            for d in response["datasets"].as_array().into_iter().flatten() {
                databases.push(
                    d["datasetReference"]["datasetId"]
                        .as_str()
                        .ok_or_else(decode_error)?
                        .into(),
                );
            }
            token = response["nextPageToken"]
                .as_str()
                .unwrap_or_default()
                .into();
            if token.is_empty() {
                break;
            }
        }
        Ok(databases)
    }
    async fn use_database(&self, name: &str) -> Result<()> {
        if self.kind != DatabaseKind::BigQuery {
            return Err(DbxError::Unsupported {
                operation: "use_database".into(),
                kind: self.kind,
            });
        }
        self.request(Method::GET, self.url(&["datasets", name]), None)
            .await?;
        *self.database.write().await = name.into();
        Ok(())
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        if self.kind == DatabaseKind::Elasticsearch {
            let mut url = self.url(&["_cat", "indices"]);
            url.query_pairs_mut()
                .append_pair("format", "json")
                .append_pair("h", "index");
            let response = self.request(Method::GET, url, None).await?;
            return array(&response)?
                .iter()
                .map(|v| {
                    Ok(TableInfo {
                        name: v["index"].as_str().ok_or_else(decode_error)?.into(),
                        schema: None,
                        kind: EntityKind::Collection,
                    })
                })
                .collect();
        }
        if self.kind == DatabaseKind::BigQuery {
            let db = self.database.read().await.clone();
            if db.is_empty() {
                return Ok(Vec::new());
            }
            let mut tables = Vec::new();
            let mut token = String::new();
            loop {
                let mut url = self.url(&["datasets", &db, "tables"]);
                if !token.is_empty() {
                    url.query_pairs_mut().append_pair("pageToken", &token);
                }
                let response = self.request(Method::GET, url, None).await?;
                for t in response["tables"].as_array().into_iter().flatten() {
                    tables.push(TableInfo {
                        name: t["tableReference"]["tableId"]
                            .as_str()
                            .ok_or_else(decode_error)?
                            .into(),
                        schema: None,
                        kind: if t["type"] == "VIEW" {
                            EntityKind::View
                        } else {
                            EntityKind::Table
                        },
                    });
                }
                token = response["nextPageToken"]
                    .as_str()
                    .unwrap_or_default()
                    .into();
                if token.is_empty() {
                    break;
                }
            }
            return Ok(tables);
        }
        let result=self.query("SELECT name, type FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY name",QueryOptions {max_rows:None}).await?;
        Ok(result
            .rows
            .iter()
            .map(|r| TableInfo {
                name: super::text(r, 0),
                schema: None,
                kind: if super::text(r, 1) == "view" {
                    EntityKind::View
                } else {
                    EntityKind::Table
                },
            })
            .collect())
    }
    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        if self.kind == DatabaseKind::BigQuery {
            let db = self.database.read().await.clone();
            let response = self
                .request(
                    Method::GET,
                    self.url(&["datasets", &db, "tables", &table.name]),
                    None,
                )
                .await?;
            return array(&response["schema"]["fields"])?
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let mut c = super::column(
                        f["name"].as_str().ok_or_else(decode_error)?,
                        f["type"].as_str().ok_or_else(decode_error)?,
                        i,
                    );
                    c.nullable = f["mode"] != "REQUIRED";
                    c.default_value = f["defaultValueExpression"].as_str().map(str::to_owned);
                    Ok(c)
                })
                .collect();
        }
        if self.kind == DatabaseKind::Elasticsearch {
            return Ok(self
                .query(
                    &super::browse_command(
                        self.kind,
                        table,
                        Some(crate::Page {
                            limit: 1,
                            offset: 0,
                        }),
                    )?,
                    QueryOptions { max_rows: Some(1) },
                )
                .await?
                .columns);
        }
        let result = self
            .query(
                &format!("PRAGMA table_info('{}')", table.name.replace('\'', "''")),
                QueryOptions { max_rows: None },
            )
            .await?;
        // D1 JSON object key order differs from SQLite's positional PRAGMA.
        let index = |name: &str| {
            result
                .columns
                .iter()
                .position(|c| c.name == name)
                .ok_or_else(decode_error)
        };
        let default = index("dflt_value")?;
        let (name, ty, notnull, pk) = (
            index("name")?,
            index("type")?,
            index("notnull")?,
            index("pk")?,
        );
        Ok(result
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut c = super::column(super::text(r, name), super::text(r, ty), i);
                c.primary_key = super::text(r, pk) != "0";
                c.nullable = super::text(r, notnull) == "0" && !c.primary_key;
                c.default_value = r
                    .values
                    .get(default)
                    .filter(|value| !matches!(value, CellValue::Null))
                    .map(ToString::to_string);
                c
            })
            .collect())
    }
    async fn table_structure(&self, table: &TableRef) -> Result<crate::TableStructure> {
        let mut columns = self.describe_table(table).await?;
        if self.kind == DatabaseKind::BigQuery {
            let db = self.database.read().await.clone();
            let metadata = self
                .request(
                    Method::GET,
                    self.url(&["datasets", &db, "tables", &table.name]),
                    None,
                )
                .await?;
            let constraints = &metadata["tableConstraints"];
            for column in &mut columns {
                column.primary_key = constraints["primaryKey"]["columns"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|key| key.as_str() == Some(&column.name)));
            }
            let foreign_keys = constraints["foreignKeys"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|key| {
                    let references = array(&key["columnReferences"])?;
                    let dataset = key["referencedTable"]["datasetId"]
                        .as_str()
                        .ok_or_else(decode_error)?;
                    Ok(crate::ForeignKeyInfo {
                        constraint_name: key["name"].as_str().map(str::to_owned),
                        columns: references
                            .iter()
                            .map(|r| {
                                r["referencingColumn"]
                                    .as_str()
                                    .map(str::to_owned)
                                    .ok_or_else(decode_error)
                            })
                            .collect::<Result<Vec<_>>>()?,
                        referenced_schema: (dataset != db).then(|| dataset.into()),
                        referenced_table: key["referencedTable"]["tableId"]
                            .as_str()
                            .ok_or_else(decode_error)?
                            .into(),
                        referenced_columns: references
                            .iter()
                            .map(|r| {
                                r["referencedColumn"]
                                    .as_str()
                                    .map(str::to_owned)
                                    .ok_or_else(decode_error)
                            })
                            .collect::<Result<Vec<_>>>()?,
                        on_update: None,
                        on_delete: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(crate::TableStructure {
                columns,
                foreign_keys,
                definition: metadata["view"]["query"].as_str().map(str::to_owned),
                ..Default::default()
            });
        }
        if !matches!(self.kind, DatabaseKind::Turso | DatabaseKind::CloudflareD1) {
            return Ok(crate::TableStructure {
                columns,
                foreign_keys: Vec::new(),
                ..Default::default()
            });
        }
        let definition_result = self
            .query_statement(
                &SqlStatement::new(
                    "SELECT sql FROM sqlite_master WHERE name=? AND type IN ('table','view')",
                    vec![CellValue::Text(table.name.clone())],
                ),
                QueryOptions::default(),
            )
            .await?;
        let definition = definition_result
            .rows
            .first()
            .and_then(|row| row.values.first())
            .filter(|value| !matches!(value, CellValue::Null))
            .map(ToString::to_string);
        let index_result = self
            .query(
                &format!("PRAGMA index_list('{}')", table.name.replace('\'', "''")),
                QueryOptions { max_rows: None },
            )
            .await?;
        let column_index = |name: &str| {
            index_result
                .columns
                .iter()
                .position(|column| column.name == name)
                .ok_or_else(decode_error)
        };
        let checks = definition
            .as_deref()
            .map(crate::schema_objects::sqlite_checks)
            .unwrap_or_default();
        let mut indexes = Vec::new();
        if !index_result.rows.is_empty() {
            let name_index = column_index("name")?;
            let unique_index = column_index("unique")?;
            let origin_index = column_index("origin")?;
            for row in &index_result.rows {
                let name = super::text(row, name_index);
                let parts = self
                    .query(
                        &format!("PRAGMA index_info('{}')", name.replace('\'', "''")),
                        QueryOptions { max_rows: None },
                    )
                    .await?;
                let part_index = parts
                    .columns
                    .iter()
                    .position(|column| column.name == "name");
                let columns = parts
                    .rows
                    .iter()
                    .filter_map(|row| part_index.and_then(|index| row.values.get(index)))
                    .filter(|value| !matches!(value, CellValue::Null))
                    .map(ToString::to_string)
                    .collect();
                let source = self
                    .query_statement(
                        &SqlStatement::new(
                            "SELECT sql FROM sqlite_master WHERE type='index' AND name=?",
                            vec![CellValue::Text(name.clone())],
                        ),
                        QueryOptions::default(),
                    )
                    .await?;
                let definition = source
                    .rows
                    .first()
                    .and_then(|row| row.values.first())
                    .filter(|value| !matches!(value, CellValue::Null))
                    .map(ToString::to_string);
                indexes.push(crate::IndexInfo {
                    name,
                    columns,
                    unique: super::text(row, unique_index) == "1",
                    primary: super::text(row, origin_index) == "pk",
                    method: None,
                    predicate: None,
                    definition,
                });
            }
        }
        let result = self
            .query(
                &format!(
                    "PRAGMA foreign_key_list('{}')",
                    table.name.replace('\'', "''")
                ),
                QueryOptions { max_rows: None },
            )
            .await?;
        if result.rows.is_empty() {
            return Ok(crate::TableStructure {
                columns,
                indexes,
                definition,
                checks,
                foreign_keys: Vec::new(),
            });
        }
        let index = |name: &str| {
            result
                .columns
                .iter()
                .position(|c| c.name == name)
                .ok_or_else(decode_error)
        };
        let (id, seq, target, from, to, update, delete) = (
            index("id")?,
            index("seq")?,
            index("table")?,
            index("from")?,
            index("to")?,
            index("on_update")?,
            index("on_delete")?,
        );
        let mut rows = result.rows;
        rows.sort_by_key(|r| {
            (
                super::text(r, id).parse::<u64>().unwrap_or(0),
                super::text(r, seq).parse::<u64>().unwrap_or(0),
            )
        });
        let mut groups = std::collections::BTreeMap::new();
        for row in rows {
            let key = super::text(&row, id);
            let fk = groups.entry(key).or_insert_with(|| crate::ForeignKeyInfo {
                constraint_name: None,
                columns: Vec::new(),
                referenced_schema: None,
                referenced_table: super::text(&row, target),
                referenced_columns: Vec::new(),
                on_update: crate::ReferentialAction::from_metadata(&super::text(&row, update)),
                on_delete: crate::ReferentialAction::from_metadata(&super::text(&row, delete)),
            });
            fk.columns.push(super::text(&row, from));
            let target_column = super::text(&row, to);
            if target_column != "NULL" {
                fk.referenced_columns.push(target_column);
            }
        }
        Ok(crate::TableStructure {
            columns,
            foreign_keys: groups.into_values().collect(),
            indexes,
            checks,
            definition,
        })
    }
    async fn query(&self, command: &str, options: QueryOptions) -> Result<QueryResult> {
        if self.kind != DatabaseKind::Elasticsearch {
            return self
                .sql_query(&SqlStatement::new(command, Vec::new()), options)
                .await;
        }
        let started = Instant::now();
        let (line, body) = command.split_once('\n').unwrap_or((command, ""));
        let (method, path) = line.trim().split_once(' ').ok_or_else(|| {
            DbxError::Parse("Use METHOD /path followed by an optional JSON body".into())
        })?;
        let method = Method::from_bytes(method.as_bytes())
            .map_err(|_| DbxError::Parse("Invalid HTTP method".into()))?;
        if !path.starts_with('/') || path.starts_with("//") || path.contains('#') {
            return Err(DbxError::Parse(
                "Use a relative Elasticsearch API path starting with /".into(),
            ));
        }
        let mut url = self.endpoint.clone();
        let (path, query) = path
            .split_once('?')
            .map_or((path, None), |(p, q)| (p, Some(q)));
        url.set_path(&format!(
            "{}{}",
            self.endpoint.path().trim_end_matches('/'),
            path
        ));
        url.set_query(query);
        let body = if body.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str(body)
                    .map_err(|_| DbxError::Parse("Invalid JSON body".into()))?,
            )
        };
        let response = self.request(method, url, body).await?;
        let values = if let Some(hits) = response["hits"]["hits"].as_array() {
            hits.iter()
                .map(|hit| {
                    let mut value = hit["_source"].as_object().cloned().unwrap_or_default();
                    value.insert("_id".into(), hit["_id"].clone());
                    value.insert("_index".into(), hit["_index"].clone());
                    Value::Object(value)
                })
                .collect()
        } else if let Some(values) = response.as_array() {
            values.clone()
        } else {
            vec![response.clone()]
        };
        let mut result = super::json_rows(values, options, started);
        let total = response["hits"]["total"]["value"]
            .as_u64()
            .or_else(|| response["hits"]["total"].as_u64());
        result.truncated |= total.is_some_and(|total| total > result.rows.len() as u64);
        Ok(result)
    }
    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        if self.kind == DatabaseKind::Elasticsearch {
            if !statement.params.is_empty() {
                return Err(invalid(
                    "Elasticsearch uses JSON request bodies, not SQL parameters",
                ));
            }
            self.query(&statement.sql, options).await
        } else {
            self.sql_query(statement, options).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn fixture(
        kind: DatabaseKind,
        replies: Vec<Value>,
    ) -> (HttpEngine, tokio::task::JoinHandle<Vec<(String, Value)>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut chunk = [0; 4096];
                let (header_end, content_length) = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..i]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|s| s.parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        break (i + 4, length);
                    }
                };
                while data.len() < header_end + content_length {
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&chunk[..n]);
                }
                let header = String::from_utf8_lossy(&data[..header_end]).into_owned();
                assert!(
                    header
                        .to_ascii_lowercase()
                        .contains("authorization: bearer fixture-secret")
                );
                let body = if content_length > 0 {
                    serde_json::from_slice(&data[header_end..header_end + content_length]).unwrap()
                } else {
                    Value::Null
                };
                requests.push((header, body));
                let body = reply.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
            requests
        });
        let engine = HttpEngine {
            kind,
            client: Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
            endpoint: Url::parse(&format!("http://127.0.0.1:{port}/projects/project")).unwrap(),
            token: "fixture-secret".into(),
            google_auth: None,
            username: String::new(),
            database: RwLock::new("dataset".into()),
            location: Some("EU".into()),
            timeout: Duration::from_secs(3),
        };
        (engine, task)
    }
    #[tokio::test]
    async fn turso_binds_values_closes_stream_and_decodes_exact_integers_and_blobs() {
        let reply = json!({"results":[{"type":"ok","response":{"result":{"cols":[{"name":"id","decltype":"INTEGER"},{"name":"data","decltype":"BLOB"}],"rows":[[{"type":"integer","value":"9007199254740993"},{"type":"blob","base64":"AP8="}],[{"type":"integer","value":"2"},{"type":"blob","base64":""}]],"affected_row_count":0}}}]});
        let (engine, server) = fixture(DatabaseKind::Turso, vec![reply]).await;
        let result = engine
            .query_statement(
                &SqlStatement::new("SELECT ?", vec![CellValue::Text("O'Reilly".into())]),
                QueryOptions { max_rows: Some(1) },
            )
            .await
            .unwrap();
        assert_eq!(
            result.rows[0].values,
            vec![
                CellValue::Integer(9007199254740993),
                CellValue::Bytes(vec![0, 255])
            ]
        );
        assert!(result.truncated);
        let requests = server.await.unwrap();
        assert!(
            requests[0]
                .0
                .starts_with("POST /projects/project/v2/pipeline ")
        );
        assert_eq!(
            requests[0].1["requests"][0]["stmt"]["args"][0],
            json!({"type":"text","value":"O'Reilly"})
        );
        assert_eq!(requests[0].1["requests"][1]["type"], "close");
    }
    #[tokio::test]
    async fn bigquery_requests_a_current_google_token_for_each_api_call() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Provider(AtomicUsize);
        #[async_trait]
        impl gcp_auth::TokenProvider for Provider {
            async fn token(
                &self,
                scopes: &[&str],
            ) -> std::result::Result<Arc<gcp_auth::Token>, gcp_auth::Error> {
                assert_eq!(scopes, ["https://www.googleapis.com/auth/cloud-platform"]);
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(Arc::new(
                    serde_json::from_value(
                        json!({"access_token":"fixture-secret", "expires_in":3600}),
                    )
                    .unwrap(),
                ))
            }
            async fn project_id(&self) -> std::result::Result<Arc<str>, gcp_auth::Error> {
                Ok(Arc::from("project"))
            }
        }
        let (mut engine, server) = fixture(
            DatabaseKind::BigQuery,
            vec![json!({"datasets":[]}), json!({"datasets":[]})],
        )
        .await;
        let provider = Arc::new(Provider(AtomicUsize::new(0)));
        engine.token.clear();
        engine.google_auth = Some(provider.clone());
        engine
            .request(Method::GET, engine.url(&["datasets"]), None)
            .await
            .unwrap();
        engine
            .request(Method::GET, engine.url(&["datasets"]), None)
            .await
            .unwrap();
        assert_eq!(provider.0.load(Ordering::Relaxed), 2);
        assert_eq!(server.await.unwrap().len(), 2);
    }
    #[tokio::test]
    async fn d1_uses_database_endpoint_and_bound_parameters() {
        let (engine,server)=fixture(DatabaseKind::CloudflareD1,vec![json!({"success":true,"result":[{"success":true,"results":[{"id":1,"name":"hello"}],"meta":{"changes":0}}]})]).await;
        let result = engine
            .query_statement(
                &SqlStatement::new(
                    "SELECT * FROM items WHERE name = ?",
                    vec![CellValue::Text("hello".into())],
                ),
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 1);
        let requests = server.await.unwrap();
        assert!(
            requests[0]
                .0
                .starts_with("POST /projects/project/dataset/query ")
        );
        assert_eq!(requests[0].1["params"], json!(["hello"]));
    }
    #[tokio::test]
    async fn bigquery_polls_jobs_preserves_location_and_pages_results() {
        let fields =
            json!({"fields":[{"name":"id","type":"INTEGER"},{"name":"value","type":"RECORD"}]});
        let job = json!({"jobId":"job1","location":"EU"});
        let replies = vec![
            json!({"jobComplete":false,"jobReference":job}),
            json!({"jobComplete":true,"jobReference":job,"schema":fields,"rows":[{"f":[{"v":"9007199254740993"},{"v":{"f":[{"v":"nested"}]}}]}],"pageToken":"next"}),
            json!({"jobComplete":true,"rows":[{"f":[{"v":"2"},{"v":null}]}]}),
        ];
        let (engine, server) = fixture(DatabaseKind::BigQuery, replies).await;
        let result = engine
            .query_statement(
                &SqlStatement::new("SELECT ?", vec![CellValue::Integer(9007199254740993)]),
                QueryOptions { max_rows: Some(2) },
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(
            result.rows[0].values[0],
            CellValue::Integer(9007199254740993)
        );
        assert!(!result.truncated);
        let requests = server.await.unwrap();
        assert_eq!(requests[0].1["useLegacySql"], false);
        assert_eq!(
            requests[0].1["queryParameters"][0]["parameterValue"]["value"],
            "9007199254740993"
        );
        assert!(requests[1].0.contains("location=EU"));
        assert!(requests[2].0.contains("pageToken=next"));
    }
    #[test]
    fn rejects_unrepresentable_parameters_instead_of_losing_values() {
        assert!(turso_arg(&CellValue::Unsigned(u64::MAX)).is_err());
        assert!(bq_arg(&CellValue::Real(f64::NAN)).is_err());
        assert!(
            super::super::d1_binding::bind(&SqlStatement::new(
                "SELECT ?",
                vec![CellValue::Bytes(vec![1])]
            ))
            .is_err()
        );
    }
}
