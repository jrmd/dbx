use std::fmt;

use serde::{Deserialize, Serialize};

/// The database families supported by DBX.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseKind {
    PostgreSQL,
    MySQL,
    SQLite,
    Redis,
    MongoDB,
    CockroachDB,
    DuckDB,
    Elasticsearch,
    BigQuery,
    Kafka,
    Turso,
    CloudflareD1,
    ClickHouse,
}

impl DatabaseKind {
    pub const ALL: [Self; 13] = [
        Self::PostgreSQL,
        Self::MySQL,
        Self::SQLite,
        Self::Redis,
        Self::MongoDB,
        Self::CockroachDB,
        Self::DuckDB,
        Self::Elasticsearch,
        Self::BigQuery,
        Self::Kafka,
        Self::Turso,
        Self::CloudflareD1,
        Self::ClickHouse,
    ];
    pub const SQL: [Self; 9] = [
        Self::PostgreSQL,
        Self::MySQL,
        Self::SQLite,
        Self::CockroachDB,
        Self::DuckDB,
        Self::BigQuery,
        Self::Turso,
        Self::CloudflareD1,
        Self::ClickHouse,
    ];

    pub const fn is_sql(self) -> bool {
        matches!(
            self,
            Self::PostgreSQL
                | Self::MySQL
                | Self::SQLite
                | Self::CockroachDB
                | Self::DuckDB
                | Self::BigQuery
                | Self::Turso
                | Self::CloudflareD1
                | Self::ClickHouse
        )
    }

