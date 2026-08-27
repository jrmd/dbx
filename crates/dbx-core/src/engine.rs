use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    ColumnInfo, ConnectionConfig, CreateTableRequest, DatabaseKind, DbxError, ExecResult, Filter,
    InsertRequest, Order, Page, QueryResult, RelationalSchema, Result, SqlStatement, TableInfo,
    TableRef, TableStructure, UpdateRequest, build_create_table, build_delete, build_drop_table,
    build_insert, build_select, build_truncate_table, build_update_with_columns,
};
use crate::{RedisEngine, SqlxEngine};

/// Controls how many rows an arbitrary query may materialize in memory.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueryOptions {
    /// `None` means no client-side cap. A cap is recommended for ad-hoc SQL
    /// and is applied after the driver returns rows.
    pub max_rows: Option<usize>,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            max_rows: Some(10_000),
        }
    }
}

/// Common asynchronous interface implemented by every DBX connection.
#[async_trait]
pub trait Engine: Send + Sync {
    fn kind(&self) -> DatabaseKind;

    async fn list_tables(&self) -> Result<Vec<TableInfo>>;

    /// Names of the databases reachable through this connection. For SQLite
    /// these are the attached database aliases; for Redis they are the
    /// logical indexes.
    async fn list_databases(&self) -> Result<Vec<String>>;

    /// Name (or index label) of the database the connection currently uses.
    async fn current_database(&self) -> Result<String>;

    /// Switch the active database while keeping the same [`Engine`] object.
    ///
    /// MySQL issues `USE`, Redis issues `SELECT`, PostgreSQL swaps the
    /// internal pool for one connected to the target database, and SQLite
    /// rejects the operation because a file is itself one database.
    async fn use_database(&self, name: &str) -> Result<()>;

    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>>;

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure>;

    /// Load a complete relational metadata snapshot for the active database.
    async fn relational_schema(&self) -> Result<RelationalSchema>;

    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult>;

    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult>;

    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult>;
}

/// A connected database. The enum keeps backend-specific dependencies behind
/// a single object while still allowing each backend to optimize internally.
pub enum DatabaseEngine {
    Sql(SqlxEngine),
    Redis(RedisEngine),
}

impl std::fmt::Debug for DatabaseEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DatabaseEngine")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

impl DatabaseEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        config.validate()?;
        if config.kind.is_sql() {
            Ok(Self::Sql(SqlxEngine::connect(config).await?))
        } else {
            Ok(Self::Redis(RedisEngine::connect(config).await?))
        }
    }

    pub fn kind(&self) -> DatabaseKind {
        match self {
            Self::Sql(engine) => engine.kind(),
            Self::Redis(engine) => engine.kind(),
        }
    }

    /// Discover the Redis commands available on this connected server.
    pub async fn redis_command_catalog(&self) -> Result<crate::RedisCommandCatalog> {
        match self {
            Self::Redis(engine) => engine.command_catalog().await,
            Self::Sql(_) => Err(DbxError::Unsupported {
                operation: "redis_command_catalog".into(),
                kind: self.kind(),
            }),
        }
    }

    pub async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        Engine::list_tables(self).await
    }

    pub async fn list_databases(&self) -> Result<Vec<String>> {
        Engine::list_databases(self).await
    }

    pub async fn current_database(&self) -> Result<String> {
        Engine::current_database(self).await
    }

    pub async fn use_database(&self, name: &str) -> Result<()> {
        Engine::use_database(self, name).await
    }

    pub async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        Engine::describe_table(self, table).await
    }

    pub async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        Engine::table_structure(self, table).await
    }

    pub async fn relational_schema(&self) -> Result<RelationalSchema> {
        Engine::relational_schema(self).await
    }

    pub async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        Engine::query(self, sql, options).await
    }

    pub async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        Engine::query_statement(self, statement, options).await
    }

    pub async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        Engine::execute(self, statement).await
    }

    pub async fn execute_sql(&self, sql: &str) -> Result<ExecResult> {
        self.execute(&SqlStatement::new(sql, Vec::new())).await
    }

    pub async fn query_table(
        &self,
        table: &TableRef,
        columns: &[String],
        filters: &[Filter],
        order: &[Order],
        page: Option<Page>,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        ensure_sql(self.kind(), "query_table")?;
        let statement = build_select(self.kind(), table, columns, filters, order, page)?;
        let mut result = self.query_statement(&statement, options).await?;
        // SQLx cannot expose result-set metadata for an empty `SELECT` through
        // `AnyRow`. Fall back to the table schema so an empty table still has
        // usable headers in the grid.
        if result.columns.is_empty() {
            result.columns = self.describe_table(table).await?;
        }
        Ok(result)
    }

    pub async fn create_table(&self, request: &CreateTableRequest) -> Result<ExecResult> {
        ensure_sql(self.kind(), "create_table")?;
        let statement = build_create_table(self.kind(), request)?;
        self.execute(&statement).await
    }

    pub async fn insert(&self, request: &InsertRequest) -> Result<ExecResult> {
        ensure_sql(self.kind(), "insert")?;
        let statement = build_insert(self.kind(), request)?;
        self.execute(&statement).await
    }

    pub async fn update(&self, request: &UpdateRequest) -> Result<ExecResult> {
        ensure_sql(self.kind(), "update")?;
        let columns = self.describe_table(&request.table).await?;
        ensure_primary_key_filters(&columns, &request.filters)?;
        let statement = build_update_with_columns(self.kind(), request, &columns)?;
        self.execute(&statement).await
    }

    pub async fn delete(&self, table: &TableRef, filters: &[Filter]) -> Result<ExecResult> {
        ensure_sql(self.kind(), "delete")?;
        let statement = build_delete(self.kind(), table, filters)?;
        self.execute(&statement).await
    }

    pub async fn truncate_table(&self, table: &TableRef) -> Result<ExecResult> {
        ensure_sql(self.kind(), "truncate_table")?;
        let statement = build_truncate_table(self.kind(), table)?;
        self.execute(&statement).await
    }

    pub async fn drop_table(&self, table: &TableRef) -> Result<ExecResult> {
        ensure_sql(self.kind(), "drop_table")?;
        let statement = build_drop_table(self.kind(), table)?;
        self.execute(&statement).await
    }
}

