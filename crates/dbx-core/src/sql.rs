use std::fmt::Write;

use crate::{
    CellValue, ColumnInfo, CreateTableRequest, DatabaseKind, DbxError, Filter, FilterOperator,
    InsertRequest, MutationValue, Order, OrderDirection, Page, Result, TableRef, UpdateRequest,
};

/// A parameterized SQL statement. Values are kept separately so a caller can
/// inspect or log the statement without interpolating user data into SQL.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlStatement {
    pub sql: String,
    pub params: Vec<CellValue>,
}

impl SqlStatement {
    pub fn new(sql: impl Into<String>, params: Vec<CellValue>) -> Self {
        Self {
            sql: sql.into(),
            params,
        }
    }
}

/// Quote a table/column identifier for a SQL dialect. Dotted identifiers are
/// quoted segment-by-segment so `public.users` remains addressable.
pub fn quote_identifier(kind: DatabaseKind, identifier: &str) -> Result<String> {
    if identifier.trim().is_empty() {
        return Err(DbxError::Parse("identifier cannot be empty".into()));
    }
    let quote = if kind == DatabaseKind::MySQL {
        '`'
    } else {
        '"'
    };
    let mut output = String::new();
    for (index, part) in identifier.split('.').enumerate() {
        if part.is_empty() || part.contains('\0') {
            return Err(DbxError::Parse(format!(
                "invalid identifier `{identifier}`"
            )));
        }
        if index > 0 {
            output.push('.');
        }
        output.push(quote);
        for character in part.chars() {
            if character == quote {
                output.push(quote);
            }
            output.push(character);
        }
        output.push(quote);
    }
    Ok(output)
}

pub fn quote_table(kind: DatabaseKind, table: &TableRef) -> Result<String> {
    match &table.schema {
        Some(schema) => quote_identifier(kind, &format!("{schema}.{}", table.name)),
        None => quote_identifier(kind, &table.name),
    }
}

pub fn build_select(
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[String],
    filters: &[Filter],
    order: &[Order],
    page: Option<Page>,
) -> Result<SqlStatement> {
    let table = quote_table(kind, table)?;
    let projection = if columns.is_empty() {
        "*".to_owned()
    } else {
        let mut projection = String::new();
        for (index, column) in columns.iter().enumerate() {
            if index > 0 {
                projection.push_str(", ");
            }
            projection.push_str(&quote_identifier(kind, column)?);
        }
        projection
    };

    let mut statement = String::from("SELECT ");
    statement.push_str(&projection);
    statement.push_str(" FROM ");
    statement.push_str(&table);
    let mut params = Vec::new();
    append_filters(kind, &mut statement, &mut params, filters)?;
    append_order(kind, &mut statement, order)?;
    append_page(kind, &mut statement, &mut params, page)?;
    Ok(SqlStatement::new(statement, params))
}

pub fn build_insert(kind: DatabaseKind, request: &InsertRequest) -> Result<SqlStatement> {
    if request.columns.len() != request.values.len() {
        return Err(DbxError::Parse(
            "insert columns and values must have the same length".into(),
        ));
    }
    let table = quote_table(kind, &request.table)?;
    if request.columns.is_empty() {
        // A row editor may legitimately leave every field at its database
        // default (for example, an identity-only table). MySQL spells this
        // form with an empty column list; PostgreSQL and SQLite support the
        // standard DEFAULT VALUES form.
        let statement = match kind {
            DatabaseKind::MySQL => format!("INSERT INTO {table} () VALUES ()"),
            DatabaseKind::PostgreSQL | DatabaseKind::SQLite => {
                format!("INSERT INTO {table} DEFAULT VALUES")
            }
            DatabaseKind::Redis => {
                return Err(DbxError::Unsupported {
                    operation: "insert".to_owned(),
                    kind,
                });
            }
        };
        return Ok(SqlStatement::new(statement, Vec::new()));
    }
    let mut statement = format!("INSERT INTO {table} (");
    for (index, column) in request.columns.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        statement.push_str(&quote_identifier(kind, column)?);
    }
    statement.push_str(") VALUES (");
    let mut params = Vec::with_capacity(request.values.len());
    for (index, value) in request.values.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        append_mutation_value(kind, &mut statement, &mut params, value)?;
    }
    statement.push(')');
    Ok(SqlStatement::new(statement, params))
}