    pub const fn dialect(self) -> Self {
        match self {
            Self::CockroachDB => Self::PostgreSQL,
            Self::Turso | Self::CloudflareD1 => Self::SQLite,
            other => other,
        }
    }
    pub const fn is_file(self) -> bool {
        matches!(self, Self::SQLite | Self::DuckDB)
    }
    pub const fn supports_transport(self) -> bool {
        matches!(
            self,
            Self::PostgreSQL | Self::MySQL | Self::Redis | Self::CockroachDB
        )
    }
    pub const fn supports_details(self) -> bool {
        self.supports_transport()
            || matches!(
                self,
                Self::MongoDB | Self::Elasticsearch | Self::Kafka | Self::ClickHouse
            )
    }
    pub const fn supports_row_mutations(self) -> bool {
        self.is_sql() && !matches!(self, Self::BigQuery | Self::ClickHouse)
    }
    pub fn accepts_scheme(self, scheme: &str) -> bool {
        match self {
            Self::PostgreSQL | Self::CockroachDB => matches!(scheme, "postgres" | "postgresql"),
            Self::MongoDB => matches!(scheme, "mongodb" | "mongodb+srv"),
            Self::Elasticsearch => matches!(scheme, "http" | "https"),
            Self::ClickHouse => matches!(scheme, "clickhouse" | "http" | "https"),
            Self::Turso => matches!(scheme, "libsql" | "turso" | "https" | "http"),
            _ => scheme == self.scheme(),
        }
    }
    pub const fn default_port(self) -> Option<&'static str> {
        match self {
            Self::PostgreSQL => Some("5432"),
            Self::CockroachDB => Some("26257"),
            Self::MySQL => Some("3306"),
            Self::Redis => Some("6379"),
            Self::MongoDB => Some("27017"),
            Self::Elasticsearch => Some("9200"),
            Self::Kafka => Some("9092"),
            Self::ClickHouse => Some("8123"),
            _ => None,
        }
    }
    pub const fn default_url(self) -> &'static str {
        match self {
            Self::PostgreSQL => "postgres://postgres@localhost:5432/postgres",
            Self::MySQL => "mysql://root@localhost:3306/mysql",
            Self::SQLite => "sqlite://dbx.db?mode=rwc",
            Self::Redis => "redis://127.0.0.1:6379/0",
            Self::CockroachDB => "postgres://root@localhost:26257/defaultdb?sslmode=require",
            Self::MongoDB => "mongodb://localhost:27017/test",
            Self::DuckDB => "duckdb::memory:",
            Self::Elasticsearch => "http://localhost:9200",
            Self::Kafka => "kafka://localhost:9092",
            Self::BigQuery => "bigquery://project/dataset",
            Self::Turso => "libsql://database-organization.turso.io",
            Self::CloudflareD1 => "d1://account-id/database-id",
            Self::ClickHouse => "clickhouse://default@localhost:8123/default",
        }
    }
    pub const fn connection_help(self) -> &'static str {
        match self {
            Self::ClickHouse => {
                "Use clickhouse://user:password@host:8123/database for HTTP, or https://user:password@host:8443/database for TLS (ClickHouse Cloud). Row editing is disabled; run writes in SQL."
            }
            Self::PostgreSQL => "Supabase: use your direct or session-pooler PostgreSQL URL.",
            Self::CockroachDB => {
                "Use a PostgreSQL URL with the TLS options supplied by CockroachDB."
            }
            Self::MongoDB => {
                "MongoDB URI, including mongodb+srv:// and driver options. Queries use JSON database commands."
            }
            Self::DuckDB => {
                "Open a DuckDB file with duckdb:///absolute/path.duckdb or use duckdb::memory:."
            }
            Self::Elasticsearch => {
                "HTTP(S) endpoint. Use username/password for Basic auth, or :API_KEY@host for API-key auth. Queries: METHOD /path followed by JSON."
            }
            Self::BigQuery => {
                "bigquery://project/dataset?location=US. Enter a Google OAuth access token with BigQuery permissions below."
            }
            Self::Kafka => {
                "kafka://host:9092?brokers=host:9092,other:9092. TLS/SASL options: security.protocol, sasl.mechanism; username/password in URL. Queries use JSON actions."
            }
            Self::Turso => {
                "libsql://database-organization.turso.io (also accepts turso:// and HTTPS). Enter your database token below."
            }
            Self::CloudflareD1 => {
                "d1://account-id/database-id. Enter an API token with D1 permissions below."
            }
            _ => "",
        }
    }
    pub const fn default_query(self) -> &'static str {
        match self {
            Self::PostgreSQL | Self::CockroachDB => "SELECT current_database(), current_user;",
            Self::MySQL => "SELECT DATABASE(), CURRENT_USER();",
            Self::SQLite | Self::Turso | Self::CloudflareD1 => "SELECT sqlite_version();",
            Self::DuckDB => "SELECT version();",
            Self::ClickHouse => "SELECT currentDatabase(), currentUser(), version();",
            Self::BigQuery => "SELECT 1 AS connected;",
            Self::Redis => "SCAN 0 COUNT 100",
            Self::MongoDB => "{\"ping\": 1}",
            Self::Elasticsearch => "GET /",
            Self::Kafka => "{\"action\": \"topics\"}",
        }
    }

    pub const fn scheme(self) -> &'static str {
        match self {
            Self::PostgreSQL | Self::CockroachDB => "postgres",
            Self::MySQL => "mysql",
            Self::SQLite => "sqlite",
            Self::Redis => "redis",
            Self::MongoDB => "mongodb",
            Self::DuckDB => "duckdb",
            Self::Elasticsearch => "https",
            Self::BigQuery => "bigquery",
            Self::Kafka => "kafka",
            Self::Turso => "libsql",
            Self::CloudflareD1 => "d1",
            Self::ClickHouse => "clickhouse",
        }
    }
}

impl fmt::Display for DatabaseKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PostgreSQL => "PostgreSQL",
            Self::MySQL => "MySQL",
            Self::SQLite => "SQLite",
            Self::Redis => "Redis",
            Self::MongoDB => "MongoDB",
            Self::CockroachDB => "CockroachDB",
            Self::DuckDB => "DuckDB",
            Self::Elasticsearch => "Elasticsearch",
            Self::BigQuery => "BigQuery",
            Self::Kafka => "Kafka",
            Self::Turso => "Turso",
            Self::CloudflareD1 => "Cloudflare D1",
            Self::ClickHouse => "ClickHouse",
        })
    }
}

