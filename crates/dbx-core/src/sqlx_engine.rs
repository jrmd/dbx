use std::time::Instant;

use futures_util::TryStreamExt;
use sqlx::{
    Column, MySql, MySqlPool, Postgres, Row, Sqlite, SqlitePool, TypeInfo, ValueRef,
    mysql::{MySqlArguments, MySqlPoolOptions, MySqlRow},
    postgres::{PgArguments, PgPool, PgPoolOptions, PgRow},
    sqlite::{SqliteArguments, SqlitePoolOptions, SqliteRow},
};
use tokio::sync::RwLock;

use crate::engine::{exec_result, query_result, row_limit};
use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, EntityKind, ExecResult,
    ForeignKeyInfo, QueryOptions, QueryResult, ReferentialAction, Result, RowData, SqlStatement,
    TableInfo, TableRef, TableStructure,
};
use async_trait::async_trait;

/// SQLx-backed engine for PostgreSQL, MySQL, and SQLite.
pub struct SqlxEngine {
    kind: DatabaseKind,
    /// Retained so `use_database` can rebuild a pool for another database
    /// while reusing the original host, credentials, and pool settings.
    config: ConnectionConfig,
    /// Guarded because `use_database` replaces the pool. Readers clone the
    /// pool (an `Arc`-backed handle) instead of holding the lock across a
    /// query, so in-flight queries never block a database switch for long.
    pool: RwLock<SqlxPool>,
}

/// Native SQLx pools for the supported SQL drivers.
///
/// SQLx's `AnyPool` is useful for common scalar types, but it intentionally
/// rejects several normal PostgreSQL/MySQL types while converting rows. Keeping
/// the concrete pool here lets DBX decode those values without giving up one
/// engine abstraction at the public API boundary.
#[derive(Clone)]
pub enum SqlxPool {
    Postgres(PgPool),
    MySql(MySqlPool),
    SQLite(SqlitePool),
}

impl std::fmt::Debug for SqlxPool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple(match self {
                Self::Postgres(_) => "Postgres",
                Self::MySql(_) => "MySql",
                Self::SQLite(_) => "SQLite",
            })
            .finish()
    }
}

