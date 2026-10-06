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
    if kind == DatabaseKind::ClickHouse {
        if identifier.contains('\0') || identifier.split('.').any(str::is_empty) {
            return Err(DbxError::Parse("Invalid ClickHouse identifier".into()));
        }
        return Ok(identifier
            .split('.')
            .map(|part| format!("`{}`", part.replace('\\', "\\\\").replace('`', "\\`")))
            .collect::<Vec<_>>()
            .join("."));
    }
    if kind == DatabaseKind::BigQuery {
        if identifier.contains('\0') || identifier.split('.').any(str::is_empty) {
            return Err(DbxError::Parse("Invalid BigQuery identifier".into()));
        }
        return Ok(format!(
            "`{}`",
            identifier.replace('\\', "\\\\").replace('`', "\\`")
        ));
    }
    if kind == DatabaseKind::SqlServer {
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
            output.push('[');
            output.push_str(&part.replace(']', "]]"));
            output.push(']');
        }
        return Ok(output);
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
    build_select_with_columns(kind, table, columns, filters, order, page, &[])
}

/// Build a select using table metadata so filter parameters are typed for
/// the compared column (PostgreSQL does not coerce `text` to `uuid`,
/// timestamps, enums, and similar types).
pub fn build_select_with_columns(
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[String],
    filters: &[Filter],
    order: &[Order],
    page: Option<Page>,
    metadata: &[ColumnInfo],
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
    append_filters(kind, &mut statement, &mut params, filters, metadata)?;
    append_order(kind, &mut statement, order)?;
    append_page(kind, &mut statement, &mut params, page)?;
    Ok(SqlStatement::new(statement, params))
}

pub fn build_insert(kind: DatabaseKind, request: &InsertRequest) -> Result<SqlStatement> {
    build_insert_with_columns(kind, request, &[])
}

/// Build an insert using table metadata to type parameters the driver
/// cannot bind directly; see [`build_update_with_columns`].
pub fn build_insert_with_columns(
    kind: DatabaseKind,
    request: &InsertRequest,
    columns: &[ColumnInfo],
) -> Result<SqlStatement> {
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
        let statement = match kind.dialect() {
            DatabaseKind::MySQL => format!("INSERT INTO {table} () VALUES ()"),
            DatabaseKind::PostgreSQL
            | DatabaseKind::SQLite
            | DatabaseKind::DuckDB
            | DatabaseKind::BigQuery
            | DatabaseKind::SqlServer => {
                format!("INSERT INTO {table} DEFAULT VALUES")
            }
            _ => {
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
    for (index, (column, value)) in request.columns.iter().zip(&request.values).enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        append_column_value(
            kind,
            &mut statement,
            &mut params,
            value,
            column_metadata(columns, column),
        )?;
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
    build_multi_row_insert_with_columns(kind, table, columns, rows, &[])
}
pub(crate) fn build_multi_row_insert_with_columns(
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[String],
    rows: &[Vec<CellValue>],
    metadata: &[ColumnInfo],
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
            append_column_value(
                kind,
                &mut statement,
                &mut params,
                &value.clone().into(),
                column_metadata(metadata, &columns[column_index]),
            )?;
        }
        statement.push(')');
    }
    Ok(SqlStatement::new(statement, params))
}

pub fn build_update(kind: DatabaseKind, request: &UpdateRequest) -> Result<SqlStatement> {
    build_update_with_columns(kind, request, &[])
}

/// Build an update using table metadata to type parameters the driver
/// cannot bind directly: PostgreSQL will not assign a `text` parameter to a
/// date, uuid, enum, or other non-string column, and MySQL reads a string
/// written to BIT as its character bytes.
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
        write!(statement, "{} = ", quote_identifier(kind, column)?)
            .map_err(|error| DbxError::Parse(error.to_string()))?;
        append_column_value(
            kind,
            &mut statement,
            &mut params,
            value,
            column_metadata(columns, column),
        )?;
    }
    append_filters(kind, &mut statement, &mut params, &request.filters, columns)?;
    Ok(SqlStatement::new(statement, params))
}