fn ensure_sql(kind: DatabaseKind, operation: &str) -> Result<()> {
    if kind.is_sql() {
        Ok(())
    } else {
        Err(DbxError::Unsupported {
            operation: operation.to_owned(),
            kind,
        })
    }
}

fn ensure_primary_key_filters(columns: &[ColumnInfo], filters: &[Filter]) -> Result<()> {
    let primary_keys: Vec<_> = columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| column.name.as_str())
        .collect();
    if primary_keys.is_empty() {
        return Err(DbxError::Parse(
            "update requires a table with a primary key".into(),
        ));
    }
    if filters.len() != primary_keys.len()
        || filters.iter().any(|filter| {
            filter.operator != crate::FilterOperator::Equals
                || filter.value.is_none()
                || !primary_keys.contains(&filter.column.as_str())
        })
        || primary_keys.iter().any(|column| {
            filters
                .iter()
                .filter(|filter| filter.column == *column)
                .count()
                != 1
        })
    {
        return Err(DbxError::Parse(
            "update requires equality predicates for every primary-key column".into(),
        ));
    }
    Ok(())
}

#[async_trait]
impl Engine for DatabaseEngine {
    fn kind(&self) -> DatabaseKind {
        self.kind()
    }

    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        match self {
            Self::Sql(engine) => engine.list_tables().await,
            Self::Redis(engine) => engine.list_tables().await,
        }
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        match self {
            Self::Sql(engine) => engine.list_databases().await,
            Self::Redis(engine) => engine.list_databases().await,
        }
    }

    async fn current_database(&self) -> Result<String> {
        match self {
            Self::Sql(engine) => engine.current_database().await,
            Self::Redis(engine) => engine.current_database().await,
        }
    }

    async fn use_database(&self, name: &str) -> Result<()> {
        match self {
            Self::Sql(engine) => engine.use_database(name).await,
            Self::Redis(engine) => engine.use_database(name).await,
        }
    }

    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        match self {
            Self::Sql(engine) => engine.describe_table(table).await,
            Self::Redis(engine) => engine.describe_table(table).await,
        }
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        match self {
            Self::Sql(engine) => engine.table_structure(table).await,
            Self::Redis(engine) => engine.table_structure(table).await,
        }
    }

    async fn relational_schema(&self) -> Result<RelationalSchema> {
        match self {
            Self::Sql(engine) => engine.relational_schema().await,
            Self::Redis(engine) => engine.relational_schema().await,
        }
    }

    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        match self {
            Self::Sql(engine) => engine.query(sql, options).await,
            Self::Redis(engine) => engine.query(sql, options).await,
        }
    }

    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        match self {
            Self::Sql(engine) => engine.query_statement(statement, options).await,
            Self::Redis(engine) => engine.query_statement(statement, options).await,
        }
    }

    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        match self {
            Self::Sql(engine) => engine.execute(statement).await,
            Self::Redis(engine) => engine.execute(statement).await,
        }
    }
}

/// Reusable conversion for driver implementations when a result must be
/// bounded by [`QueryOptions`].
pub(crate) fn row_limit(options: QueryOptions) -> Option<usize> {
    options.max_rows.map(|limit| limit.max(1))
}

/// Convert a generic affected-row count into the common result shape.
pub(crate) fn exec_result(
    rows_affected: u64,
    last_insert_id: Option<u64>,
    started: std::time::Instant,
) -> ExecResult {
    ExecResult {
        rows_affected,
        last_insert_id,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}

/// Convert a query timer into the common result shape.
pub(crate) fn query_result(
    columns: Vec<ColumnInfo>,
    rows: Vec<crate::RowData>,
    rows_affected: Option<u64>,
    truncated: bool,
    started: std::time::Instant,
) -> QueryResult {
    QueryResult {
        columns,
        rows,
        rows_affected,
        truncated,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
    }
}