/// Build one multi-row `INSERT` for bulk loading, for example during CSV/TSV
/// imports. Every row must supply exactly one value per column; values stay
/// parameterized and identifiers quoted like the single-row builder.
pub fn build_multi_row_insert(
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[String],
    rows: &[Vec<CellValue>],
) -> Result<SqlStatement> {
    if rows.is_empty() {
        return Err(DbxError::Parse(
            "bulk insert requires at least one row".into(),
        ));
    }
    let width = columns.len();
    if width == 0 {
        return Err(DbxError::Parse(
            "bulk insert requires at least one column".into(),
        ));
    }
    if rows.iter().any(|row| row.len() != width) {
        return Err(DbxError::Parse(
            "bulk insert rows must all match the column count".into(),
        ));
    }
    let mut statement = format!("INSERT INTO {} (", quote_table(kind, table)?);
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        statement.push_str(&quote_identifier(kind, column)?);
    }
    statement.push_str(") VALUES ");
    let mut params = Vec::with_capacity(rows.len() * width);
    for (row_index, row) in rows.iter().enumerate() {
        if row_index > 0 {
            statement.push_str(", ");
        }
        statement.push('(');
        for (column_index, value) in row.iter().enumerate() {
            if column_index > 0 {
                statement.push_str(", ");
            }
            statement.push_str(&placeholder(kind, params.len() + 1));
            params.push(value.clone());
        }
        statement.push(')');
    }
    Ok(SqlStatement::new(statement, params))
}

pub fn build_update(kind: DatabaseKind, request: &UpdateRequest) -> Result<SqlStatement> {
    build_update_with_columns(kind, request, &[])
}

/// Build an update while retaining the database type information needed by
/// drivers whose enum parameters cannot be inferred from a text bind.
pub fn build_update_with_columns(
    kind: DatabaseKind,
    request: &UpdateRequest,
    columns: &[ColumnInfo],
) -> Result<SqlStatement> {
    if request.assignments.is_empty() {
        return Err(DbxError::Parse(
            "update requires at least one assignment".into(),
        ));
    }
    if request.filters.is_empty() {
        return Err(DbxError::Parse(
            "update requires primary-key equality predicates; use raw SQL for an intentional full-table update"
                .into(),
        ));
    }
    if request
        .filters
        .iter()
        .any(|filter| filter.operator != FilterOperator::Equals)
    {
        return Err(DbxError::Parse(
            "update requires primary-key equality predicates".into(),
        ));
    }
    let mut statement = format!("UPDATE {} SET ", quote_table(kind, &request.table)?);
    let mut params = Vec::with_capacity(request.assignments.len() + request.filters.len());
    for (index, (column, value)) in request.assignments.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        let value_sql = if kind == DatabaseKind::PostgreSQL
            && matches!(value, MutationValue::Parameter(_))
            && columns
                .iter()
                .find(|metadata| metadata.name == *column)
                .is_some_and(|metadata| !metadata.enum_values.is_empty())
        {
            let enum_type = columns
                .iter()
                .find(|metadata| metadata.name == *column)
                .map(|metadata| metadata.data_type.as_str())
                .ok_or_else(|| DbxError::Parse("enum column metadata disappeared".into()))?;
            let placeholder = placeholder(kind, params.len() + 1);
            format!(
                "CAST({placeholder} AS {})",
                quote_identifier(kind, enum_type)?
            )
        } else {
            let mut value_sql = String::new();
            append_mutation_value(kind, &mut value_sql, &mut params, value)?;
            value_sql
        };
        write!(
            statement,
            "{} = {}",
            quote_identifier(kind, column)?,
            value_sql
        )
        .map_err(|error| DbxError::Parse(error.to_string()))?;
        if let MutationValue::Parameter(value) = value
            && kind == DatabaseKind::PostgreSQL
            && columns
                .iter()
                .find(|metadata| metadata.name == *column)
                .is_some_and(|metadata| !metadata.enum_values.is_empty())
        {
            params.push(value.clone());
        }
    }
    append_filters(kind, &mut statement, &mut params, &request.filters)?;
    Ok(SqlStatement::new(statement, params))
}

fn append_mutation_value(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    value: &MutationValue,
) -> Result<()> {
    match value {
        MutationValue::Parameter(value) => {
            statement.push_str(&placeholder(kind, params.len() + 1));
            params.push(value.clone());
        }
        MutationValue::Expression(expression) => {
            statement.push_str(validate_sql_expression(expression)?)
        }
    }
    Ok(())
}

/// Validate a mutation SQL expression before it is interpolated into an
/// otherwise parameterized `INSERT` or `UPDATE`. The returned slice is
/// trimmed and safe for the deliberately narrow expression position.
pub fn validate_sql_expression(expression: &str) -> Result<&str> {
    if expression.contains(['\n', '\r']) {
        return Err(DbxError::Parse(
            "mutation expression must be a single non-empty SQL expression without comments or statement separators"
                .into(),
        ));
    }
    let expression = expression.trim();
    if expression.is_empty()
        || expression.contains('\0')
        || expression.contains(';')
        || expression.contains("--")
        || expression.contains('#')
        || expression.contains("/*")
        || expression.contains("*/")
    {
        return Err(DbxError::Parse(
            "mutation expression must be a single non-empty SQL expression without comments or statement separators"
                .into(),
        ));
    }
    Ok(expression)
}