fn column_metadata<'a>(columns: &'a [ColumnInfo], column: &str) -> Option<&'a ColumnInfo> {
    columns.iter().find(|metadata| metadata.name == column)
}

/// Extend an identity-guarded mutation with the values the user actually saw.
/// NULL uses IS NULL; all other values remain bound, including rich SQL types.
pub(crate) fn guard_original_values(
    kind: DatabaseKind,
    statement: &mut SqlStatement,
    originals: &[(String, CellValue)],
    columns: &[ColumnInfo],
) -> Result<()> {
    for (name, value) in originals {
        let column = columns
            .iter()
            .find(|column| column.name == *name)
            .ok_or_else(|| DbxError::Parse(format!("unknown original column `{name}`")))?;
        statement.sql.push_str(" AND ");
        let identifier = quote_identifier(kind, name)?;
        if matches!(value, CellValue::Null) {
            statement.sql.push_str(&format!("{identifier} IS NULL"));
        } else if kind.dialect() == DatabaseKind::PostgreSQL
            && column.data_type.eq_ignore_ascii_case("json")
        {
            // PostgreSQL json has no equality operator, but jsonb does.
            statement
                .sql
                .push_str(&format!("CAST({identifier} AS jsonb) = CAST("));
            append_mutation_value(
                kind,
                &mut statement.sql,
                &mut statement.params,
                &value.clone().into(),
            )?;
            statement.sql.push_str(" AS jsonb)");
        } else {
            statement.sql.push_str(&format!("{identifier} = "));
            append_column_value(
                kind,
                &mut statement.sql,
                &mut statement.params,
                &value.clone().into(),
                Some(column),
            )?;
        }
    }
    Ok(())
}

/// Append one assignment/insert value, casting text and NULL parameters to
/// the column type where the dialect needs it.
fn append_column_value(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    value: &MutationValue,
    column: Option<&ColumnInfo>,
) -> Result<()> {
    let cast = match (kind, value, column) {
        (
            DatabaseKind::PostgreSQL | DatabaseKind::CockroachDB,
            MutationValue::Parameter(CellValue::Text(_) | CellValue::Null),
            Some(column),
        ) => cast_type_name(&column.data_type),
        (DatabaseKind::MySQL, MutationValue::Parameter(CellValue::Text(_)), Some(column))
            if column
                .data_type
                .trim()
                .get(..3)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("bit")) =>
        {
            Some("UNSIGNED")
        }
        _ => None,
    };
    let Some(cast) = cast else {
        return append_mutation_value(kind, statement, params, value);
    };
    statement.push_str("CAST(");
    append_mutation_value(kind, statement, params, value)?;
    write!(statement, " AS {cast})").map_err(|error| DbxError::Parse(error.to_string()))
}

/// Accept a catalog type name for use in `CAST(... AS type)`. PostgreSQL's
/// `format_type` output is already valid SQL (quoting odd identifiers), so
/// this only rejects anything that could end the expression. Unusual names
/// fall back to an uncast parameter rather than failing the mutation.
fn cast_type_name(data_type: &str) -> Option<&str> {
    let data_type = data_type.trim();
    let safe = !data_type.is_empty()
        && data_type.matches('"').count().is_multiple_of(2)
        && !data_type.contains("--")
        && !data_type.contains("/*")
        && data_type.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '_' | '(' | ')' | ',' | ' ' | '.' | '[' | ']' | '"'
                )
        });
    safe.then_some(data_type)
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
    build_delete_with_columns(kind, table, filters, &[])
}