impl std::fmt::Debug for SqlxEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqlxEngine")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl SqlxEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        if !config.kind.is_sql() {
            return Err(DbxError::InvalidConfig(format!(
                "SQLx cannot open a {} connection",
                config.kind
            )));
        }
        config.validate()?;
        let timeout = std::time::Duration::from_millis(config.connect_timeout_ms);
        // Every connection to `sqlite::memory:` owns a different database.
        // A one-connection pool keeps the in-memory database stable across
        // schema, query, and mutation calls. File-backed SQLite still honors
        // the configured pool size.
        let max_connections =
            if config.kind == DatabaseKind::SQLite && is_sqlite_memory_url(&config.url) {
                1
            } else {
                config.max_connections
            };
        let pool = match config.kind {
            DatabaseKind::PostgreSQL => SqlxPool::Postgres(
                PgPoolOptions::new()
                    .max_connections(max_connections)
                    .acquire_timeout(timeout)
                    .connect(&config.url)
                    .await
                    .map_err(|error| {
                        DbxError::Connection(crate::error::connection_message(&config.url, error))
                    })?,
            ),
            DatabaseKind::MySQL => SqlxPool::MySql(
                MySqlPoolOptions::new()
                    .max_connections(max_connections)
                    .acquire_timeout(timeout)
                    .connect(&config.url)
                    .await
                    .map_err(|error| {
                        DbxError::Connection(crate::error::connection_message(&config.url, error))
                    })?,
            ),
            DatabaseKind::SQLite => SqlxPool::SQLite(
                SqlitePoolOptions::new()
                    .max_connections(max_connections)
                    .acquire_timeout(timeout)
                    .connect(&config.url)
                    .await
                    .map_err(|error| {
                        DbxError::Connection(crate::error::connection_message(&config.url, error))
                    })?,
            ),
            DatabaseKind::Redis => unreachable!(),
        };
        Ok(Self {
            kind: config.kind,
            config,
            pool: RwLock::new(pool),
        })
    }

    pub fn kind(&self) -> DatabaseKind {
        self.kind
    }

    pub fn pool(&self) -> &RwLock<SqlxPool> {
        &self.pool
    }

    /// Snapshot the current pool handle. `SqlxPool` clones are cheap and stay
    /// valid even if a later `use_database` swaps the pool underneath.
    async fn pool_snapshot(&self) -> SqlxPool {
        self.pool.read().await.clone()
    }

    async fn query_with_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        let limit = row_limit(options);
        let mut columns = Vec::new();
        let mut output = Vec::with_capacity(limit.unwrap_or(64).min(1024));
        match &self.pool_snapshot().await {
            SqlxPool::Postgres(pool) => {
                let mut rows = bind_postgres_query(statement).fetch(pool);
                while let Some(row) = rows.try_next().await? {
                    if columns.is_empty() {
                        columns = result_columns(row.columns());
                    }
                    output.push(RowData::new(decode_postgres_row(&row)?));
                    if limit.is_some_and(|limit| output.len() >= limit) {
                        break;
                    }
                }
            }
            SqlxPool::MySql(pool) => {
                let mut rows = bind_mysql_query(statement).fetch(pool);
                while let Some(row) = rows.try_next().await? {
                    if columns.is_empty() {
                        columns = result_columns(row.columns());
                    }
                    output.push(RowData::new(decode_mysql_row(&row)?));
                    if limit.is_some_and(|limit| output.len() >= limit) {
                        break;
                    }
                }
            }
            SqlxPool::SQLite(pool) => {
                let mut rows = bind_sqlite_query(statement).fetch(pool);
                while let Some(row) = rows.try_next().await? {
                    if columns.is_empty() {
                        columns = result_columns(row.columns());
                    }
                    output.push(RowData::new(decode_sqlite_row(&row)?));
                    if limit.is_some_and(|limit| output.len() >= limit) {
                        break;
                    }
                }
            }
        }
        Ok(query_result(columns, output, None, started))
    }

    async fn execute_statement(&self, statement: &SqlStatement) -> Result<ExecResult> {
        let started = Instant::now();
        let (rows_affected, last_insert_id) = match &self.pool_snapshot().await {
            SqlxPool::Postgres(pool) => {
                let result = bind_postgres_query(statement).execute(pool).await?;
                (result.rows_affected(), None)
            }
            SqlxPool::MySql(pool) => {
                let result = bind_mysql_query(statement).execute(pool).await?;
                // MySQL exposes this as u64. Going through AnyPool converted
                // it to i64 first, which silently lost IDs above i64::MAX.
                (result.rows_affected(), Some(result.last_insert_id()))
            }
            SqlxPool::SQLite(pool) => {
                let result = bind_sqlite_query(statement).execute(pool).await?;
                (result.rows_affected(), None)
            }
        };
        Ok(exec_result(rows_affected, last_insert_id, started))
    }

    async fn metadata_query(&self, sql: &str, params: &[CellValue]) -> Result<QueryResult> {
        self.query_with_statement(
            &SqlStatement::new(sql, params.to_vec()),
            QueryOptions { max_rows: None },
        )
        .await
    }

    async fn list_sql_tables(&self) -> Result<Vec<TableInfo>> {
        let result = match self.kind {
            DatabaseKind::SQLite => {
                self.metadata_query(
                    "SELECT name, type FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY name",
                    &[],
                )
                .await?
            }
            DatabaseKind::PostgreSQL => {
                self.metadata_query(
                    "SELECT table_schema, table_name, table_type FROM information_schema.tables WHERE table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY table_schema, table_name",
                    &[],
                )
                .await?
            }
            DatabaseKind::MySQL => {
                self.metadata_query(
                    "SELECT CAST(TABLE_SCHEMA AS CHAR) AS table_schema, CAST(TABLE_NAME AS CHAR) AS table_name, CAST(TABLE_TYPE AS CHAR) AS table_type FROM information_schema.tables WHERE TABLE_SCHEMA = DATABASE() ORDER BY TABLE_NAME",
                    &[],
                )
                .await?
            }
            DatabaseKind::Redis => unreachable!(),
        };
        let mut tables = Vec::with_capacity(result.rows.len());
        for row in result.rows {
            let (schema, name, entity_kind) = match self.kind {
                DatabaseKind::SQLite => (
                    None,
                    text_value(&row, 0)?,
                    match text_value(&row, 1)?.to_ascii_lowercase().as_str() {
                        "view" => EntityKind::View,
                        _ => EntityKind::Table,
                    },
                ),
                DatabaseKind::PostgreSQL | DatabaseKind::MySQL => (
                    Some(text_value(&row, 0)?),
                    text_value(&row, 1)?,
                    match text_value(&row, 2)?.to_ascii_lowercase().as_str() {
                        "view" => EntityKind::View,
                        _ => EntityKind::Table,
                    },
                ),
                DatabaseKind::Redis => unreachable!(),
            };
            tables.push(TableInfo {
                name,
                schema,
                kind: entity_kind,
            });
        }
        Ok(tables)
    }

    async fn describe_sql_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        let result = match self.kind {
            DatabaseKind::SQLite => {
                // PRAGMA accepts a quoted string for the table name. Escaping
                // here prevents a table name from changing the pragma query.
                let escaped = table.name.replace('\'', "''");
                self.metadata_query(
                    &format!("PRAGMA table_info('{escaped}')"),
                    &[],
                )
                .await?
            }
            DatabaseKind::PostgreSQL => {
                let schema = table.schema.clone().unwrap_or_else(|| "public".to_owned());
                self.metadata_query(
                    "SELECT c.column_name, c.data_type, c.is_nullable, c.ordinal_position, CASE WHEN EXISTS (SELECT 1 FROM information_schema.key_column_usage kcu JOIN information_schema.table_constraints tc ON tc.constraint_schema = kcu.constraint_schema AND tc.constraint_name = kcu.constraint_name WHERE kcu.table_schema = c.table_schema AND kcu.table_name = c.table_name AND kcu.column_name = c.column_name AND tc.constraint_type = 'PRIMARY KEY') THEN TRUE ELSE FALSE END AS is_primary_key FROM information_schema.columns c WHERE c.table_schema = $1 AND c.table_name = $2 ORDER BY c.ordinal_position",
                    &[CellValue::Text(schema), CellValue::Text(table.name.clone())],
                )
                .await?
            }
            DatabaseKind::MySQL => {
                self.metadata_query(
                    "SELECT CAST(COLUMN_NAME AS CHAR) AS column_name, CAST(DATA_TYPE AS CHAR) AS data_type, CAST(IS_NULLABLE AS CHAR) AS is_nullable, ORDINAL_POSITION, CASE WHEN COLUMN_KEY = 'PRI' THEN TRUE ELSE FALSE END AS is_primary_key FROM information_schema.columns WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    &[CellValue::Text(table.name.clone())],
                )
                .await?
            }
            DatabaseKind::Redis => unreachable!(),
        };
        let mut columns = Vec::with_capacity(result.rows.len());
        for (index, row) in result.rows.iter().enumerate() {
            let (name, data_type, nullable, ordinal, primary_key) = match self.kind {
                DatabaseKind::SQLite => {
                    let primary_key = boolish_value(row, 5)?;
                    (
                        text_value(row, 1)?,
                        text_value(row, 2)?,
                        !boolish_value(row, 3)? && !primary_key,
                        integer_value(row, 0)?.max(0) as usize,
                        primary_key,
                    )
                }
                DatabaseKind::PostgreSQL | DatabaseKind::MySQL => (
                    text_value(row, 0)?,
                    text_value(row, 1)?,
                    text_value(row, 2)?.eq_ignore_ascii_case("yes")
                        || text_value(row, 2)?.eq_ignore_ascii_case("true"),
                    integer_value(row, 3)?.max(1) as usize,
                    boolish_value(row, 4)?,
                ),
                DatabaseKind::Redis => unreachable!(),
            };
            columns.push(ColumnInfo {
                name,
                data_type,
                nullable,
                ordinal: if ordinal == 0 { index + 1 } else { ordinal },
                primary_key,
            });
        }
        Ok(columns)
    }

    async fn foreign_keys(&self, table: &TableRef) -> Result<Vec<ForeignKeyInfo>> {
        let result = match self.kind {
            DatabaseKind::SQLite => {
                let escaped = table.name.replace('\'', "''");
                self.metadata_query(&format!("PRAGMA foreign_key_list('{escaped}')"), &[])
                    .await?
            }
            DatabaseKind::PostgreSQL => {
                let schema = table.schema.clone().unwrap_or_else(|| "public".to_owned());
                self.metadata_query(
                    "SELECT tc.constraint_name, kcu.column_name, ccu.table_schema, ccu.table_name, ccu.column_name, rc.update_rule, rc.delete_rule FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage kcu ON kcu.constraint_catalog = tc.constraint_catalog AND kcu.constraint_schema = tc.constraint_schema AND kcu.constraint_name = tc.constraint_name JOIN information_schema.referential_constraints rc ON rc.constraint_catalog = tc.constraint_catalog AND rc.constraint_schema = tc.constraint_schema AND rc.constraint_name = tc.constraint_name JOIN information_schema.key_column_usage ccu ON ccu.constraint_catalog = rc.unique_constraint_catalog AND ccu.constraint_schema = rc.unique_constraint_schema AND ccu.constraint_name = rc.unique_constraint_name AND ccu.ordinal_position = kcu.position_in_unique_constraint WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = $1 AND tc.table_name = $2 ORDER BY tc.constraint_name, kcu.ordinal_position",
                    &[CellValue::Text(schema), CellValue::Text(table.name.clone())],
                )
                .await?
            }
            DatabaseKind::MySQL => {
                self.metadata_query(
                    "SELECT CAST(kcu.CONSTRAINT_NAME AS CHAR), CAST(kcu.COLUMN_NAME AS CHAR), CAST(kcu.REFERENCED_TABLE_SCHEMA AS CHAR), CAST(kcu.REFERENCED_TABLE_NAME AS CHAR), CAST(kcu.REFERENCED_COLUMN_NAME AS CHAR), CAST(rc.UPDATE_RULE AS CHAR), CAST(rc.DELETE_RULE AS CHAR) FROM information_schema.key_column_usage kcu LEFT JOIN information_schema.referential_constraints rc ON rc.CONSTRAINT_SCHEMA = kcu.CONSTRAINT_SCHEMA AND rc.CONSTRAINT_NAME = kcu.CONSTRAINT_NAME AND rc.TABLE_NAME = kcu.TABLE_NAME WHERE kcu.TABLE_SCHEMA = DATABASE() AND kcu.TABLE_NAME = ? AND kcu.REFERENCED_TABLE_NAME IS NOT NULL ORDER BY kcu.CONSTRAINT_NAME, kcu.ORDINAL_POSITION",
                    &[CellValue::Text(table.name.clone())],
                )
                .await?
            }
            DatabaseKind::Redis => unreachable!(),
        };
        foreign_keys_from_rows(self.kind, result.rows)
    }

    async fn table_structure_sql(&self, table: &TableRef) -> Result<TableStructure> {
        Ok(TableStructure {
            columns: self.describe_sql_table(table).await?,
            foreign_keys: self.foreign_keys(table).await?,
        })
    }

    async fn list_sql_databases(&self) -> Result<Vec<String>> {
        let result = match self.kind {
            DatabaseKind::SQLite => {
                self.metadata_query("PRAGMA database_list", &[]).await?
            }
            DatabaseKind::PostgreSQL => {
                self.metadata_query(
                    "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
                    &[],
                )
                .await?
            }
            DatabaseKind::MySQL => {
                self.metadata_query(
                    "SELECT CAST(SCHEMA_NAME AS CHAR) AS schema_name FROM information_schema.schemata WHERE SCHEMA_NAME NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') ORDER BY SCHEMA_NAME",
                    &[],
                )
                .await?
            }
            DatabaseKind::Redis => unreachable!(),
        };
        let mut names = Vec::with_capacity(result.rows.len());
        for row in result.rows {
            // PRAGMA database_list exposes (seq, name, file); both other
            // queries return a single name column.
            let column = if self.kind == DatabaseKind::SQLite {
                1
            } else {
                0
            };
            names.push(text_value(&row, column)?);
        }
        Ok(names)
    }

    async fn current_sql_database(&self) -> Result<String> {
        match self.kind {
            DatabaseKind::SQLite => Ok("main".to_owned()),
            DatabaseKind::PostgreSQL => {
                let result = self
                    .metadata_query("SELECT current_database()", &[])
                    .await?;
                result
                    .rows
                    .first()
                    .map(|row| text_value(row, 0))
                    .transpose()
                    .map(|value| value.unwrap_or_else(|| "postgres".to_owned()))
            }
            DatabaseKind::MySQL => {
                let result = self.metadata_query("SELECT DATABASE()", &[]).await?;
                result
                    .rows
                    .first()
                    .map(|row| text_value(row, 0))
                    .transpose()
                    .map(|value| value.unwrap_or_default())
            }
            DatabaseKind::Redis => unreachable!(),
        }
    }

    async fn use_sql_database(&self, name: &str) -> Result<()> {
        validate_database_name(name)?;
        match self.kind {
            DatabaseKind::SQLite => Err(DbxError::Unsupported {
                operation: "use_database".to_owned(),
                kind: self.kind,
            }),
            DatabaseKind::MySQL => {
                let escaped = name.replace('`', "``");
                self.execute_statement(&SqlStatement::new(format!("USE `{escaped}`"), Vec::new()))
                    .await?;
                Ok(())
            }
            DatabaseKind::PostgreSQL => {
                // PostgreSQL cannot change database on a live connection, so
                // swap in a pool connected to the target database. Callers
                // keep using the same engine object.
                let url = with_database_path(&self.config.url, name)?;
                let timeout = std::time::Duration::from_millis(self.config.connect_timeout_ms);
                let pool = PgPoolOptions::new()
                    .max_connections(self.config.max_connections)
                    .acquire_timeout(timeout)
                    .connect(&url)
                    .await
                    .map_err(|error| DbxError::Connection(error.to_string()))?;
                *self.pool.write().await = SqlxPool::Postgres(pool);
                Ok(())
            }
            DatabaseKind::Redis => unreachable!(),
        }
    }
}

