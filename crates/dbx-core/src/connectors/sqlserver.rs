//! Microsoft SQL Server over TDS. Metadata and each query document own
//! separate clients. Query documents discard failed transports without replay.
use crate::{
    CellValue, CheckConstraintInfo, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine,
    EntityKind, ForeignKeyInfo, IndexInfo, QueryOptions, QueryResult, ReferentialAction, Result,
    RowData, SqlStatement, TableInfo, TableRef, TableStructure,
};
use async_trait::async_trait;
use futures_util::TryStreamExt;
use std::{
    borrow::Cow,
    time::{Duration, Instant},
};
use tiberius::{
    AuthMethod, Client, ColumnData, ColumnType, Config, EncryptionLevel, FromSql, QueryItem, ToSql,
};
use tokio::{net::TcpStream, sync::Mutex};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use url::Url;

type SqlServerClient = Client<Compat<TcpStream>>;

/// Interactive results share the native connectors' retained-data bound.
const RESULT_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct SqlServerEngine {
    config: Mutex<Config>,
    client: Mutex<Option<SqlServerClient>>,
    connect_timeout: Duration,
    read_only: bool,
    isolated: bool,
}

fn invalid(message: &str) -> DbxError {
    DbxError::InvalidConfig(message.into())
}

fn driver_error(error: tiberius::error::Error) -> DbxError {
    match error {
        tiberius::error::Error::Io { .. } | tiberius::error::Error::Tls(_) => {
            DbxError::Connection(error.to_string())
        }
        tiberius::error::Error::Server(ref token) => {
            DbxError::Query(format!("{} (error {})", token.message(), token.code()))
        }
        other => DbxError::Query(other.to_string()),
    }
}

/// Parse `sqlserver://user:password@host:port/database?options`.
fn parse_config(config: &ConnectionConfig) -> Result<Config> {
    let url = Url::parse(&config.url).map_err(|_| invalid("Invalid SQL Server URL"))?;
    if !matches!(url.scheme(), "sqlserver" | "mssql") {
        return Err(invalid("SQL Server URLs start with sqlserver://"));
    }
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| invalid("SQL Server host is required"))?;
    let mut tds = Config::new();
    tds.host(host.trim_matches(['[', ']']));
    tds.port(url.port().unwrap_or(1433));
    tds.application_name("DBX");
    let database = super::decode(url.path().trim_matches('/'))?;
    if !database.is_empty() {
        tds.database(database);
    }
    let username = super::decode(url.username())?;
    let password = url
        .password()
        .map(super::decode)
        .transpose()?
        .unwrap_or_default();
    if username.is_empty() {
        return Err(invalid("Enter a SQL Server login"));
    }
    tds.authentication(AuthMethod::sql_server(username, password));
    tds.encryption(EncryptionLevel::Required);
    for (key, value) in url.query_pairs() {
        let enabled = || match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Ok(true),
            "false" | "no" | "0" => Ok(false),
            _ => Err(invalid("SQL Server boolean options use true or false")),
        };
        match key.as_ref() {
            "encrypt" => {
                if !enabled()? {
                    tds.encryption(EncryptionLevel::NotSupported);
                }
            }
            "trust_server_certificate" | "trustServerCertificate" => {
                if enabled()? {
                    tds.trust_cert();
                }
            }
            "user" | "password" | "token" => {
                return Err(invalid(
                    "Put SQL Server credentials in the URL authority, not query options",
                ));
            }
            _ => {
                return Err(invalid(
                    "SQL Server URLs accept only encrypt and trust_server_certificate options",
                ));
            }
        }
    }
    if config.read_only {
        tds.readonly(true);
    }
    Ok(tds)
}

/// The SQL Server type name for result-set metadata.
fn column_type_name(column_type: ColumnType) -> &'static str {
    match column_type {
        ColumnType::Null => "null",
        ColumnType::Bit | ColumnType::Bitn => "bit",
        ColumnType::Int1 => "tinyint",
        ColumnType::Int2 => "smallint",
        ColumnType::Int4 | ColumnType::Intn => "int",
        ColumnType::Int8 => "bigint",
        ColumnType::Float4 => "real",
        ColumnType::Float8 | ColumnType::Floatn => "float",
        ColumnType::Money | ColumnType::Money4 => "money",
        ColumnType::Datetime | ColumnType::Datetimen => "datetime",
        ColumnType::Datetime4 => "smalldatetime",
        ColumnType::Daten => "date",
        ColumnType::Timen => "time",
        ColumnType::Datetime2 => "datetime2",
        ColumnType::DatetimeOffsetn => "datetimeoffset",
        ColumnType::Guid => "uniqueidentifier",
        ColumnType::Decimaln | ColumnType::Numericn => "decimal",
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image => "varbinary",
        ColumnType::BigVarChar | ColumnType::BigChar | ColumnType::Text => "varchar",
        ColumnType::NVarchar | ColumnType::NChar | ColumnType::NText => "nvarchar",
        ColumnType::Xml => "xml",
        ColumnType::Udt => "udt",
        ColumnType::SSVariant => "sql_variant",
    }
}

