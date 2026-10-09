use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    CellValue, ColumnInfo, ConnectionConfig, CreateTableRequest, DatabaseKind, DbxError,
    ExecResult, Filter, InsertRequest, Order, Page, QueryResult, RelationalSchema, Result,
    RowChange, SqlStatement, TableInfo, TableRef, TableStructure, UpdateRequest, build_count,
    build_create_table, build_delete_with_columns, build_drop_table, build_insert_with_columns,
    build_row_estimate, build_select_with_columns, build_truncate_table, build_update_with_columns,
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
    fn is_read_only(&self) -> bool {
        false
    }
    fn capabilities(&self) -> crate::Capabilities {
        self.kind().capabilities(self.is_read_only())
    }

    /// Connect an independent interactive session when this connector owns a
    /// stateful protocol outside SQLx. Stateless connectors return None.
    async fn open_query_session(&self) -> Result<Option<Box<dyn Engine>>> {
        Ok(None)
    }

    async fn list_tables(&self) -> Result<Vec<TableInfo>>;

    /// Names of the databases reachable through this connection. For SQLite
    /// these are the attached database aliases; for Redis they are the
    /// logical indexes.
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![self.current_database().await?])
    }

    /// Name (or index label) of the database the connection currently uses.
    async fn current_database(&self) -> Result<String>;

    /// Switch the active database while keeping the same [`Engine`] object.
    ///
    /// MySQL issues `USE`, Redis issues `SELECT`, PostgreSQL swaps the
    /// internal pool for one connected to the target database, and SQLite
    /// rejects the operation because a file is itself one database.
    async fn use_database(&self, _name: &str) -> Result<()> {
        Err(DbxError::Unsupported {
            operation: "use_database".into(),
            kind: self.kind(),
        })
    }

    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>>;

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        Ok(TableStructure {
            columns: self.describe_table(table).await?,
            foreign_keys: Vec::new(),
            ..Default::default()
        })
    }

    async fn schema_objects(&self) -> Result<Vec<crate::SchemaObject>> {
        crate::schema_objects::capture(self).await
    }

    /// Load a complete relational metadata snapshot for the active database.
    async fn relational_schema(&self) -> Result<RelationalSchema> {
        if !self.kind().is_sql() {
            return Err(DbxError::Unsupported {
                operation: "relational_schema".into(),
                kind: self.kind(),
            });
        }
        let mut tables = Vec::new();
        for info in self.list_tables().await? {
            let table = TableRef {
                name: info.name.clone(),
                schema: info.schema.clone(),
            };
            let structure = self.table_structure(&table).await?;
            tables.push(crate::RelationalTable {
                table: info,
                structure,
            });
        }
        Ok(RelationalSchema {
            database: self.current_database().await?,
            tables,
            objects: self.schema_objects().await?,
            objects_captured: true,
            details_captured: true,
        })
    }

    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult>;

    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        if !statement.params.is_empty() {
            return Err(DbxError::Unsupported {
                operation: "bound parameters".into(),
                kind: self.kind(),
            });
        }
        self.query(&statement.sql, options).await
    }

    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        let result = self
            .query_statement(statement, QueryOptions::default())
            .await?;
        Ok(ExecResult {
            rows_affected: result.rows_affected.unwrap_or(0),
            last_insert_id: None,
            elapsed_ms: result.elapsed_ms,
        })
    }
}