pub fn build_delete(
    kind: DatabaseKind,
    table: &TableRef,
    filters: &[Filter],
) -> Result<SqlStatement> {
    if filters.is_empty() {
        return Err(DbxError::Parse(
            "delete requires at least one filter; use raw SQL for an intentional full-table delete"
                .into(),
        ));
    }
    let mut statement = format!("DELETE FROM {}", quote_table(kind, table)?);
    let mut params = Vec::new();
    append_filters(kind, &mut statement, &mut params, filters)?;
    Ok(SqlStatement::new(statement, params))
}

/// Build the dialect-specific statement used to remove every row from a
/// table while retaining the table definition.
pub fn build_truncate_table(kind: DatabaseKind, table: &TableRef) -> Result<SqlStatement> {
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "truncate_table".to_owned(),
            kind,
        });
    }
    let statement = match kind {
        DatabaseKind::PostgreSQL | DatabaseKind::MySQL => {
            format!("TRUNCATE TABLE {}", quote_table(kind, table)?)
        }
        // SQLite has no TRUNCATE statement. DELETE keeps the schema and
        // indexes intact while matching the operation's row-removal
        // semantics.
        DatabaseKind::SQLite => format!("DELETE FROM {}", quote_table(kind, table)?),
        DatabaseKind::Redis => unreachable!("non-SQL kinds are rejected above"),
    };
    Ok(SqlStatement::new(statement, Vec::new()))
}

/// Build a statement that drops a table using a safely quoted table
/// identifier.
pub fn build_drop_table(kind: DatabaseKind, table: &TableRef) -> Result<SqlStatement> {
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "drop_table".to_owned(),
            kind,
        });
    }
    Ok(SqlStatement::new(
        format!("DROP TABLE {}", quote_table(kind, table)?),
        Vec::new(),
    ))
}

pub fn build_create_table(
    kind: DatabaseKind,
    request: &CreateTableRequest,
) -> Result<SqlStatement> {
    if request.columns.is_empty() {
        return Err(DbxError::Parse("table requires at least one column".into()));
    }
    let mut statement = String::from("CREATE TABLE ");
    if request.if_not_exists {
        statement.push_str("IF NOT EXISTS ");
    }
    statement.push_str(&quote_table(kind, &request.table)?);
    statement.push_str(" (");
    for (index, column) in request.columns.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        write!(
            statement,
            "{} {}",
            quote_identifier(kind, &column.name)?,
            safe_type(&column.data_type)?
        )
        .map_err(|error| DbxError::Parse(error.to_string()))?;
        if !column.nullable {
            statement.push_str(" NOT NULL");
        }
        if column.primary_key {
            statement.push_str(" PRIMARY KEY");
        }
        if let Some(default_expression) = &column.default_expression {
            statement.push_str(" DEFAULT ");
            statement.push_str(&safe_default(default_expression)?);
        }
    }
    statement.push(')');
    Ok(SqlStatement::new(statement, Vec::new()))
}

fn safe_type(data_type: &str) -> Result<String> {
    let data_type = data_type.trim();
    if data_type.is_empty()
        || data_type.contains(';')
        || data_type.contains('\\')
        || data_type.contains('\0')
    {
        return Err(DbxError::Parse("invalid column type".into()));
    }
    if !data_type.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '_' | '(' | ')' | ',' | ' ')
    }) {
        return Err(DbxError::Parse(format!(
            "invalid column type `{data_type}`"
        )));
    }
    Ok(data_type.to_owned())
}

fn safe_default(expression: &str) -> Result<String> {
    let expression = expression.trim();
    if expression.is_empty()
        || expression.contains(';')
        || expression.contains('\0')
        || expression.contains("--")
        || expression.contains("/*")
        || expression.contains("*/")
    {
        return Err(DbxError::Parse("invalid default expression".into()));
    }
    // Defaults are deliberately a narrow expression field. Literals and
    // common SQL functions are accepted; semicolon-separated statements are
    // never accepted.
    if expression
        .chars()
        .any(|character| character == '\n' || character == '\r')
    {
        return Err(DbxError::Parse("invalid default expression".into()));
    }
    Ok(expression.to_owned())
}

