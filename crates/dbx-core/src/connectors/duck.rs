use crate::{
    CellValue, ColumnInfo, ConnectionConfig, DatabaseKind, DbxError, Engine, EntityKind,
    ExecResult, QueryOptions, QueryResult, Result, RowData, SqlStatement, TableInfo, TableRef,
};
use async_trait::async_trait;
use duckdb::{Connection, params_from_iter, types::Value};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub(super) struct DuckEngine {
    connection: Arc<Mutex<Connection>>,
    database: String,
}
fn error(error: duckdb::Error) -> DbxError {
    DbxError::Query(error.to_string())
}
fn parameter(value: &CellValue) -> Result<Value> {
    Ok(match value {
        CellValue::Null => Value::Null,
        CellValue::Boolean(v) => Value::Boolean(*v),
        CellValue::Integer(v) => Value::BigInt(*v),
        CellValue::Unsigned(v) => Value::UBigInt(*v),
        CellValue::Real(v) => Value::Double(*v),
        CellValue::Text(v) => Value::Text(v.clone()),
        CellValue::Bytes(v) => Value::Blob(v.clone()),
        CellValue::Json(v) => Value::Text(v.to_string()),
    })
}
fn value(value: Value) -> CellValue {
    match value {
        Value::Null => CellValue::Null,
        Value::Boolean(v) => CellValue::Boolean(v),
        Value::TinyInt(v) => CellValue::Integer(v.into()),
        Value::SmallInt(v) => CellValue::Integer(v.into()),
        Value::Int(v) => CellValue::Integer(v.into()),
        Value::BigInt(v) => CellValue::Integer(v),
        Value::UTinyInt(v) => CellValue::Unsigned(v.into()),
        Value::USmallInt(v) => CellValue::Unsigned(v.into()),
        Value::UInt(v) => CellValue::Unsigned(v.into()),
        Value::UBigInt(v) => CellValue::Unsigned(v),
        Value::Float(v) => CellValue::Real(v.into()),
        Value::Double(v) => CellValue::Real(v),
        Value::Text(v) => CellValue::Text(v),
        Value::Blob(v) => CellValue::Bytes(v),
        Value::HugeInt(v) => CellValue::Text(v.to_string()),
        Value::Decimal(v) => CellValue::Text(v.to_string()),
        other => CellValue::Text(format!("{other:?}")),
    }
}
impl DuckEngine {
    pub async fn connect(config: ConnectionConfig) -> Result<Self> {
        let database = if config.url == "duckdb::memory:" {
            ":memory:".to_owned()
        } else {
            let url = url::Url::parse(&config.url)
                .map_err(|_| DbxError::InvalidConfig("Invalid DuckDB file URL".into()))?;
            if url.host_str().is_some() {
                return Err(DbxError::InvalidConfig(
                    "Use duckdb:///absolute/path.duckdb".into(),
                ));
            }
            let path = super::decode(url.path())?;
            if path.is_empty() {
                return Err(DbxError::InvalidConfig(
                    "DuckDB file path is required".into(),
                ));
            }
            path
        };
        let path = database.clone();
        let connection = tokio::task::spawn_blocking(move || {
            if path == ":memory:" {
                Connection::open_in_memory()
            } else {
                Connection::open(path)
            }
        })
        .await
        .map_err(|_| DbxError::Connection("DuckDB worker failed".into()))?
        .map_err(error)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            database,
        })
    }
}
#[async_trait]
impl Engine for DuckEngine {
    fn kind(&self) -> DatabaseKind {
        DatabaseKind::DuckDB
    }
    async fn current_database(&self) -> Result<String> {
        Ok(self.database.clone())
    }
    async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let result = self.query("SELECT table_schema, table_name, table_type FROM information_schema.tables ORDER BY table_schema, table_name",QueryOptions { max_rows: None }).await?;
        Ok(result
            .rows
            .iter()
            .map(|row| TableInfo {
                schema: Some(super::text(row, 0)),
                name: super::text(row, 1),
                kind: if super::text(row, 2) == "VIEW" {
                    EntityKind::View
                } else {
                    EntityKind::Table
                },
            })
            .collect())
    }
    async fn describe_table(&self, table: &TableRef) -> Result<Vec<ColumnInfo>> {
        let result = self.query_statement(&SqlStatement::new(
            "SELECT c.column_name, c.data_type, c.is_nullable, EXISTS (SELECT 1 FROM duckdb_constraints() d WHERE d.schema_name = c.table_schema AND d.table_name = c.table_name AND d.constraint_type = 'PRIMARY KEY' AND list_contains(d.constraint_column_names, c.column_name)), c.column_default FROM information_schema.columns c WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
            vec![CellValue::Text(table.schema.clone().unwrap_or_else(||"main".into())), CellValue::Text(table.name.clone())]),QueryOptions {max_rows:None}).await?;
        Ok(result
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let mut c = super::column(super::text(row, 0), super::text(row, 1), i);
                c.nullable = super::text(row, 2) == "YES";
                c.primary_key = super::text(row, 3) == "true";
                c.default_value = row
                    .values
                    .get(4)
                    .filter(|value| !matches!(value, CellValue::Null))
                    .map(ToString::to_string);
                c
            })
            .collect())
    }
    async fn query(&self, sql: &str, options: QueryOptions) -> Result<QueryResult> {
        self.query_statement(&SqlStatement::new(sql, Vec::new()), options)
            .await
    }
    async fn table_structure(&self, table: &TableRef) -> Result<crate::TableStructure> {
        let columns = self.describe_table(table).await?;
        let result=self.query_statement(&SqlStatement::new(
            "SELECT constraint_name, to_json(constraint_column_names), referenced_table, to_json(referenced_column_names) FROM duckdb_constraints() WHERE schema_name = ? AND table_name = ? AND constraint_type = 'FOREIGN KEY' ORDER BY constraint_index",
            vec![CellValue::Text(table.schema.clone().unwrap_or_else(||"main".into())),CellValue::Text(table.name.clone())]),QueryOptions {max_rows:None}).await?;
        let names = |row: &RowData, i| {
            serde_json::from_str::<Vec<String>>(&super::text(row, i))
                .map_err(|_| DbxError::Decode("Invalid DuckDB foreign key metadata".into()))
        };
        let foreign_keys = result
            .rows
            .iter()
            .map(|row| {
                Ok(crate::ForeignKeyInfo {
                    constraint_name: Some(super::text(row, 0)),
                    columns: names(row, 1)?,
                    referenced_schema: Some(table.schema.clone().unwrap_or_else(|| "main".into())),
                    referenced_table: super::text(row, 2),
                    referenced_columns: names(row, 3)?,
                    on_update: Some(crate::ReferentialAction::NoAction),
                    on_delete: Some(crate::ReferentialAction::NoAction),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let params = vec![
            CellValue::Text(table.schema.clone().unwrap_or_else(|| "main".into())),
            CellValue::Text(table.name.clone()),
        ];
        let constraints = self.query_statement(&SqlStatement::new("SELECT constraint_name, expression FROM duckdb_constraints() WHERE schema_name=? AND table_name=? AND constraint_type='CHECK' ORDER BY constraint_index", params.clone()), QueryOptions { max_rows: None }).await?;
        let checks = constraints
            .rows
            .iter()
            .map(|row| crate::CheckConstraintInfo {
                name: Some(super::text(row, 0)),
                expression: super::text(row, 1),
            })
            .collect();
        let sources = self.query_statement(&SqlStatement::new("SELECT sql FROM duckdb_tables() WHERE schema_name=? AND table_name=? UNION ALL SELECT sql FROM duckdb_views() WHERE schema_name=? AND view_name=?", params.iter().cloned().chain(params.iter().cloned()).collect()), QueryOptions::default()).await?;
        let definition = sources
            .rows
            .first()
            .and_then(|row| row.values.first())
            .map(ToString::to_string);
        let index_rows = self.query_statement(&SqlStatement::new("SELECT index_name, is_unique, is_primary, expressions, sql FROM duckdb_indexes() WHERE schema_name=? AND table_name=? ORDER BY index_name", params), QueryOptions { max_rows: None }).await?;
        let indexes = index_rows
            .rows
            .iter()
            .map(|row| crate::IndexInfo {
                name: super::text(row, 0),
                unique: super::text(row, 1) == "true",
                primary: super::text(row, 2) == "true",
                columns: Vec::new(),
                method: None,
                predicate: None,
                definition: Some(super::text(row, 4)),
            })
            .collect();
        Ok(crate::TableStructure {
            columns,
            foreign_keys,
            indexes,
            checks,
            definition,
        })
    }
    async fn query_statement(
        &self,
        statement: &SqlStatement,
        options: QueryOptions,
    ) -> Result<QueryResult> {
        if crate::split_sql_statements(&statement.sql).len() > 1 {
            return Err(DbxError::Parse(
                "Run one DuckDB SQL statement at a time".into(),
            ));
        }
        let connection = self.connection.clone();
        let statement = statement.clone();
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let connection = connection
                .lock()
                .map_err(|_| DbxError::Connection("DuckDB connection lock failed".into()))?;
            let params = statement
                .params
                .iter()
                .map(parameter)
                .collect::<Result<Vec<_>>>()?;
            let mut prepared = connection.prepare(&statement.sql).map_err(error)?;
            // DuckDB only exposes result metadata after execution.
            let mut cursor = prepared
                .query(params_from_iter(params.iter()))
                .map_err(error)?;
            let mut rows = Vec::new();
            let mut columns = Vec::new();
            let limit = crate::engine::row_limit(options).unwrap_or(usize::MAX);
            let mut truncated = false;
            while let Some(row) = cursor.next().map_err(error)? {
                if columns.is_empty() {
                    columns = row
                        .as_ref()
                        .column_names()
                        .iter()
                        .enumerate()
                        .map(|(i, n)| super::column(n, "DuckDB", i))
                        .collect();
                }
                if rows.len() >= limit {
                    truncated = true;
                    break;
                }
                rows.push(RowData::new(
                    (0..row.as_ref().column_count())
                        .map(|i| row.get::<_, Value>(i).map(value).map_err(error))
                        .collect::<Result<Vec<_>>>()?,
                ));
            }
            drop(cursor);
            if columns.is_empty() {
                columns = prepared
                    .column_names()
                    .iter()
                    .enumerate()
                    .map(|(i, n)| super::column(n, "DuckDB", i))
                    .collect();
            }
            Ok(crate::engine::query_result(
                columns, rows, None, truncated, started,
            ))
        })
        .await
        .map_err(|_| DbxError::Query("DuckDB query worker failed".into()))?
    }
    async fn execute(&self, statement: &SqlStatement) -> Result<ExecResult> {
        if crate::split_sql_statements(&statement.sql).len() > 1 {
            return Err(DbxError::Parse(
                "Run one DuckDB SQL statement at a time".into(),
            ));
        }
        let connection = self.connection.clone();
        let statement = statement.clone();
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let connection = connection
                .lock()
                .map_err(|_| DbxError::Connection("DuckDB lock failed".into()))?;
            let params = statement
                .params
                .iter()
                .map(parameter)
                .collect::<Result<Vec<_>>>()?;
            let count = connection
                .execute(&statement.sql, params_from_iter(params.iter()))
                .map_err(error)?;
            Ok(crate::engine::exec_result(count as u64, None, started))
        })
        .await
        .map_err(|_| DbxError::Query("DuckDB worker failed".into()))?
    }
}