fn temporal<'a, T: FromSql<'a> + ToString>(data: &'a ColumnData<'static>) -> CellValue {
    match T::from_sql(data) {
        Ok(Some(value)) => CellValue::Text(value.to_string()),
        Ok(None) => CellValue::Null,
        Err(error) => CellValue::Text(format!("<{error}>")),
    }
}

fn cell_value(data: &ColumnData<'static>) -> CellValue {
    match data {
        ColumnData::U8(value) => value.map_or(CellValue::Null, |v| CellValue::Integer(v.into())),
        ColumnData::I16(value) => value.map_or(CellValue::Null, |v| CellValue::Integer(v.into())),
        ColumnData::I32(value) => value.map_or(CellValue::Null, |v| CellValue::Integer(v.into())),
        ColumnData::I64(value) => value.map_or(CellValue::Null, CellValue::Integer),
        ColumnData::F32(value) => value.map_or(CellValue::Null, |v| CellValue::Real(v.into())),
        ColumnData::F64(value) => value.map_or(CellValue::Null, CellValue::Real),
        ColumnData::Bit(value) => value.map_or(CellValue::Null, CellValue::Boolean),
        ColumnData::String(value) => value
            .as_ref()
            .map_or(CellValue::Null, |v| CellValue::Text(v.to_string())),
        ColumnData::Guid(value) => value
            .as_ref()
            .map_or(CellValue::Null, |v| CellValue::Text(v.to_string())),
        ColumnData::Binary(value) => value
            .as_ref()
            .map_or(CellValue::Null, |v| CellValue::Bytes(v.to_vec())),
        // Decimals keep their exact text rather than rounding through f64.
        ColumnData::Numeric(value) => value
            .as_ref()
            .map_or(CellValue::Null, |v| CellValue::Text(v.to_string())),
        ColumnData::Xml(value) => value
            .as_ref()
            .map_or(CellValue::Null, |v| CellValue::Text(v.to_string())),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            temporal::<chrono::NaiveDateTime>(data)
        }
        ColumnData::Date(_) => temporal::<chrono::NaiveDate>(data),
        ColumnData::Time(_) => temporal::<chrono::NaiveTime>(data),
        ColumnData::DateTimeOffset(_) => temporal::<chrono::DateTime<chrono::FixedOffset>>(data),
    }
}

/// An owned parameter that binds with SQL Server's native types.
enum Parameter {
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Text(String),
    Bytes(Vec<u8>),
    Null,
}

impl From<&CellValue> for Parameter {
    fn from(value: &CellValue) -> Self {
        match value {
            CellValue::Null => Self::Null,
            CellValue::Boolean(value) => Self::Boolean(*value),
            CellValue::Integer(value) => Self::Integer(*value),
            CellValue::Unsigned(value) => match i64::try_from(*value) {
                Ok(value) => Self::Integer(value),
                Err(_) => Self::Text(value.to_string()),
            },
            CellValue::Real(value) => Self::Real(*value),
            CellValue::Text(value) => Self::Text(value.clone()),
            CellValue::Bytes(value) => Self::Bytes(value.clone()),
            CellValue::Json(value) => Self::Text(value.to_string()),
        }
    }
}

impl ToSql for Parameter {
    fn to_sql(&self) -> ColumnData<'_> {
        match self {
            Self::Integer(value) => ColumnData::I64(Some(*value)),
            Self::Real(value) => ColumnData::F64(Some(*value)),
            Self::Boolean(value) => ColumnData::Bit(Some(*value)),
            Self::Text(value) => ColumnData::String(Some(Cow::Borrowed(value))),
            Self::Bytes(value) => ColumnData::Binary(Some(Cow::Borrowed(value))),
            // An untyped NULL converts implicitly to any column type.
            Self::Null => ColumnData::String(None),
        }
    }
}