#[async_trait]
impl crate::Engine for SqlxEngine {
    fn kind(&self) -> DatabaseKind {
        self.kind
    }

    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        self.list_sql_tables().await
    }

    async fn list_databases(&self) -> Result<Vec<String>> {
        self.list_sql_databases().await
    }

    async fn current_database(&self) -> Result<String> {
        self.current_sql_database().await
    }

    async fn use_database(&self, name: &str) -> Result<()> {
        self.use_sql_database(name).await
    }

    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        self.describe_sql_table(table).await
    }

    async fn table_structure(&self, table: &TableRef) -> Result<TableStructure> {
        self.table_structure_sql(table).await
    }

    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        self.query_with_statement(&SqlStatement::new(sql, Vec::new()), options)
            .await
    }

    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        self.query_with_statement(statement, options).await
    }

    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        self.execute_statement(statement).await
    }
}

fn is_sqlite_memory_url(url: &str) -> bool {
    url.to_ascii_lowercase().contains(":memory:")
}

/// Reject names that could break out of quoting or URL rewriting before they
/// reach a driver.
fn validate_database_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(DbxError::InvalidConfig(
            "database name cannot be empty".into(),
        ));
    }
    if trimmed.len() > 128
        || trimmed.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || matches!(character, '\0' | ';' | '/')
        })
    {
        return Err(DbxError::InvalidConfig(format!(
            "invalid database name `{name}`"
        )));
    }
    Ok(())
}