/// Describes how the engine should connect to one database.
///
/// `url` may contain credentials and is intentionally redacted by `Debug` so
/// it is safe to log the rest of a connection configuration.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionConfig {
    pub kind: DatabaseKind,
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    /// PostgreSQL socket directory, or MySQL/Redis socket file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<std::path::PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<crate::SshConfig>,
}

fn default_max_connections() -> u32 {
    8
}

fn default_connect_timeout_ms() -> u64 {
    10_000
}

impl ConnectionConfig {
    pub fn new(kind: DatabaseKind, url: impl Into<String>) -> Self {
        Self {
            kind,
            url: url.into(),
            max_connections: default_max_connections(),
            connect_timeout_ms: default_connect_timeout_ms(),
            socket: None,
            ssh: None,
        }
    }

    pub fn with_max_connections(mut self, max_connections: u32) -> Self {
        self.max_connections = max_connections;
        self
    }

    pub fn with_connect_timeout_ms(mut self, connect_timeout_ms: u64) -> Self {
        self.connect_timeout_ms = connect_timeout_ms;
        self
    }

    pub fn validate(&self) -> crate::Result<()> {
        crate::transport::validate(self)?;
        if self.url.trim().is_empty() {
            return Err(crate::DbxError::InvalidConfig("URL cannot be empty".into()));
        }
        if self.max_connections == 0 {
            return Err(crate::DbxError::InvalidConfig(
                "max_connections must be greater than zero".into(),
            ));
        }
        if self.connect_timeout_ms == 0 {
            return Err(crate::DbxError::InvalidConfig(
                "connect_timeout_ms must be greater than zero".into(),
            ));
        }
        let expected = self.kind.scheme();
        let scheme = self
            .url
            .split_once(':')
            .map(|(scheme, _)| scheme.to_ascii_lowercase());
        if !scheme
            .as_deref()
            .is_some_and(|scheme| self.kind.accepts_scheme(scheme))
        {
            return Err(crate::DbxError::InvalidConfig(format!(
                "expected a {expected} connection URL"
            )));
        }
        if let Some((_, query)) = self.url.split_once('?') {
            for (key, _) in
                url::form_urlencoded::parse(query.split('#').next().unwrap_or_default().as_bytes())
            {
                let key = key.to_ascii_lowercase();
                if key.contains("token")
                    || key.contains("password")
                    || key.contains("secret")
                    || key == "apikey"
                    || key == "api_key"
                    || key == "authmechanismproperties"
                {
                    return Err(crate::DbxError::InvalidConfig("Store credentials in the URL password field so DBX can keep them in the vault".into()));
                }
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectionConfig")
            .field("kind", &self.kind)
            .field("url", &crate::error::redact_url(&self.url))
            .field("max_connections", &self.max_connections)
            .field("connect_timeout_ms", &self.connect_timeout_ms)
            .field("socket", &self.socket)
            .field("ssh", &self.ssh)
            .finish()
    }
}

/// A database object shown in the navigator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TableInfo {
    pub name: String,
    pub schema: Option<String>,
    pub kind: EntityKind,
}

impl TableInfo {
    pub fn table(name: impl Into<String>, schema: Option<String>) -> Self {
        Self {
            name: name.into(),
            schema,
            kind: EntityKind::Table,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Table,
    View,
    Collection,
    Keyspace,
}

/// A column in a table or a result set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    /// Ordered values for a database enum column. Empty for ordinary scalar
    /// columns and result-set metadata that does not expose enum semantics.
    #[serde(default)]
    pub enum_values: Vec<String>,
    pub nullable: bool,
    pub ordinal: usize,
    pub primary_key: bool,
}

impl ColumnInfo {
    pub fn result(name: impl Into<String>, ordinal: usize, data_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            data_type: data_type.into(),
            enum_values: Vec::new(),
            nullable: true,
            ordinal,
            primary_key: false,
        }
    }
}

/// A normalized foreign-key constraint on a table.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ForeignKeyInfo {
    /// The database constraint name, when the engine exposes one.
    pub constraint_name: Option<String>,
    pub columns: Vec<String>,
    pub referenced_schema: Option<String>,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_update: Option<ReferentialAction>,
    pub on_delete: Option<ReferentialAction>,
}