/// Build a delete using table metadata to type filter parameters; see
/// [`build_select_with_columns`].
pub fn build_delete_with_columns(
    kind: DatabaseKind,
    table: &TableRef,
    filters: &[Filter],
    columns: &[ColumnInfo],
) -> Result<SqlStatement> {
    if filters.is_empty() {
        return Err(DbxError::Parse(
            "delete requires at least one filter; use raw SQL for an intentional full-table delete"
                .into(),
        ));
    }
    let mut statement = format!("DELETE FROM {}", quote_table(kind, table)?);
    let mut params = Vec::new();
    append_filters(kind, &mut statement, &mut params, filters, columns)?;
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
    let statement = match kind.dialect() {
        DatabaseKind::PostgreSQL
        | DatabaseKind::MySQL
        | DatabaseKind::DuckDB
        | DatabaseKind::ClickHouse
        | DatabaseKind::BigQuery
        | DatabaseKind::SqlServer => {
            format!("TRUNCATE TABLE {}", quote_table(kind, table)?)
        }
        // SQLite has no TRUNCATE statement. DELETE keeps the schema and
        // indexes intact while matching the operation's row-removal
        // semantics.
        DatabaseKind::SQLite => format!("DELETE FROM {}", quote_table(kind, table)?),
        _ => unreachable!("non-SQL kinds are rejected above"),
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
    let mut statement = String::new();
    if request.if_not_exists && kind == DatabaseKind::SqlServer {
        // SQL Server has no CREATE TABLE IF NOT EXISTS.
        statement.push_str(&format!(
            "IF OBJECT_ID(N'{}', N'U') IS NULL ",
            quote_table(kind, &request.table)?.replace('\'', "''")
        ));
    }
    statement.push_str("CREATE TABLE ");
    if request.if_not_exists && kind != DatabaseKind::SqlServer {
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
    columns: &[ColumnInfo],
) -> Result<()> {
    if filters.is_empty() {
        return Ok(());
    }
    statement.push_str(" WHERE ");
    for (index, filter) in filters.iter().enumerate() {
        if index > 0 {
            statement.push_str(" AND ");
        }
        let column = column_metadata(columns, &filter.column);
        let identifier = quote_identifier(kind, &filter.column)?;
        let comparison = match filter.operator {
            FilterOperator::Equals => Some(" = "),
            FilterOperator::NotEquals => Some(" <> "),
            FilterOperator::GreaterThan => Some(" > "),
            FilterOperator::GreaterThanOrEqual => Some(" >= "),
            FilterOperator::LessThan => Some(" < "),
            FilterOperator::LessThanOrEqual => Some(" <= "),
            _ => None,
        };
        if let Some(operator) = comparison {
            statement.push_str(&identifier);
            push_value_predicate(kind, statement, params, filter, operator, column)?;
            continue;
        }
        match filter.operator {
            FilterOperator::Contains => push_like_predicate(
                kind,
                statement,
                params,
                filter,
                &identifier,
                column,
                "%",
                "%",
            )?,
            FilterOperator::StartsWith => push_like_predicate(
                kind,
                statement,
                params,
                filter,
                &identifier,
                column,
                "",
                "%",
            )?,
            FilterOperator::EndsWith => push_like_predicate(
                kind,
                statement,
                params,
                filter,
                &identifier,
                column,
                "%",
                "",
            )?,
            FilterOperator::IsNull => {
                if filter.value.is_some() {
                    return Err(DbxError::Parse("IS NULL does not accept a value".into()));
                }
                statement.push_str(&identifier);
                statement.push_str(" IS NULL");
            }
            FilterOperator::IsNotNull => {
                if filter.value.is_some() {
                    return Err(DbxError::Parse(
                        "IS NOT NULL does not accept a value".into(),
                    ));
                }
                statement.push_str(&identifier);
                statement.push_str(" IS NOT NULL");
            }
            _ => unreachable!("comparison operators are handled above"),
        }
    }
    Ok(())
}

/// Whether a PostgreSQL catalog type is a character type that compares
/// directly with a `text` parameter. Casting to `varchar(n)`/`char(n)`
/// would silently truncate the user's value, so these stay uncast.
fn is_postgres_text_type(data_type: &str) -> bool {
    let data_type = data_type.trim().to_ascii_lowercase();
    let base = data_type.split('(').next().unwrap_or_default().trim();
    matches!(
        base,
        "text"
            | "character varying"
            | "varchar"
            | "character"
            | "char"
            | "bpchar"
            | "name"
            | "citext"
    )
}

/// The type a text filter parameter must be cast to before it is compared
/// with `column`, or `None` when the dialect coerces it implicitly.
fn filter_parameter_cast<'a>(
    kind: DatabaseKind,
    value: &CellValue,
    column: Option<&'a ColumnInfo>,
) -> Option<&'a str> {
    match (kind, value, column) {
        (
            DatabaseKind::PostgreSQL | DatabaseKind::CockroachDB,
            CellValue::Text(_),
            Some(column),
        ) if !is_postgres_text_type(&column.data_type) => cast_type_name(&column.data_type),
        _ => None,
    }
}