/// Rewrite the path of a PostgreSQL URL to point at another database while
/// preserving host, credentials, and query options.
fn with_database_path(base: &str, database: &str) -> Result<String> {
    let mut parsed =
        url::Url::parse(base).map_err(|error| DbxError::InvalidConfig(error.to_string()))?;
    parsed.set_path(&format!("/{database}"));
    Ok(parsed.to_string())
}

fn foreign_keys_from_rows(kind: DatabaseKind, rows: Vec<RowData>) -> Result<Vec<ForeignKeyInfo>> {
    match kind {
        DatabaseKind::SQLite => {
            let mut foreign_keys: Vec<(i64, ForeignKeyInfo)> = Vec::new();
            for row in rows {
                let id = integer_value(&row, 0)?;
                let local_column = text_value(&row, 3)?;
                let referenced_column = optional_text_value(&row, 4)?;
                if let Some((_, foreign_key)) = foreign_keys
                    .last_mut()
                    .filter(|(last_id, _)| *last_id == id)
                {
                    foreign_key.columns.push(local_column);
                    if let Some(referenced_column) = referenced_column {
                        foreign_key.referenced_columns.push(referenced_column);
                    }
                    continue;
                }
                foreign_keys.push((
                    id,
                    ForeignKeyInfo {
                        constraint_name: None,
                        columns: vec![local_column],
                        referenced_schema: None,
                        referenced_table: text_value(&row, 2)?,
                        referenced_columns: referenced_column.into_iter().collect(),
                        on_update: optional_text_value(&row, 5)?
                            .and_then(|value| ReferentialAction::from_metadata(&value)),
                        on_delete: optional_text_value(&row, 6)?
                            .and_then(|value| ReferentialAction::from_metadata(&value)),
                    },
                ));
            }
            Ok(foreign_keys
                .into_iter()
                .map(|(_, foreign_key)| foreign_key)
                .collect())
        }
        DatabaseKind::PostgreSQL | DatabaseKind::MySQL => {
            let mut foreign_keys: Vec<ForeignKeyInfo> = Vec::new();
            for row in rows {
                let constraint_name = optional_text_value(&row, 0)?;
                let local_column = text_value(&row, 1)?;
                let referenced_column = text_value(&row, 4)?;
                if let Some(foreign_key) = foreign_keys
                    .last_mut()
                    .filter(|foreign_key| foreign_key.constraint_name == constraint_name)
                {
                    foreign_key.columns.push(local_column);
                    foreign_key.referenced_columns.push(referenced_column);
                    continue;
                }
                foreign_keys.push(ForeignKeyInfo {
                    constraint_name,
                    columns: vec![local_column],
                    referenced_schema: optional_text_value(&row, 2)?,
                    referenced_table: text_value(&row, 3)?,
                    referenced_columns: vec![referenced_column],
                    on_update: optional_text_value(&row, 5)?
                        .and_then(|value| ReferentialAction::from_metadata(&value)),
                    on_delete: optional_text_value(&row, 6)?
                        .and_then(|value| ReferentialAction::from_metadata(&value)),
                });
            }
            Ok(foreign_keys)
        }
        DatabaseKind::Redis => unreachable!(),
    }
}