/// A referential action declared by a foreign-key constraint.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferentialAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

impl ReferentialAction {
    pub(crate) fn from_metadata(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "NO ACTION" => Some(Self::NoAction),
            "RESTRICT" => Some(Self::Restrict),
            "CASCADE" => Some(Self::Cascade),
            "SET NULL" => Some(Self::SetNull),
            "SET DEFAULT" => Some(Self::SetDefault),
            _ => None,
        }
    }
}

/// The full structural metadata for a table or collection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TableStructure {
    pub columns: Vec<ColumnInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

/// A point-in-time snapshot of the relational metadata available to a
/// connection. This lets consumers inspect an entire schema without
/// coordinating independent table and constraint requests themselves.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelationalSchema {
    pub database: String,
    pub tables: Vec<RelationalTable>,
}

/// Structural metadata for one table or view in a [`RelationalSchema`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelationalTable {
    pub table: TableInfo,
    pub structure: TableStructure,
}

/// A row is kept as a positional vector to preserve duplicate/aliased column
/// names returned by arbitrary SQL.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RowData {
    pub values: Vec<CellValue>,
}

impl RowData {
    pub fn new(values: Vec<CellValue>) -> Self {
        Self { values }
    }
}

/// Values that can be sent through all supported SQL drivers and represented
/// in a GPUI table without losing the common scalar types.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CellValue {
    #[default]
    Null,
    Boolean(bool),
    Integer(i64),
    Unsigned(u64),
    Real(f64),
    Text(String),
    Bytes(Vec<u8>),
    Json(serde_json::Value),
}

impl Eq for CellValue {}

/// A value used by an insert or update mutation.
///
/// Parameters retain the database driver's normal binding and escaping. SQL
/// expressions are deliberately opt-in and validated by the statement builder
/// before they are emitted into a mutation statement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MutationValue {
    Parameter(CellValue),
    Expression(String),
}

impl MutationValue {
    /// Create a parameterized mutation value.
    pub fn parameter(value: CellValue) -> Self {
        Self::Parameter(value)
    }

    /// Create an explicit SQL expression for a mutation value.
    pub fn expression(expression: impl Into<String>) -> Self {
        Self::Expression(expression.into())
    }
}

impl From<CellValue> for MutationValue {
    fn from(value: CellValue) -> Self {
        Self::Parameter(value)
    }
}

impl fmt::Display for CellValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => formatter.write_str("NULL"),
            Self::Boolean(value) => value.fmt(formatter),
            Self::Integer(value) => value.fmt(formatter),
            Self::Unsigned(value) => value.fmt(formatter),
            Self::Real(value) => value.fmt(formatter),
            Self::Text(value) => formatter.write_str(value),
            Self::Bytes(value) => write!(formatter, "0x{}", hex(value)),
            Self::Json(value) => value.fmt(formatter),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// A table reference with an optional schema/database qualifier.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
}

impl TableRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            schema: None,
            name: name.into(),
        }
    }

    pub fn in_schema(schema: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            schema: Some(schema.into()),
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueryResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<RowData>,
    pub rows_affected: Option<u64>,
    /// True when the configured row limit omitted additional result rows.
    #[serde(default)]
    pub truncated: bool,
    pub elapsed_ms: u64,
}