/// A connected database. The enum keeps backend-specific dependencies behind
/// a single object while still allowing each backend to optimize internally.
pub enum DatabaseEngine {
    Sql(SqlxEngine),
    Redis(RedisEngine),
    Other(Box<dyn Engine>),
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
        if matches!(
            config.kind,
            DatabaseKind::PostgreSQL
                | DatabaseKind::MySQL
                | DatabaseKind::SQLite
                | DatabaseKind::CockroachDB
        ) {
            Ok(Self::Sql(SqlxEngine::connect(config).await?))
        } else if config.kind == DatabaseKind::Redis {
            Ok(Self::Redis(RedisEngine::connect(config).await?))
        } else {
            let protected = config.read_only;
            let engine = crate::connectors::connect(config).await?;
            Ok(Self::Other(if protected {
                Box::new(crate::protected::ProtectedEngine(engine))
            } else {
                engine
            }))
        }
    }

    pub fn kind(&self) -> DatabaseKind {
        match self {
            Self::Sql(engine) => engine.kind(),
            Self::Redis(engine) => engine.kind(),
            Self::Other(engine) => engine.kind(),
        }
    }

    pub fn is_read_only(&self) -> bool {
        match self {
            Self::Sql(engine) => engine.is_read_only(),
            Self::Redis(engine) => engine.is_read_only(),
            Self::Other(engine) => engine.is_read_only(),
        }
    }
    pub fn capabilities(&self) -> crate::Capabilities {
        self.kind().capabilities(self.is_read_only())
    }
    pub(crate) fn ensure_writable(&self) -> Result<()> {
        if self.is_read_only() {
            return Err(DbxError::Query(
                "Protected connection: writes are disabled".into(),
            ));
        }
        Ok(())
    }

    /// Browse one page of the Redis keyspace from `cursor`. Returns the key
    /// grid and the cursor for the next page (`0` when complete).
    pub async fn redis_scan_page(
        &self,
        pattern: &str,
        cursor: u64,
        target: usize,
    ) -> Result<(QueryResult, u64)> {
        match self {
            Self::Redis(engine) => engine.scan_page(pattern, cursor, target).await,
            _ => Err(DbxError::Unsupported {
                operation: "redis_scan_page".into(),
                kind: self.kind(),
            }),
        }
    }

    /// Discover the Redis commands available on this connected server.
    pub async fn redis_command_catalog(&self) -> Result<crate::RedisCommandCatalog> {
        match self {
            Self::Redis(engine) => engine.command_catalog().await,
            _ => Err(DbxError::Unsupported {
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

    pub async fn schema_objects(&self) -> Result<Vec<crate::SchemaObject>> {
        Engine::schema_objects(self).await
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

    /// Execute an ordered SQL script on one connection and commit only after
    /// every statement succeeds. PostgreSQL and SQLite include transactional
    /// DDL; MySQL may implicitly commit DDL according to server semantics.
    pub async fn execute_transaction(&self, statements: &[String]) -> Result<()> {
        self.ensure_writable()?;
        match self {
            Self::Sql(engine) => engine.execute_transaction(statements).await,
            _ => Err(DbxError::Unsupported {
                operation: "execute_transaction".into(),
                kind: self.kind(),
            }),
        }
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
        self.query_table_with_columns(table, columns, filters, order, page, options, None)
            .await
    }

    /// Like [`Self::query_table`], but reuses column metadata the caller
    /// already holds instead of describing the table again. `known_columns`
    /// must describe `table`.
    #[allow(clippy::too_many_arguments)]
    pub async fn query_table_with_columns(
        &self,
        table: &TableRef,
        columns: &[String],
        filters: &[Filter],
        order: &[Order],
        page: Option<Page>,
        options: QueryOptions,
        known_columns: Option<&[ColumnInfo]>,
    ) -> Result<QueryResult> {
        if !self.kind().is_sql() {
            if !filters.is_empty() || !order.is_empty() || !columns.is_empty() {
                return Err(DbxError::Unsupported {
                    operation: "structured filters".into(),
                    kind: self.kind(),
                });
            }
            if let Self::Other(engine) = self {
                let command = crate::connectors::browse_command(self.kind(), table, page)?;
                return engine.query(&command, options).await;
            }
        }
        ensure_sql(self.kind(), "query_table")?;
        // PostgreSQL also needs the column types to choose which columns to
        // read as text; see `postgres_reads_as_text`.
        let metadata = match known_columns {
            _ if self.kind() != DatabaseKind::PostgreSQL => {
                self.filter_metadata(table, filters, known_columns).await?
            }
            Some(columns) if !columns.is_empty() => Some(columns.to_vec()),
            _ => Some(self.describe_table(table).await?),
        };
        let statement = build_select_with_columns(
            self.kind(),
            table,
            columns,
            filters,
            order,
            page,
            metadata.as_deref().unwrap_or_default(),
        )?;
        let mut result = match self {
            Self::Sql(engine) => {
                engine
                    .query_statement_headerless(&statement, options)
                    .await?
            }
            _ => self.query_statement(&statement, options).await?,
        };
        // A column read as text reports `text`; keep its declared type.
        if self.kind() == DatabaseKind::PostgreSQL
            && let Some(metadata) = &metadata
        {
            for column in &mut result.columns {
                if let Some(declared) = metadata.iter().find(|known| known.name == column.name)
                    && crate::sql::postgres_reads_as_text(&declared.data_type)
                {
                    column.data_type = declared.data_type.clone();
                }
            }
        }
        // MySQL flags text with a binary collation (`utf8mb4_bin`, and every
        // MariaDB JSON column) as binary, so the driver reads it as bytes.
        // Only the declared column type tells it apart from a real BLOB.
        if self.kind().dialect() == DatabaseKind::MySQL
            && result.rows.iter().any(|row| {
                row.values
                    .iter()
                    .any(|value| matches!(value, CellValue::Bytes(_)))
            })
        {
            let declared = match known_columns {
                Some(columns) if !columns.is_empty() => columns.to_vec(),
                _ => self.describe_table(table).await?,
            };
            crate::sql::recover_mysql_binary_collation_text(&mut result, &declared);
        }
        // An empty `SELECT` exposes no result-set metadata. Fall back to the
        // table schema (already known to many callers) so an empty table still
        // has usable headers in the grid, without sqlx's costly describe.
        if result.columns.is_empty() {
            result.columns = match (metadata, known_columns) {
                (Some(metadata), _) => metadata,
                (None, Some(known)) if !known.is_empty() => known.to_vec(),
                (None, _) => self.describe_table(table).await?,
            };
        }
        Ok(result)
    }

    /// Count the rows matching `filters` exactly.
    pub async fn count_rows(
        &self,
        table: &TableRef,
        filters: &[Filter],
        known_columns: Option<&[ColumnInfo]>,
    ) -> Result<u64> {
        ensure_sql(self.kind(), "count_rows")?;
        let metadata = self.filter_metadata(table, filters, known_columns).await?;
        let statement = build_count(
            self.kind(),
            table,
            filters,
            metadata.as_deref().unwrap_or_default(),
        )?;
        let result = self
            .query_statement(&statement, QueryOptions::default())
            .await?;
        first_count(&result).ok_or_else(|| DbxError::Query("COUNT(*) returned no number".into()))
    }

    /// The catalog's row estimate for `table`, when the engine keeps one.
    /// Never scans the table.
    pub async fn estimate_rows(&self, table: &TableRef) -> Result<Option<u64>> {
        if !self.kind().is_sql() {
            return Ok(None);
        }
        let Some(statement) = build_row_estimate(self.kind(), table)? else {
            return Ok(None);
        };
        let result = self
            .query_statement(&statement, QueryOptions::default())
            .await?;
        Ok(first_count(&result))
    }

    /// PostgreSQL needs column types to cast text filter parameters (for
    /// example, to `uuid`); other dialects coerce them implicitly.
    async fn filter_metadata(
        &self,
        table: &TableRef,
        filters: &[Filter],
        known_columns: Option<&[ColumnInfo]>,
    ) -> Result<Option<Vec<ColumnInfo>>> {
        if self.kind().dialect() != DatabaseKind::PostgreSQL || filters.is_empty() {
            return Ok(None);
        }
        match known_columns {
            Some(columns) if !columns.is_empty() => Ok(Some(columns.to_vec())),
            _ => self.describe_table(table).await.map(Some),
        }
    }

    pub async fn create_table(&self, request: &CreateTableRequest) -> Result<ExecResult> {
        ensure_clickhouse_sql_writes(self.kind(), "create_table")?;
        ensure_sql(self.kind(), "create_table")?;
        let statement = build_create_table(self.kind(), request)?;
        self.execute(&statement).await
    }

    pub async fn insert(&self, request: &InsertRequest) -> Result<ExecResult> {
        ensure_clickhouse_sql_writes(self.kind(), "insert")?;
        ensure_sql(self.kind(), "insert")?;
        let columns = self.describe_table(&request.table).await?;
        let statement = build_insert_with_columns(self.kind(), request, &columns)?;
        self.execute(&statement).await
    }

    pub async fn update(&self, request: &UpdateRequest) -> Result<ExecResult> {
        self.update_checked(request, &[]).await
    }

    pub async fn update_checked(
        &self,
        request: &UpdateRequest,
        originals: &[(String, CellValue)],
    ) -> Result<ExecResult> {
        ensure_clickhouse_sql_writes(self.kind(), "update")?;
        ensure_sql(self.kind(), "update")?;
        let columns = self.describe_table(&request.table).await?;
        ensure_primary_key_filters(&columns, &request.filters)?;
        let mut statement = build_update_with_columns(self.kind(), request, &columns)?;
        crate::sql::guard_original_values(self.kind(), &mut statement, originals, &columns)?;
        let result = self.execute(&statement).await?;
        if result.rows_affected != 1 {
            return Err(DbxError::Conflict);
        }
        Ok(result)
    }

    /// Apply several checked updates as one unit. Each must still match its
    /// original values and affect exactly one row.
    pub async fn update_checked_batch(
        &self,
        updates: &[(UpdateRequest, Vec<(String, CellValue)>)],
    ) -> Result<u64> {
        let changes = updates
            .iter()
            .map(|(request, originals)| RowChange::Update {
                request: request.clone(),
                originals: originals.clone(),
            })
            .collect::<Vec<_>>();
        self.apply_row_changes(&changes, None).await
    }

    /// Apply a staged changeset as one unit. Every update and delete must
    /// still match its original values and affect exactly one row. Native SQL
    /// engines run the whole set in one transaction, so any conflict or error
    /// rolls every change back; other writable engines apply changes in order
    /// and stop at the first failure, reporting how many were already applied.
    ///
    /// `known` supplies already-described columns for one table, saving a
    /// metadata round trip per save.
    pub async fn apply_row_changes(
        &self,
        changes: &[RowChange],
        known: Option<(&TableRef, &[ColumnInfo])>,
    ) -> Result<u64> {
        ensure_clickhouse_sql_writes(self.kind(), "change rows")?;
        ensure_sql(self.kind(), "change rows")?;
        let mut described: Vec<(TableRef, Vec<ColumnInfo>)> = known
            .filter(|(_, columns)| !columns.is_empty())
            .map(|(table, columns)| vec![(table.clone(), columns.to_vec())])
            .unwrap_or_default();
        let mut statements = Vec::with_capacity(changes.len());
        for change in changes {
            let table = change.table();
            let columns = match described.iter().find(|(known, _)| known == table) {
                Some((_, columns)) => columns.clone(),
                None => {
                    let columns = self.describe_table(table).await?;
                    described.push((table.clone(), columns.clone()));
                    columns
                }
            };
            let (statement, checked) = match change {
                RowChange::Insert(request) => (
                    build_insert_with_columns(self.kind(), request, &columns)?,
                    false,
                ),
                RowChange::Update { request, originals } => {
                    ensure_primary_key_filters(&columns, &request.filters)?;
                    let mut statement = build_update_with_columns(self.kind(), request, &columns)?;
                    crate::sql::guard_original_values(
                        self.kind(),
                        &mut statement,
                        originals,
                        &columns,
                    )?;
                    (statement, true)
                }
                RowChange::Delete {
                    table,
                    filters,
                    originals,
                } => {
                    ensure_primary_key_filters(&columns, filters)?;
                    let mut statement =
                        build_delete_with_columns(self.kind(), table, filters, &columns)?;
                    crate::sql::guard_original_values(
                        self.kind(),
                        &mut statement,
                        originals,
                        &columns,
                    )?;
                    (statement, true)
                }
            };
            statements.push((statement, checked));
        }
        if !matches!(self, Self::Sql(_)) && self.kind() != DatabaseKind::SqlServer {
            for (applied, (statement, checked)) in statements.iter().enumerate() {
                let outcome = self.execute(statement).await.and_then(|result| {
                    if *checked && result.rows_affected != 1 {
                        Err(DbxError::Conflict)
                    } else {
                        Ok(result)
                    }
                });
                outcome.map_err(|error| match applied {
                    0 => error,
                    applied => DbxError::Query(format!(
                        "{error}; {applied} earlier row change(s) were already applied"
                    )),
                })?;
            }
            return Ok(statements.len() as u64);
        }
        let mut transaction = crate::console::SqlTransaction::begin(self, false).await?;
        for (statement, checked) in &statements {
            let result = transaction.query(statement).await?;
            if *checked && result.rows_affected != Some(1) {
                // Dropping the transaction closes its connection and rolls back.
                return Err(DbxError::Conflict);
            }
        }
        transaction.commit().await?;
        Ok(statements.len() as u64)
    }

    /// Delete exactly one unchanged, primary-key identified row.
    pub async fn delete_checked(
        &self,
        table: &TableRef,
        filters: &[Filter],
        originals: &[(String, CellValue)],
    ) -> Result<ExecResult> {
        ensure_sql(self.kind(), "delete")?;
        ensure_clickhouse_sql_writes(self.kind(), "delete")?;
        let columns = self.describe_table(table).await?;
        ensure_primary_key_filters(&columns, filters)?;
        let mut statement = build_delete_with_columns(self.kind(), table, filters, &columns)?;
        crate::sql::guard_original_values(self.kind(), &mut statement, originals, &columns)?;
        let result = self.execute(&statement).await?;
        if result.rows_affected != 1 {
            return Err(DbxError::Conflict);
        }
        Ok(result)
    }

    pub async fn delete(&self, table: &TableRef, filters: &[Filter]) -> Result<ExecResult> {
        self.delete_with_columns(table, filters, None).await
    }

    /// Like [`Self::delete`], but reuses column metadata the caller already
    /// holds. `known_columns` must describe `table`.
    pub async fn delete_with_columns(
        &self,
        table: &TableRef,
        filters: &[Filter],
        known_columns: Option<&[ColumnInfo]>,
    ) -> Result<ExecResult> {
        ensure_sql(self.kind(), "delete")?;
        ensure_clickhouse_sql_writes(self.kind(), "delete")?;
        let metadata = self.filter_metadata(table, filters, known_columns).await?;
        let statement = build_delete_with_columns(
            self.kind(),
            table,
            filters,
            metadata.as_deref().unwrap_or_default(),
        )?;
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

fn ensure_clickhouse_sql_writes(kind: DatabaseKind, operation: &str) -> Result<()> {
    if kind == DatabaseKind::ClickHouse {
        Err(DbxError::Unsupported {
            operation: format!("{operation} through row editor; use SQL"),
            kind,
        })
    } else {
        Ok(())
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
                || filter.value == Some(CellValue::Null)
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
    fn is_read_only(&self) -> bool {
        DatabaseEngine::is_read_only(self)
    }

    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        match self {
            Self::Sql(engine) => engine.list_tables().await,
            Self::Redis(engine) => engine.list_tables().await,
            Self::Other(engine) => engine.list_tables().await,
        }
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        match self {
            Self::Sql(engine) => engine.list_databases().await,
            Self::Redis(engine) => engine.list_databases().await,
            Self::Other(engine) => engine.list_databases().await,
        }
    }

    async fn current_database(&self) -> Result<String> {
        match self {
            Self::Sql(engine) => engine.current_database().await,
            Self::Redis(engine) => engine.current_database().await,
            Self::Other(engine) => engine.current_database().await,
        }
    }

    async fn use_database(&self, name: &str) -> Result<()> {
        match self {
            Self::Sql(engine) => engine.use_database(name).await,
            Self::Redis(engine) => engine.use_database(name).await,
            Self::Other(engine) => engine.use_database(name).await,
        }
    }

    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        match self {
            Self::Sql(engine) => engine.describe_table(table).await,
            Self::Redis(engine) => engine.describe_table(table).await,
            Self::Other(engine) => engine.describe_table(table).await,
        }
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        match self {
            Self::Sql(engine) => engine.table_structure(table).await,
            Self::Redis(engine) => engine.table_structure(table).await,
            Self::Other(engine) => engine.table_structure(table).await,
        }
    }

    async fn relational_schema(&self) -> Result<RelationalSchema> {
        match self {
            Self::Sql(engine) => engine.relational_schema().await,
            Self::Redis(engine) => engine.relational_schema().await,
            Self::Other(engine) => engine.relational_schema().await,
        }
    }

    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        match self {
            Self::Sql(engine) => engine.query(sql, options).await,
            Self::Redis(engine) => engine.query(sql, options).await,
            Self::Other(engine) => engine.query(sql, options).await,
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
            Self::Other(engine) => engine.query_statement(statement, options).await,
        }
    }

    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        match self {
            Self::Sql(engine) => engine.execute(statement).await,
            Self::Redis(engine) => engine.execute(statement).await,
            Self::Other(engine) => engine.execute(statement).await,
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

/// The first cell of a one-row numeric result. Negative values (PostgreSQL's
/// "never analyzed" `-1`) count as unknown.
fn first_count(result: &QueryResult) -> Option<u64> {
    match result.rows.first()?.values.first()? {
        CellValue::Integer(value) => u64::try_from(*value).ok(),
        CellValue::Unsigned(value) => Some(*value),
        CellValue::Real(value) if *value >= 0.0 => Some(*value as u64),
        CellValue::Text(value) => value.trim().parse().ok(),
        _ => None,
    }
}