fn result_columns<C>(columns: &[C]) -> Vec<ColumnInfo>
where
    C: Column,
{
    columns
        .iter()
        .enumerate()
        .map(|(ordinal, column)| {
            ColumnInfo::result(column.name(), ordinal, column.type_info().name())
        })
        .collect()
}

fn bind_postgres_query<'q>(
    statement: &'q SqlStatement,
) -> sqlx::query::Query<'q, Postgres, PgArguments> {
    let mut query = sqlx::query::<Postgres>(statement.sql.as_str());
    for value in &statement.params {
        query = match value {
            // A NULL has no value to encode. String is used only as the
            // fallback parameter type; PostgreSQL still transmits a NULL
            // marker and coerces it in normal INSERT/UPDATE contexts.
            CellValue::Null => query.bind(Option::<String>::None),
            CellValue::Boolean(value) => query.bind(*value),
            CellValue::Integer(value) => query.bind(*value),
            CellValue::Unsigned(value) => match i64::try_from(*value) {
                Ok(value) => query.bind(value),
                // PostgreSQL has no unsigned integer type. Keeping an
                // out-of-range value as text avoids wrapping it into a
                // negative BIGINT; NUMERIC columns can coerce it exactly.
                Err(_) => query.bind(value.to_string()),
            },
            CellValue::Real(value) => query.bind(*value),
            CellValue::Text(value) => query.bind(value.clone()),
            CellValue::Bytes(value) => query.bind(value.clone()),
            CellValue::Json(value) => query.bind(sqlx::types::Json(value.clone())),
        };
    }
    query
}