/// The leading keyword decides whether a batch reports affected rows.
fn is_row_count_statement(sql: &str) -> bool {
    let keyword = sql
        .trim_start()
        .split(|character: char| !character.is_ascii_alphabetic())
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(keyword.as_str(), "INSERT" | "UPDATE" | "DELETE" | "MERGE")
}

fn object_name(table: &TableRef) -> Result<String> {
    crate::sql::quote_table(
        DatabaseKind::SqlServer,
        &TableRef {
            schema: Some(table.schema.clone().unwrap_or_else(|| "dbo".into())),
            name: table.name.clone(),
        },
    )
}

fn text_at(row: &RowData, index: usize) -> Option<String> {
    match row.values.get(index)? {
        CellValue::Null => None,
        value => Some(value.to_string()),
    }
}

fn flag(row: &RowData, index: usize) -> bool {
    matches!(
        row.values.get(index),
        Some(CellValue::Boolean(true) | CellValue::Integer(1))
    )
}

impl SqlServerEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        config.validate()?;
        let engine = Self {
            config: Mutex::new(parse_config(&config)?),
            client: Mutex::new(None),
            connect_timeout: Duration::from_millis(config.connect_timeout_ms),
            read_only: config.read_only,
            isolated: false,
        };
        // Fail Test Connection and Connect on bad credentials immediately.
        engine.query("SELECT 1", QueryOptions::default()).await?;
        Ok(engine)
    }

    async fn open(&self) -> Result<SqlServerClient> {
        let config = self.config.lock().await.clone();
        let connect = async {
            let tcp = TcpStream::connect(config.get_addr())
                .await
                .map_err(|error| DbxError::Connection(error.to_string()))?;
            let _ = tcp.set_nodelay(true);
            Client::connect(config, tcp.compat_write())
                .await
                .map_err(|error| match error {
                    tiberius::error::Error::Tls(message) => DbxError::Connection(format!(
                        "TLS failed: {message}. For a local self-signed server add ?trust_server_certificate=true"
                    )),
                    other => driver_error(other),
                })
        };
        tokio::time::timeout(self.connect_timeout, connect)
            .await
            .map_err(|_| DbxError::Connection("SQL Server connection timed out".into()))?
    }

    /// Run `sql` on the shared connection. Without parameters a batch keeps
    /// `USE` and `SET` effects; with parameters it runs through
    /// `sp_executesql`.
    async fn run(
        &self,
        sql: &str,
        params: &[CellValue],
        options: QueryOptions,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        let mut guard = self.client.lock().await;
        if let Some(client) = guard.as_mut() {
            let healthy = tokio::time::timeout(Duration::from_secs(5), async {
                client.simple_query("SELECT 1").await?.into_results().await
            })
            .await
            .is_ok_and(|result| result.is_ok());
            if !healthy {
                guard.take();
                if self.isolated {
                    return Err(DbxError::Connection(
                        "SQL Server query session connection was lost".into(),
                    ));
                }
            }
        }
        if guard.is_none() {
            *guard = Some(self.open().await?);
        }
        let client = guard.as_mut().expect("connected above");
        let owned = params.iter().map(Parameter::from).collect::<Vec<_>>();
        let bound = owned
            .iter()
            .map(|parameter| parameter as &dyn ToSql)
            .collect::<Vec<_>>();
        let outcome = if is_row_count_statement(sql) {
            client
                .execute(sql, &bound)
                .await
                .map(|result| QueryResult::empty(Some(result.total()), 0))
        } else {
            collect(client, sql, &bound, options).await
        };
        match outcome {
            Ok(mut result) => {
                result.elapsed_ms = started.elapsed().as_millis() as u64;
                Ok(result)
            }
            Err(error) => {
                let error = driver_error(error);
                // A broken transport cannot be reused; reconnect next time.
                if matches!(error, DbxError::Connection(_)) {
                    guard.take();
                }
                Err(error)
            }
        }
    }

    async fn metadata(&self, sql: &str, params: &[CellValue]) -> Result<Vec<RowData>> {
        Ok(self
            .run(sql, params, QueryOptions { max_rows: None })
            .await?
            .rows)
    }

    async fn indexes(&self, object: &str) -> Result<Vec<IndexInfo>> {
        let rows = self
            .metadata(
                "SELECT i.name, i.is_unique, i.is_primary_key, i.type_desc, i.filter_definition, c.name, ic.is_descending_key FROM sys.indexes i JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 0 JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id WHERE i.object_id = OBJECT_ID(@P1) AND i.name IS NOT NULL ORDER BY i.is_primary_key DESC, i.name, ic.key_ordinal",
                &[CellValue::Text(object.into())],
            )
            .await?;
        let mut indexes: Vec<IndexInfo> = Vec::new();
        for row in &rows {
            let name = text_at(row, 0).unwrap_or_default();
            let mut part = text_at(row, 5).unwrap_or_default();
            if flag(row, 6) {
                part.push_str(" DESC");
            }
            match indexes.last_mut().filter(|index| index.name == name) {
                Some(index) => index.columns.push(part),
                None => indexes.push(IndexInfo {
                    name,
                    columns: vec![part],
                    unique: flag(row, 1),
                    primary: flag(row, 2),
                    method: text_at(row, 3),
                    predicate: text_at(row, 4),
                    definition: None,
                }),
            }
        }
        Ok(indexes)
    }

    async fn foreign_keys(&self, object: &str) -> Result<Vec<ForeignKeyInfo>> {
        let rows = self
            .metadata(
                "SELECT fk.name, pc.name, rs.name, rt.name, rc.name, fk.update_referential_action_desc, fk.delete_referential_action_desc FROM sys.foreign_keys fk JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id JOIN sys.columns pc ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id JOIN sys.tables rt ON rt.object_id = fkc.referenced_object_id JOIN sys.schemas rs ON rs.schema_id = rt.schema_id JOIN sys.columns rc ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id WHERE fk.parent_object_id = OBJECT_ID(@P1) ORDER BY fk.name, fkc.constraint_column_id",
                &[CellValue::Text(object.into())],
            )
            .await?;
        let action = |value: Option<String>| {
            value.and_then(|value| ReferentialAction::from_metadata(&value.replace('_', " ")))
        };
        let mut keys: Vec<ForeignKeyInfo> = Vec::new();
        for row in &rows {
            let name = text_at(row, 0);
            let (local, referenced) = (
                text_at(row, 1).unwrap_or_default(),
                text_at(row, 4).unwrap_or_default(),
            );
            match keys.last_mut().filter(|key| key.constraint_name == name) {
                Some(key) => {
                    key.columns.push(local);
                    key.referenced_columns.push(referenced);
                }
                None => keys.push(ForeignKeyInfo {
                    constraint_name: name,
                    columns: vec![local],
                    referenced_schema: text_at(row, 2),
                    referenced_table: text_at(row, 3).unwrap_or_default(),
                    referenced_columns: vec![referenced],
                    on_update: action(text_at(row, 5)),
                    on_delete: action(text_at(row, 6)),
                }),
            }
        }
        Ok(keys)
    }

    async fn checks(&self, object: &str) -> Result<Vec<CheckConstraintInfo>> {
        let rows = self
            .metadata(
                "SELECT name, definition FROM sys.check_constraints WHERE parent_object_id = OBJECT_ID(@P1) ORDER BY name",
                &[CellValue::Text(object.into())],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| CheckConstraintInfo {
                name: text_at(row, 0),
                expression: strip_parentheses(&text_at(row, 1).unwrap_or_default()),
            })
            .collect())
    }
}