impl QueryResult {
    pub fn empty(rows_affected: Option<u64>, elapsed_ms: u64) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected,
            truncated: false,
            elapsed_ms,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecResult {
    pub rows_affected: u64,
    pub last_insert_id: Option<u64>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderDirection {
    #[default]
    Ascending,
    Descending,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Order {
    pub column: String,
    #[serde(default)]
    pub direction: OrderDirection,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Page {
    pub limit: u32,
    pub offset: u64,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            limit: 100,
            offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Contains,
    StartsWith,
    EndsWith,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    IsNull,
    IsNotNull,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Filter {
    pub column: String,
    pub operator: FilterOperator,
    pub value: Option<CellValue>,
}

impl Filter {
    pub fn new(
        column: impl Into<String>,
        operator: FilterOperator,
        value: Option<CellValue>,
    ) -> Self {
        Self {
            column: column.into(),
            operator,
            value,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InsertRequest {
    pub table: TableRef,
    pub columns: Vec<String>,
    pub values: Vec<MutationValue>,
}

impl InsertRequest {
    /// Construct a parameterized insert from a complete row.
    pub fn from_row(table: TableRef, values: Vec<(String, CellValue)>) -> Self {
        Self::from_mutation_row(
            table,
            values
                .into_iter()
                .map(|(column, value)| (column, value.into()))
                .collect(),
        )
    }

    /// Construct an insert from a complete row with explicit mutation values.
    pub fn from_mutation_row(table: TableRef, values: Vec<(String, MutationValue)>) -> Self {
        let (columns, values): (Vec<_>, Vec<_>) = values.into_iter().unzip();
        Self {
            table,
            columns,
            values,
        }
    }

    /// Construct an insert from an explicit column/value split.
    pub fn new(table: TableRef, columns: Vec<String>, values: Vec<CellValue>) -> Self {
        Self::new_with_mutation_values(table, columns, values.into_iter().map(Into::into).collect())
    }

    /// Construct an insert with explicit parameter or SQL expression values.
    pub fn new_with_mutation_values(
        table: TableRef,
        columns: Vec<String>,
        values: Vec<MutationValue>,
    ) -> Self {
        Self {
            table,
            columns,
            values,
        }
    }
}

/// A guarded update. `filters` must contain equality predicates for every
/// primary-key column of the target row; the SQL builder enforces the
/// equality-only shape before it emits a statement. The caller obtains the
/// primary-key columns from table metadata because this request is kept
/// independent of a second metadata round trip.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UpdateRequest {
    pub table: TableRef,
    pub assignments: Vec<(String, MutationValue)>,
    pub filters: Vec<Filter>,
}

impl UpdateRequest {
    /// Construct an update with explicit guard filters.
    pub fn new(
        table: TableRef,
        assignments: Vec<(String, CellValue)>,
        filters: Vec<Filter>,
    ) -> Self {
        Self::new_with_mutation_values(
            table,
            assignments
                .into_iter()
                .map(|(column, value)| (column, value.into()))
                .collect(),
            filters,
        )
    }

    /// Construct an update with explicit parameter or SQL expression values.
    pub fn new_with_mutation_values(
        table: TableRef,
        assignments: Vec<(String, MutationValue)>,
        filters: Vec<Filter>,
    ) -> Self {
        Self {
            table,
            assignments,
            filters,
        }
    }

    /// Construct an update guarded by one or more primary-key values. A
    /// composite key is represented by multiple `(column, value)` pairs.
    pub fn for_primary_key(
        table: TableRef,
        assignments: Vec<(String, CellValue)>,
        primary_key: Vec<(String, CellValue)>,
    ) -> Self {
        Self::for_primary_key_with_mutation_values(
            table,
            assignments
                .into_iter()
                .map(|(column, value)| (column, value.into()))
                .collect(),
            primary_key,
        )
    }

    /// Construct a primary-key guarded update with explicit mutation values.
    pub fn for_primary_key_with_mutation_values(
        table: TableRef,
        assignments: Vec<(String, MutationValue)>,
        primary_key: Vec<(String, CellValue)>,
    ) -> Self {
        let filters = primary_key
            .into_iter()
            .map(|(column, value)| Filter::new(column, FilterOperator::Equals, Some(value)))
            .collect();
        Self {
            table,
            assignments,
            filters,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateColumn {
    pub name: String,
    /// A driver-specific type expression, for example `TEXT` or `BIGINT`.
    /// The expression is validated as a conservative identifier-like SQL
    /// fragment by the statement builder.
    pub data_type: String,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub primary_key: bool,
    pub default_expression: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateTableRequest {
    pub table: TableRef,
    pub columns: Vec<CreateColumn>,
    #[serde(default)]
    pub if_not_exists: bool,
}