fn bind_mysql_query<'q>(
    statement: &'q SqlStatement,
) -> sqlx::query::Query<'q, MySql, MySqlArguments> {
    let mut query = sqlx::query::<MySql>(statement.sql.as_str());
    for value in &statement.params {
        query = match value {
            CellValue::Null => query.bind(Option::<String>::None),
            CellValue::Boolean(value) => query.bind(*value),
            CellValue::Integer(value) => query.bind(*value),
            // MySQL supports BIGINT UNSIGNED, so preserve the full u64.
            CellValue::Unsigned(value) => query.bind(*value),
            CellValue::Real(value) => query.bind(*value),
            CellValue::Text(value) => query.bind(value.clone()),
            CellValue::Bytes(value) => query.bind(value.clone()),
            CellValue::Json(value) => query.bind(sqlx::types::Json(value.clone())),
        };
    }
    query
}

fn bind_sqlite_query<'q>(
    statement: &'q SqlStatement,
) -> sqlx::query::Query<'q, Sqlite, SqliteArguments<'q>> {
    let mut query = sqlx::query::<Sqlite>(statement.sql.as_str());
    for value in &statement.params {
        query = match value {
            CellValue::Null => query.bind(Option::<String>::None),
            // SQLite stores booleans as integer values.
            CellValue::Boolean(value) => query.bind(i64::from(*value)),
            CellValue::Integer(value) => query.bind(*value),
            CellValue::Unsigned(value) => match i64::try_from(*value) {
                Ok(value) => query.bind(value),
                // SQLite's integer representation is signed. Do not wrap a
                // large unsigned value into a negative integer.
                Err(_) => query.bind(value.to_string()),
            },
            CellValue::Real(value) => query.bind(*value),
            CellValue::Text(value) => query.bind(value.clone()),
            CellValue::Bytes(value) => query.bind(value.clone()),
            CellValue::Json(value) => query.bind(sqlx::types::Json(value.clone())),
        };
    }
    query
}

fn decode_postgres_row(row: &PgRow) -> Result<Vec<CellValue>> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, column)| decode_postgres_cell(row, index, column.type_info().name()))
        .collect()
}

fn decode_mysql_row(row: &MySqlRow) -> Result<Vec<CellValue>> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, column)| decode_mysql_cell(row, index, column.type_info().name()))
        .collect()
}

fn decode_sqlite_row(row: &SqliteRow) -> Result<Vec<CellValue>> {
    row.columns()
        .iter()
        .enumerate()
        .map(|(index, column)| decode_sqlite_cell(row, index, column.type_info().name()))
        .collect()
}

fn decode_postgres_cell(row: &PgRow, index: usize, type_name: &str) -> Result<CellValue> {
    let type_name = type_name.to_ascii_lowercase();
    if (type_name == "bool" || type_name == "boolean")
        && let Ok(value) = row.try_get::<Option<bool>, _>(index)
    {
        return Ok(value.map(CellValue::Boolean).unwrap_or(CellValue::Null));
    }
    if is_postgres_integer(&type_name) {
        let value = match type_name.as_str() {
            "smallint" | "int2" => row
                .try_get::<Option<i16>, _>(index)
                .ok()
                .map(|value| value.map(i64::from)),
            "integer" | "int4" | "serial" => row
                .try_get::<Option<i32>, _>(index)
                .ok()
                .map(|value| value.map(i64::from)),
            "bigint" | "int8" | "bigserial" => row.try_get::<Option<i64>, _>(index).ok(),
            _ => None,
        };
        if let Some(value) = value {
            return Ok(value.map(CellValue::Integer).unwrap_or(CellValue::Null));
        }
    }
    if is_postgres_decimal(&type_name)
        && let Ok(value) = row.try_get::<Option<sqlx::types::BigDecimal>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if (type_name == "real" || type_name == "float4")
        && let Ok(value) = row.try_get::<Option<f32>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Real(f64::from(value)))
            .unwrap_or(CellValue::Null));
    }
    if (type_name == "double precision" || type_name == "float8")
        && let Ok(value) = row.try_get::<Option<f64>, _>(index)
    {
        return Ok(value.map(CellValue::Real).unwrap_or(CellValue::Null));
    }
    if type_name == "bytea"
        && let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index)
    {
        return Ok(value.map(CellValue::Bytes).unwrap_or(CellValue::Null));
    }
    if (type_name == "json" || type_name == "jsonb")
        && let Ok(value) = row.try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Json(value.0))
            .unwrap_or(CellValue::Null));
    }
    if type_name == "uuid"
        && let Ok(value) = row.try_get::<Option<sqlx::types::Uuid>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if type_name == "date"
        && let Ok(value) = row.try_get::<Option<sqlx::types::chrono::NaiveDate>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if type_name == "time"
        && let Ok(value) = row.try_get::<Option<sqlx::types::chrono::NaiveTime>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if type_name == "timestamp"
        && let Ok(value) = row.try_get::<Option<sqlx::types::chrono::NaiveDateTime>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if (type_name == "timestamp with time zone" || type_name == "timestamptz")
        && let Ok(value) =
            row.try_get::<Option<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_rfc3339()))
            .unwrap_or(CellValue::Null));
    }
    decode_text_or_bytes_pg(row, index, &type_name)
}