fn append_filters(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    filters: &[Filter],
) -> Result<()> {
    if filters.is_empty() {
        return Ok(());
    }
    statement.push_str(" WHERE ");
    for (index, filter) in filters.iter().enumerate() {
        if index > 0 {
            statement.push_str(" AND ");
        }
        statement.push_str(&quote_identifier(kind, &filter.column)?);
        match filter.operator {
            FilterOperator::Equals => push_value_predicate(kind, statement, params, filter, " = ")?,
            FilterOperator::NotEquals => {
                push_value_predicate(kind, statement, params, filter, " <> ")?
            }
            FilterOperator::GreaterThan => {
                push_value_predicate(kind, statement, params, filter, " > ")?
            }
            FilterOperator::GreaterThanOrEqual => {
                push_value_predicate(kind, statement, params, filter, " >= ")?
            }
            FilterOperator::LessThan => {
                push_value_predicate(kind, statement, params, filter, " < ")?
            }
            FilterOperator::LessThanOrEqual => {
                push_value_predicate(kind, statement, params, filter, " <= ")?
            }
            FilterOperator::Contains => {
                push_like_predicate(kind, statement, params, filter, "%", "%")?
            }
            FilterOperator::StartsWith => {
                push_like_predicate(kind, statement, params, filter, "", "%")?
            }
            FilterOperator::EndsWith => {
                push_like_predicate(kind, statement, params, filter, "%", "")?
            }
            FilterOperator::IsNull => {
                if filter.value.is_some() {
                    return Err(DbxError::Parse("IS NULL does not accept a value".into()));
                }
                statement.push_str(" IS NULL");
            }
            FilterOperator::IsNotNull => {
                if filter.value.is_some() {
                    return Err(DbxError::Parse(
                        "IS NOT NULL does not accept a value".into(),
                    ));
                }
                statement.push_str(" IS NOT NULL");
            }
        }
    }
    Ok(())
}

fn push_value_predicate(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    filter: &Filter,
    operator: &str,
) -> Result<()> {
    let Some(value) = filter.value.as_ref() else {
        return Err(DbxError::Parse("filter operator requires a value".into()));
    };
    if matches!(value, CellValue::Null) {
        match operator {
            " = " => statement.push_str(" IS NULL"),
            " <> " => statement.push_str(" IS NOT NULL"),
            _ => {
                return Err(DbxError::Parse(
                    "NULL can only be compared with equality or inequality".into(),
                ));
            }
        }
        return Ok(());
    }
    statement.push_str(operator);
    statement.push_str(&placeholder(kind, params.len() + 1));
    params.push(value.clone());
    Ok(())
}

fn push_like_predicate(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    filter: &Filter,
    prefix: &str,
    suffix: &str,
) -> Result<()> {
    let Some(value) = filter.value.as_ref() else {
        return Err(DbxError::Parse("LIKE filter requires a value".into()));
    };
    let CellValue::Text(value) = value else {
        return Err(DbxError::Parse("LIKE filter requires text value".into()));
    };
    statement.push_str(" LIKE ");
    statement.push_str(&placeholder(kind, params.len() + 1));
    statement.push_str(" ESCAPE '!'");
    let escaped = value
        .replace('!', "!!")
        .replace('%', "!%")
        .replace('_', "!_");
    params.push(CellValue::Text(format!("{prefix}{escaped}{suffix}")));
    Ok(())
}

fn append_order(kind: DatabaseKind, statement: &mut String, order: &[Order]) -> Result<()> {
    if order.is_empty() {
        return Ok(());
    }
    statement.push_str(" ORDER BY ");
    for (index, item) in order.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        statement.push_str(&quote_identifier(kind, &item.column)?);
        statement.push_str(match item.direction {
            OrderDirection::Ascending => " ASC",
            OrderDirection::Descending => " DESC",
        });
    }
    Ok(())
}

fn append_page(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    page: Option<Page>,
) -> Result<()> {
    let Some(page) = page else {
        return Ok(());
    };
    if page.limit == 0 {
        return Err(DbxError::Parse(
            "page limit must be greater than zero".into(),
        ));
    }
    if u64::from(page.limit) > i64::MAX as u64 || page.offset > i64::MAX as u64 {
        return Err(DbxError::Parse(
            "page values exceed the SQL integer range".into(),
        ));
    }
    statement.push_str(" LIMIT ");
    statement.push_str(&placeholder(kind, params.len() + 1));
    params.push(CellValue::Unsigned(u64::from(page.limit)));
    statement.push_str(" OFFSET ");
    statement.push_str(&placeholder(kind, params.len() + 1));
    params.push(CellValue::Unsigned(page.offset));
    Ok(())
}

fn placeholder(kind: DatabaseKind, position: usize) -> String {
    if kind == DatabaseKind::PostgreSQL {
        format!("${position}")
    } else {
        "?".to_owned()
    }
}