fn push_value_predicate(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    filter: &Filter,
    operator: &str,
    column: Option<&ColumnInfo>,
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
    let placeholder = placeholder(kind, params.len() + 1);
    match filter_parameter_cast(kind, value, column) {
        Some(cast) => write!(statement, "CAST({placeholder} AS {cast})")
            .map_err(|error| DbxError::Parse(error.to_string()))?,
        None => statement.push_str(&placeholder),
    }
    params.push(value.clone());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn push_like_predicate(
    kind: DatabaseKind,
    statement: &mut String,
    params: &mut Vec<CellValue>,
    filter: &Filter,
    identifier: &str,
    column: Option<&ColumnInfo>,
    prefix: &str,
    suffix: &str,
) -> Result<()> {
    let Some(value) = filter.value.as_ref() else {
        return Err(DbxError::Parse("LIKE filter requires a value".into()));
    };
    let CellValue::Text(value) = value else {
        return Err(DbxError::Parse("LIKE filter requires text value".into()));
    };
    // PostgreSQL has no LIKE operator for uuid, numeric, timestamp, enum, and
    // other non-character types; match against their text rendering instead.
    // Without metadata the cast is still safe because text-to-text is a no-op.
    let cast_to_text = kind.dialect() == DatabaseKind::PostgreSQL
        && column.is_none_or(|column| !is_postgres_text_type(&column.data_type));
    if cast_to_text {
        write!(statement, "CAST({identifier} AS text)")
            .map_err(|error| DbxError::Parse(error.to_string()))?;
    } else {
        statement.push_str(identifier);
    }
    statement.push_str(" LIKE ");
    statement.push_str(&placeholder(kind, params.len() + 1));
    let escaped = if matches!(kind, DatabaseKind::BigQuery | DatabaseKind::ClickHouse) {
        value
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    } else {
        statement.push_str(" ESCAPE '!'");
        // SQL Server also treats `[` as the start of a character class.
        let value = value
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_");
        if kind == DatabaseKind::SqlServer {
            value.replace('[', "![")
        } else {
            value
        }
    };
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
    if kind == DatabaseKind::SqlServer {
        // OFFSET/FETCH requires an ORDER BY; keep the server's natural order
        // when the caller did not choose one.
        if !statement.contains(" ORDER BY ") {
            statement.push_str(" ORDER BY (SELECT NULL)");
        }
        statement.push_str(" OFFSET ");
        statement.push_str(&placeholder(kind, params.len() + 1));
        params.push(CellValue::Unsigned(page.offset));
        statement.push_str(" ROWS FETCH NEXT ");
        statement.push_str(&placeholder(kind, params.len() + 1));
        params.push(CellValue::Unsigned(u64::from(page.limit)));
        statement.push_str(" ROWS ONLY");
        return Ok(());
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
    if kind.dialect() == DatabaseKind::PostgreSQL {
        format!("${position}")
    } else if kind == DatabaseKind::SqlServer {
        format!("@P{position}")
    } else {
        "?".to_owned()
    }
}