fn decode_mysql_cell(row: &MySqlRow, index: usize, type_name: &str) -> Result<CellValue> {
    let type_name = type_name.to_ascii_lowercase();
    if (type_name.contains("bool") || type_name == "tinyint(1)")
        && let Ok(value) = row.try_get::<Option<bool>, _>(index)
    {
        return Ok(value.map(CellValue::Boolean).unwrap_or(CellValue::Null));
    }
    if type_name.contains("int") || type_name.contains("year") {
        if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
            return Ok(value.map(CellValue::Integer).unwrap_or(CellValue::Null));
        }
        // BIGINT UNSIGNED values above i64::MAX must remain unsigned.
        if let Ok(value) = row.try_get::<Option<u64>, _>(index) {
            return Ok(value.map(CellValue::Unsigned).unwrap_or(CellValue::Null));
        }
    }
    if (type_name.contains("decimal") || type_name.contains("numeric"))
        && let Ok(value) = row.try_get::<Option<sqlx::types::BigDecimal>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if (type_name == "float" || type_name == "float4")
        && let Ok(value) = row.try_get::<Option<f32>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Real(f64::from(value)))
            .unwrap_or(CellValue::Null));
    }
    if (type_name == "double" || type_name == "double precision" || type_name == "real")
        && let Ok(value) = row.try_get::<Option<f64>, _>(index)
    {
        return Ok(value.map(CellValue::Real).unwrap_or(CellValue::Null));
    }
    if (type_name.contains("blob") || type_name.contains("binary") || type_name == "bit")
        && let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index)
    {
        return Ok(value.map(CellValue::Bytes).unwrap_or(CellValue::Null));
    }
    if type_name == "json"
        && let Ok(value) = row.try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Json(value.0))
            .unwrap_or(CellValue::Null));
    }
    if type_name == "date"
        && let Ok(value) = row.try_get::<Option<sqlx::types::chrono::NaiveDate>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if (type_name == "datetime" || type_name == "timestamp")
        && let Ok(value) = row.try_get::<Option<sqlx::types::chrono::NaiveDateTime>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    if type_name == "time"
        && let Ok(value) = row.try_get::<Option<sqlx::types::chrono::NaiveTime>, _>(index)
    {
        return Ok(value
            .map(|value| CellValue::Text(value.to_string()))
            .unwrap_or(CellValue::Null));
    }
    decode_text_or_bytes_mysql(row, index, &type_name)
}

fn decode_sqlite_cell(row: &SqliteRow, index: usize, type_name: &str) -> Result<CellValue> {
    let type_name = type_name.to_ascii_lowercase();
    let unknown_type = type_name == "null";
    // SQLite reports NULL for the dynamic type of some PRAGMA/result
    // columns. Inspect the value before falling back so metadata such as
    // `cid` and `pk` remains numeric rather than becoming an error cell.
    if unknown_type && let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return Ok(value.map(CellValue::Integer).unwrap_or(CellValue::Null));
    }
    if type_name.contains("bool")
        && let Ok(value) = row.try_get::<Option<bool>, _>(index)
    {
        return Ok(value.map(CellValue::Boolean).unwrap_or(CellValue::Null));
    }
    if type_name.contains("int")
        && let Ok(value) = row.try_get::<Option<i64>, _>(index)
    {
        return Ok(value.map(CellValue::Integer).unwrap_or(CellValue::Null));
    }
    if (type_name.contains("real") || type_name.contains("float") || type_name == "numeric")
        && let Ok(value) = row.try_get::<Option<f64>, _>(index)
    {
        return Ok(value.map(CellValue::Real).unwrap_or(CellValue::Null));
    }
    if type_name.contains("blob")
        && let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index)
    {
        return Ok(value.map(CellValue::Bytes).unwrap_or(CellValue::Null));
    }
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return Ok(value.map(CellValue::Text).unwrap_or(CellValue::Null));
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return Ok(value.map(CellValue::Bytes).unwrap_or(CellValue::Null));
    }
    Ok(CellValue::Text(format!(
        "<unsupported SQL type `{type_name}`>"
    )))
}

fn decode_text_or_bytes_pg(row: &PgRow, index: usize, type_name: &str) -> Result<CellValue> {
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return Ok(value.map(CellValue::Text).unwrap_or(CellValue::Null));
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return Ok(value.map(CellValue::Bytes).unwrap_or(CellValue::Null));
    }
    // Custom types such as PostgreSQL enums are transmitted as text but do
    // not match any built-in Rust decoder, so the typed `try_get` calls
    // above reject them. Read the raw wire value so enum labels stay usable
    // instead of surfacing an unsupported-type cell.
    match row.try_get_raw(index) {
        Ok(value) if value.is_null() => Ok(CellValue::Null),
        Ok(value) => match value.as_str() {
            Ok(text) => Ok(CellValue::Text(text.to_owned())),
            Err(_) => match value.as_bytes() {
                Ok(bytes) => Ok(CellValue::Bytes(bytes.to_vec())),
                Err(_) => Ok(CellValue::Text(format!(
                    "<unsupported SQL type `{type_name}`>"
                ))),
            },
        },
        Err(_) => Ok(CellValue::Text(format!(
            "<unsupported SQL type `{type_name}`>"
        ))),
    }
}