/// SQL Server stores CHECK and DEFAULT expressions wrapped in parentheses.
fn strip_parentheses(expression: &str) -> String {
    let mut expression = expression.trim();
    while let Some(inner) = expression
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let mut depth = 0i32;
        let balanced = inner.chars().all(|character| {
            match character {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            depth >= 0
        }) && depth == 0;
        if !balanced {
            break;
        }
        expression = inner.trim();
    }
    expression.to_owned()
}

async fn collect(
    client: &mut SqlServerClient,
    sql: &str,
    params: &[&dyn ToSql],
    options: QueryOptions,
) -> tiberius::Result<QueryResult> {
    let limit = crate::engine::row_limit(options).unwrap_or(usize::MAX);
    let mut stream = if params.is_empty() {
        client.simple_query(sql).await?
    } else {
        client.query(sql, params).await?
    };
    let mut columns: Option<Vec<ColumnInfo>> = None;
    let mut rows = Vec::new();
    let mut truncated = false;
    let mut retained = 0usize;
    // The first result set is shown; later ones are drained so the
    // connection is ready for the next request.
    let mut first_result = None;
    while let Some(item) = stream.try_next().await? {
        match item {
            QueryItem::Metadata(metadata) => {
                if columns.is_none() {
                    first_result = Some(metadata.result_index());
                    columns = Some(
                        metadata
                            .columns()
                            .iter()
                            .enumerate()
                            .map(|(index, column)| {
                                ColumnInfo::result(
                                    column.name(),
                                    index,
                                    column_type_name(column.column_type()),
                                )
                            })
                            .collect(),
                    );
                }
            }
            QueryItem::Row(row) => {
                if Some(row.result_index()) != first_result || truncated {
                    continue;
                }
                if rows.len() >= limit || retained > RESULT_BYTES {
                    truncated = true;
                    continue;
                }
                let values = row
                    .cells()
                    .map(|(_, data)| cell_value(data))
                    .collect::<Vec<_>>();
                retained += values
                    .iter()
                    .map(|value| match value {
                        CellValue::Text(text) => text.len(),
                        CellValue::Bytes(bytes) => bytes.len(),
                        _ => 16,
                    })
                    .sum::<usize>();
                rows.push(RowData::new(values));
            }
        }
    }
    Ok(match columns {
        Some(columns) => QueryResult {
            columns,
            rows,
            rows_affected: None,
            truncated,
            elapsed_ms: 0,
        },
        None => QueryResult::empty(Some(0), 0),
    })
}

