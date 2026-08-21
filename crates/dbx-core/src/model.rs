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
}

impl DatabaseKind {
    pub const SQL: [Self; 3] = [Self::PostgreSQL, Self::MySQL, Self::SQLite];

    pub const fn is_sql(self) -> bool {
        matches!(self, Self::PostgreSQL | Self::MySQL | Self::SQLite)
    }

    pub const fn scheme(self) -> &'static str {
        match self {
            Self::PostgreSQL => "postgres",
            Self::MySQL => "mysql",
            Self::SQLite => "sqlite",
            Self::Redis => "redis",
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

    pub(crate) fn validate(&self) -> crate::Result<()> {
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
        let scheme = self.url.split_once("://").map(|(scheme, _)| scheme);
        if let Some(scheme) = scheme
            && !scheme.eq_ignore_ascii_case(expected)
            && !(self.kind == DatabaseKind::PostgreSQL && scheme.eq_ignore_ascii_case("postgresql"))
        {
            return Err(crate::DbxError::InvalidConfig(format!(
                "expected a {expected} URL, got {scheme}"
            )));
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
    pub elapsed_ms: u64,
}

impl QueryResult {
    pub fn empty(rows_affected: Option<u64>, elapsed_ms: u64) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected,
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
    pub values: Vec<CellValue>,
}

impl InsertRequest {
    /// Construct an insert from a complete row. Each value is paired with
    /// its target column so callers can submit more than one field while
    /// retaining the request's parameterized representation.
    pub fn from_row(table: TableRef, values: Vec<(String, CellValue)>) -> Self {
        let (columns, values): (Vec<_>, Vec<_>) = values.into_iter().unzip();
        Self {
            table,
            columns,
            values,
        }
    }

    /// Construct an insert from an explicit column/value split.
    pub fn new(table: TableRef, columns: Vec<String>, values: Vec<CellValue>) -> Self {
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
    pub assignments: Vec<(String, CellValue)>,
    pub filters: Vec<Filter>,
}

impl UpdateRequest {
    /// Construct an update with explicit guard filters.
    pub fn new(
        table: TableRef,
        assignments: Vec<(String, CellValue)>,
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