fn decode_text_or_bytes_mysql(row: &MySqlRow, index: usize, type_name: &str) -> Result<CellValue> {
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return Ok(value.map(CellValue::Text).unwrap_or(CellValue::Null));
    }
    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        // MySQL can expose textual information_schema columns through its
        // binary decoder (notably TABLE_SCHEMA/TABLE_NAME). Preserve actual
        // binary data, while recovering valid UTF-8 labels as text so table
        // discovery does not render names as hexadecimal byte strings.
        return Ok(value.map(mysql_bytes_fallback).unwrap_or(CellValue::Null));
    }
    Ok(CellValue::Text(format!(
        "<unsupported SQL type `{type_name}`>"
    )))
}

fn mysql_bytes_fallback(value: Vec<u8>) -> CellValue {
    match String::from_utf8(value) {
        Ok(value) => CellValue::Text(value),
        Err(error) => CellValue::Bytes(error.into_bytes()),
    }
}

fn is_postgres_integer(type_name: &str) -> bool {
    matches!(
        type_name,
        "smallint" | "int2" | "integer" | "int4" | "bigint" | "int8" | "serial" | "bigserial"
    )
}

fn is_postgres_decimal(type_name: &str) -> bool {
    matches!(type_name, "numeric" | "decimal")
}

fn text_value(row: &RowData, index: usize) -> Result<String> {
    match row.values.get(index) {
        Some(CellValue::Text(value)) => Ok(value.clone()),
        Some(CellValue::Json(value)) => Ok(value.to_string()),
        Some(other) => Ok(other.to_string()),
        None => Err(DbxError::Decode(format!(
            "metadata row missing column {index}"
        ))),
    }
}

fn optional_text_value(row: &RowData, index: usize) -> Result<Option<String>> {
    match row.values.get(index) {
        Some(CellValue::Null) => Ok(None),
        Some(_) => text_value(row, index).map(Some),
        None => Err(DbxError::Decode(format!(
            "metadata row missing column {index}"
        ))),
    }
}

fn integer_value(row: &RowData, index: usize) -> Result<i64> {
    match row.values.get(index) {
        Some(CellValue::Null) => Ok(0),
        Some(CellValue::Integer(value)) => Ok(*value),
        Some(CellValue::Unsigned(value)) => Ok((*value).min(i64::MAX as u64) as i64),
        Some(CellValue::Text(value)) => value.parse::<i64>().map_err(|error| {
            DbxError::Decode(format!(
                "metadata value `{value}` at column {index}: {error}"
            ))
        }),
        Some(other) => other.to_string().parse::<i64>().map_err(|error| {
            DbxError::Decode(format!(
                "metadata value `{other}` at column {index}: {error}"
            ))
        }),
        None => Err(DbxError::Decode(format!(
            "metadata row missing column {index}"
        ))),
    }
}

fn boolish_value(row: &RowData, index: usize) -> Result<bool> {
    match row.values.get(index) {
        Some(CellValue::Boolean(value)) => Ok(*value),
        Some(CellValue::Integer(value)) => Ok(*value != 0),
        Some(CellValue::Unsigned(value)) => Ok(*value != 0),
        Some(CellValue::Text(value)) => Ok(matches!(
            value.to_ascii_lowercase().as_str(),
            "1" | "true" | "t" | "yes" | "y"
        )),
        Some(CellValue::Null) | None => Ok(false),
        Some(other) => Err(DbxError::Decode(format!(
            "cannot decode `{other}` as boolean"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_byte_fallback_decodes_metadata_text_and_preserves_binary() {
        assert_eq!(
            mysql_bytes_fallback(b"dbx_integration_rows".to_vec()),
            CellValue::Text("dbx_integration_rows".into())
        );
        assert_eq!(
            mysql_bytes_fallback(vec![0xff, 0x00, 0xfe]),
            CellValue::Bytes(vec![0xff, 0x00, 0xfe])
        );
    }

    #[test]
    fn database_name_validation_rejects_injection_shapes() {
        assert!(validate_database_name("app_db").is_ok());
        assert!(validate_database_name("db-1.prod").is_ok());
        assert!(validate_database_name("").is_err());
        assert!(validate_database_name("  ").is_err());
        assert!(validate_database_name("a;b").is_err());
        assert!(validate_database_name("a/b").is_err());
        assert!(validate_database_name("a b").is_err());
        assert!(validate_database_name("a\nb").is_err());
    }

    #[test]
    fn postgres_url_rewrite_targets_another_database() {
        let rewritten = with_database_path(
            "postgres://user:secret@db.example.test:5432/primary?sslmode=require",
            "analytics",
        )
        .unwrap();
        assert!(rewritten.starts_with("postgres://user:secret@db.example.test:5432/analytics"));
        assert!(rewritten.contains("sslmode=require"));
    }
}