#[async_trait]
impl Engine for SqlServerEngine {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::SqlServer
    }

    fn is_read_only(&self) -> bool {
        self.read_only
    }

    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let rows = self
            .metadata(
                "SELECT s.name, o.name, o.type FROM sys.objects o JOIN sys.schemas s ON s.schema_id = o.schema_id WHERE o.type IN ('U', 'V') AND o.is_ms_shipped = 0 ORDER BY s.name, o.name",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| TableInfo {
                schema: text_at(row, 0),
                name: text_at(row, 1).unwrap_or_default(),
                kind: if text_at(row, 2).is_some_and(|kind| kind.trim() == "V") {
                    EntityKind::View
                } else {
                    EntityKind::Table
                },
            })
            .collect())
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        let rows = self
            .metadata(
                "SELECT name FROM sys.databases WHERE HAS_DBACCESS(name) = 1 AND state_desc = 'ONLINE' ORDER BY name",
                &[],
            )
            .await?;
        Ok(rows.iter().filter_map(|row| text_at(row, 0)).collect())
    }

    async fn current_database(&self) -> Result<String> {
        let rows = self.metadata("SELECT DB_NAME()", &[]).await?;
        Ok(rows
            .first()
            .and_then(|row| text_at(row, 0))
            .unwrap_or_default())
    }

    async fn use_database(&self, name: &str) -> Result<()> {
        let quoted = crate::sql::quote_identifier(DatabaseKind::SqlServer, name)?;
        self.run(&format!("USE {quoted}"), &[], QueryOptions::default())
            .await?;
        // A reconnect must land in the same database.
        self.config.lock().await.database(name);
        Ok(())
    }

    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        let object = object_name(table)?;
        let rows = self
            .metadata(
                "SELECT c.name, TYPE_NAME(c.user_type_id) + CASE WHEN TYPE_NAME(c.user_type_id) IN ('varchar', 'char', 'varbinary', 'binary') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length AS varchar(10)) END + ')' WHEN TYPE_NAME(c.user_type_id) IN ('nvarchar', 'nchar') THEN '(' + CASE WHEN c.max_length = -1 THEN 'max' ELSE CAST(c.max_length / 2 AS varchar(10)) END + ')' WHEN TYPE_NAME(c.user_type_id) IN ('decimal', 'numeric') THEN '(' + CAST(c.precision AS varchar(10)) + ',' + CAST(c.scale AS varchar(10)) + ')' WHEN TYPE_NAME(c.user_type_id) IN ('datetime2', 'time', 'datetimeoffset') THEN '(' + CAST(c.scale AS varchar(10)) + ')' ELSE '' END, c.is_nullable, c.column_id, CASE WHEN EXISTS (SELECT 1 FROM sys.index_columns ic JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) THEN 1 ELSE 0 END, OBJECT_DEFINITION(c.default_object_id), c.is_identity, c.is_computed FROM sys.columns c WHERE c.object_id = OBJECT_ID(@P1) ORDER BY c.column_id",
                &[CellValue::Text(object)],
            )
            .await?;
        Ok(rows
            .iter()
            .enumerate()
            .map(|(index, row)| ColumnInfo {
                name: text_at(row, 0).unwrap_or_default(),
                data_type: text_at(row, 1).unwrap_or_default(),
                enum_values: Vec::new(),
                nullable: flag(row, 2),
                ordinal: match row.values.get(3) {
                    Some(CellValue::Integer(ordinal)) => (*ordinal).max(1) as usize,
                    _ => index + 1,
                },
                primary_key: flag(row, 4),
                default_value: text_at(row, 5).map(|value| strip_parentheses(&value)),
            })
            .collect())
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        let object = object_name(table)?;
        let columns = self.describe_table(table).await?;
        let foreign_keys = self.foreign_keys(&object).await?;
        let indexes = self.indexes(&object).await?;
        let checks = self.checks(&object).await?;
        let definition = self
            .metadata(
                "SELECT OBJECT_DEFINITION(OBJECT_ID(@P1))",
                &[CellValue::Text(object)],
            )
            .await?
            .first()
            .and_then(|row| text_at(row, 0));
        Ok(TableStructure {
            columns,
            foreign_keys,
            indexes,
            checks,
            definition,
        })
    }

    async fn open_query_session(&self) -> Result<Option<Box<dyn Engine>>> {
        let engine = Self {
            config: Mutex::new(self.config.lock().await.clone()),
            client: Mutex::new(None),
            connect_timeout: self.connect_timeout,
            read_only: self.read_only,
            isolated: true,
        };
        *engine.client.lock().await = Some(engine.open().await?);
        Ok(Some(Box::new(engine)))
    }

    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        self.run(sql, &[], options).await
    }

    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        self.run(&statement.sql, &statement.params, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_require_tls_unless_explicitly_disabled() {
        let config = |url: &str| ConnectionConfig::new(DatabaseKind::SqlServer, url);
        assert!(parse_config(&config("sqlserver://sa:secret@db:1433/app")).is_ok());
        assert!(
            parse_config(&config(
                "sqlserver://sa:secret@localhost/app?encrypt=false&trust_server_certificate=true"
            ))
            .is_ok()
        );
        assert!(parse_config(&config("sqlserver://db/app")).is_err());
        assert!(parse_config(&config("sqlserver://sa@db/app?password=x")).is_err());
        assert!(parse_config(&config("sqlserver://sa@db/app?encrypt=maybe")).is_err());
        assert!(parse_config(&config("sqlserver://sa@db/app?ApplicationIntent=x")).is_err());
    }

    #[test]
    fn builder_uses_brackets_named_parameters_and_offset_fetch() {
        let table = TableRef::in_schema("dbo", "odd]name");
        let statement = crate::build_select(
            DatabaseKind::SqlServer,
            &table,
            &[],
            &[crate::Filter {
                column: "note".into(),
                operator: crate::FilterOperator::Contains,
                value: Some(CellValue::Text("50%[x]".into())),
            }],
            &[],
            Some(crate::Page {
                limit: 25,
                offset: 50,
            }),
        )
        .unwrap();
        assert_eq!(
            statement.sql,
            "SELECT * FROM [dbo].[odd]]name] WHERE [note] LIKE @P1 ESCAPE '!' ORDER BY (SELECT NULL) OFFSET @P2 ROWS FETCH NEXT @P3 ROWS ONLY"
        );
        assert_eq!(
            statement.params,
            [
                CellValue::Text("%50!%![x]%".into()),
                CellValue::Unsigned(50),
                CellValue::Unsigned(25)
            ]
        );
    }

    #[test]
    fn scripts_split_on_go_and_keep_module_bodies_whole() {
        let script = "SELECT * FROM #staging; SELECT 2\nGO\nCREATE OR ALTER PROCEDURE dbo.p AS\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND\ngo\n-- note\nUPDATE t SET a = 1";
        assert_eq!(
            crate::script::checked_split_sql_for(Some(DatabaseKind::SqlServer), script).unwrap(),
            [
                "SELECT * FROM #staging",
                "SELECT 2",
                "CREATE OR ALTER PROCEDURE dbo.p AS\nBEGIN\n  SELECT 1;\n  SELECT 2;\nEND",
                "UPDATE t SET a = 1",
            ]
        );
    }

    #[test]
    fn defaults_and_checks_lose_their_storage_parentheses() {
        assert_eq!(strip_parentheses("((0))"), "0");
        assert_eq!(strip_parentheses("([qty]>=(0))"), "[qty]>=(0)");
        assert_eq!(strip_parentheses("(getdate())"), "getdate()");
        assert_eq!(strip_parentheses("(a) + (b)"), "(a) + (b)");
        assert!(is_row_count_statement("  update t set a = 1"));
        assert!(!is_row_count_statement(
            "WITH x AS (SELECT 1) SELECT * FROM x"
        ));
    }
}
