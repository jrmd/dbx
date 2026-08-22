//! Table data transfer between a connection and local files.
//!
//! Supported formats are SQL dumps (`.sql`), CSV (`.csv`), and TSV
//! (`.tsv`), each optionally gzip-compressed with a `.gz` suffix. SQL dumps
//! contain dialect-aware `INSERT` statements wrapped in a transaction; CSV
//! and TSV carry one header row of column names followed by data rows.
//!
//! Delimited conventions shared by both directions:
//!
//! - An unquoted empty field is `NULL`; a quoted empty field (`""`) is an
//!   empty string.
//! - Fields containing the delimiter, a quote, or a line break are quoted;
//!   embedded quotes are doubled.
//! - Binary values are written as lowercase hex text, because neither CSV
//!   nor TSV has a binary convention.

use std::{
    collections::VecDeque,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use crate::{
    CellValue, ColumnInfo, DatabaseEngine, DatabaseKind, DbxError, Page, QueryOptions, Result,
    TableRef, TableStructure,
    sql::{build_multi_row_insert, quote_identifier, quote_table},
};

/// Rows fetched per page while exporting. Bounded so a large table streams
/// page-by-page instead of being loaded into memory at once.
pub const EXPORT_PAGE_SIZE: usize = 1_000;

/// Rows per multi-row `INSERT` before the batch is flushed on import. The
/// effective batch is narrowed further by [`max_params_per_statement`] so
/// wide tables stay inside each driver's placeholder limit.
const IMPORT_ROWS_PER_BATCH: usize = 500;

/// The data format inside a transfer file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DumpFormat {
    Sql,
    Csv,
    Tsv,
}

impl DumpFormat {
    /// Field delimiter for delimited formats; `None` for SQL dumps.
    pub fn delimiter(self) -> Option<u8> {
        match self {
            Self::Sql => None,
            Self::Csv => Some(b','),
            Self::Tsv => Some(b'\t'),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Sql => "sql",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
        }
    }
}

impl std::fmt::Display for DumpFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Sql => "SQL dump",
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
        })
    }
}

/// A detected file format plus its gzip state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileFormat {
    pub format: DumpFormat,
    pub gzipped: bool,
}

/// Recognize a transfer file by its extension. `.gz` may wrap any supported
/// format, for example `events.sql.gz` or `rows.csv.gz`.
pub fn detect_file_format(path: &Path) -> Result<FileFormat> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| DbxError::Io(format!("`{}` is not a usable file name", path.display())))?;
    let lower = name.to_ascii_lowercase();
    let (stem, gzipped) = match lower.strip_suffix(".gz") {
        Some(stem) => (stem, true),
        None => (lower.as_str(), false),
    };
    let format = match stem.rsplit('.').next() {
        Some("sql") => DumpFormat::Sql,
        Some("csv") => DumpFormat::Csv,
        Some("tsv") => DumpFormat::Tsv,
        _ => {
            return Err(DbxError::Io(format!(
                "unsupported file type `{name}`; expected .sql, .csv, or .tsv (optionally .gz)"
            )));
        }
    };
    Ok(FileFormat { format, gzipped })
}

/// Summary of a completed export.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExportSummary {
    pub rows_exported: u64,
    pub format: DumpFormat,
    pub gzipped: bool,
}

/// A connection-level export request.
///
/// SQL exports are written as one file. CSV and TSV exports write one file per
/// selected table beneath `output_directory`, using `output_name` as the
/// filename prefix. This keeps each delimited file independently consumable by
/// spreadsheet and database tooling while still making one database export a
/// single user action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseExportRequest {
    pub tables: Vec<TableRef>,
    pub output_directory: PathBuf,
    pub output_name: String,
    pub format: DumpFormat,
    pub schema_only: bool,
    pub gzipped: bool,
}

/// Summary of a completed connection-level export.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatabaseExportSummary {
    pub tables_exported: u64,
    pub files_written: u64,
    pub rows_exported: u64,
    pub format: DumpFormat,
    pub gzipped: bool,
    pub schema_only: bool,
}

struct PreparedExportTable {
    table: TableRef,
    structure: TableStructure,
}

/// Summary of a completed import.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportReport {
    /// Statements run for SQL dumps; zero for delimited imports.
    pub statements_executed: u64,
    /// Rows inserted for delimited imports; zero for SQL dumps.
    pub rows_inserted: u64,
    pub elapsed_ms: u64,
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// Export one table to `path`. The file format and compression follow the
/// path's extension, so `orders.sql`, `orders.csv.gz`, and `orders.tsv` all
/// do what their names say.
pub async fn export_table(
    engine: &DatabaseEngine,
    table: &TableRef,
    path: &Path,
) -> Result<ExportSummary> {
    let kind = engine.kind();
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "export_table".to_owned(),
            kind,
        });
    }
    let file_format = detect_file_format(path)?;
    let columns = engine.describe_table(table).await?;
    let column_names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();

    let mut output = Vec::<u8>::new();
    match file_format.format {
        DumpFormat::Sql => write_sql_dump_header(&mut output, kind, table)?,
        DumpFormat::Csv | DumpFormat::Tsv => {
            let delimiter = file_format.format.delimiter().unwrap_or(b',');
            let header: Vec<Option<&str>> = column_names
                .iter()
                .map(|name| Some(name.as_str()))
                .collect();
            write_delimited_record(&mut output, delimiter, &header)?;
        }
    }

    let mut rows_exported = 0u64;
    let mut offset = 0u64;
    loop {
        let result = engine
            .query_table(
                table,
                &[],
                &[],
                &[],
                Some(Page {
                    limit: EXPORT_PAGE_SIZE as u32,
                    offset,
                }),
                QueryOptions { max_rows: None },
            )
            .await?;
        for row in &result.rows {
            match file_format.format {
                DumpFormat::Sql => {
                    let statement = render_sql_insert(kind, table, &column_names, &row.values)?;
                    output.extend_from_slice(statement.as_bytes());
                    output.extend_from_slice(b";\n");
                }
                DumpFormat::Csv | DumpFormat::Tsv => {
                    let delimiter = file_format.format.delimiter().unwrap_or(b',');
                    let fields: Vec<Option<String>> =
                        row.values.iter().map(delimited_value_field).collect();
                    let borrowed: Vec<Option<&str>> =
                        fields.iter().map(|field| field.as_deref()).collect();
                    write_delimited_record(&mut output, delimiter, &borrowed)?;
                }
            }
        }
        let page_rows = result.rows.len();
        rows_exported += page_rows as u64;
        offset += page_rows as u64;
        if page_rows < EXPORT_PAGE_SIZE {
            break;
        }
    }

    let gzipped = file_format.gzipped;
    let bytes = if gzipped {
        gzip_encode(output)?
    } else {
        output
    };
    let target: PathBuf = path.to_owned();
    tokio::task::spawn_blocking(move || fs::write(target, bytes))
        .await
        .map_err(|error| DbxError::Io(error.to_string()))?
        .map_err(|error| DbxError::Io(error.to_string()))?;

    Ok(ExportSummary {
        rows_exported,
        format: file_format.format,
        gzipped,
    })
}

/// Export selected tables from the active database.
///
/// SQL exports contain all generated table schemas first, followed by all data
/// inserts unless `schema_only` is set. PostgreSQL and MySQL foreign keys are
/// added after the data phase so the dump never depends on table or row order;
/// SQLite keeps them inline and orders tables by their dependencies. CSV and
/// TSV exports are written one file per table; those formats do not have a
/// portable representation for a database schema, so schema-only mode is
/// intentionally limited to SQL.
pub async fn export_database(
    engine: &DatabaseEngine,
    request: &DatabaseExportRequest,
) -> Result<DatabaseExportSummary> {
    let kind = engine.kind();
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "export_database".to_owned(),
            kind,
        });
    }
    if request.tables.is_empty() {
        return Err(DbxError::Parse(
            "database export requires at least one table".into(),
        ));
    }
    if request.schema_only && request.format != DumpFormat::Sql {
        return Err(DbxError::Parse(
            "schema-only exports require the SQL format".into(),
        ));
    }

    validate_output_directory(&request.output_directory)?;
    let stem = normalize_output_stem(&request.output_name)?;

    if request.format == DumpFormat::Sql {
        // Snapshot every structure before writing any output. This lets the
        // dump emit a complete schema phase before it starts querying rows.
        let mut export_tables = Vec::with_capacity(request.tables.len());
        for table in &request.tables {
            export_tables.push(PreparedExportTable {
                table: table.clone(),
                structure: engine.table_structure(table).await?,
            });
        }
        let table_order = table_export_order(&export_tables);
        let mut output = Vec::new();
        output.extend_from_slice(b"-- DBX database dump\n");
        output.extend_from_slice(
            format!(
                "-- Tables: {}\n{}\n",
                request.tables.len(),
                if request.schema_only {
                    "-- Schema only"
                } else {
                    "-- Schema and data"
                }
            )
            .as_bytes(),
        );
        output.extend_from_slice(b"\n-- Schema\n");
        for &index in &table_order {
            let export_table = &export_tables[index];
            output.extend_from_slice(
                format!("-- Table: {}\n", quote_table(kind, &export_table.table)?).as_bytes(),
            );
            let schema = if kind == DatabaseKind::SQLite {
                render_sql_schema(
                    kind,
                    &export_table.table,
                    &export_table.structure,
                    &request.tables,
                )?
            } else {
                render_sql_schema_without_foreign_keys(
                    kind,
                    &export_table.table,
                    &export_table.structure,
                    &request.tables,
                )?
            };
            output.extend_from_slice(schema.as_bytes());
            output.extend_from_slice(b";\n");
            output.push(b'\n');
        }

        let mut rows_exported = 0u64;
        if !request.schema_only {
            output.extend_from_slice(b"-- Data\n");
            for &index in &table_order {
                let export_table = &export_tables[index];
                rows_exported += append_sql_table_data(
                    engine,
                    kind,
                    &export_table.table,
                    &export_table.structure.columns,
                    &mut output,
                )
                .await?;
                output.push(b'\n');
            }
        }

        // SQLite cannot add a constraint with ALTER TABLE, so its selected
        // foreign keys remain in CREATE TABLE. PostgreSQL and MySQL can add
        // them after the data phase, which also handles cycles and arbitrary
        // selection order without disabling referential checks.
        if kind != DatabaseKind::SQLite {
            output.extend_from_slice(b"-- Foreign-key constraints\n");
            for &index in &table_order {
                append_sql_foreign_keys(
                    kind,
                    &export_tables[index].table,
                    &export_tables[index].structure,
                    &request.tables,
                    &mut output,
                )?;
            }
            output.push(b'\n');
        }

        let path =
            request
                .output_directory
                .join(with_extension(&stem, DumpFormat::Sql, request.gzipped));
        let bytes = if request.gzipped {
            gzip_encode(output)?
        } else {
            output
        };
        write_export_file(path, bytes).await?;
        return Ok(DatabaseExportSummary {
            tables_exported: request.tables.len() as u64,
            files_written: 1,
            rows_exported,
            format: request.format,
            gzipped: request.gzipped,
            schema_only: request.schema_only,
        });
    }

    let mut rows_exported = 0u64;
    for table in &request.tables {
        let table_stem = transfer_file_stem(table);
        let file_stem = format!("{stem}_{table_stem}");
        let path = request.output_directory.join(with_extension(
            &file_stem,
            request.format,
            request.gzipped,
        ));
        let summary = export_table(engine, table, &path).await?;
        rows_exported += summary.rows_exported;
    }

    Ok(DatabaseExportSummary {
        tables_exported: request.tables.len() as u64,
        files_written: request.tables.len() as u64,
        rows_exported,
        format: request.format,
        gzipped: request.gzipped,
        schema_only: false,
    })
}

/// Import a complete database dump. Delimited files remain table-scoped and
/// must go through [`import_file`] with an explicit target table.
pub async fn import_database(engine: &DatabaseEngine, path: &Path) -> Result<ImportReport> {
    let file_format = detect_file_format(path)?;
    if file_format.format != DumpFormat::Sql {
        return Err(DbxError::Parse(
            "database imports require an SQL dump; CSV and TSV imports target one table".into(),
        ));
    }
    import_file(engine, None, path).await
}

/// Render a portable `CREATE TABLE` statement from DBX's normalized metadata.
///
/// Defaults, generated expressions, and indexes are not currently part of
/// [`TableStructure`], so the output intentionally includes only columns,
/// primary keys, and foreign keys that DBX can verify from metadata.
pub fn render_sql_schema(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
    selected_tables: &[TableRef],
) -> Result<String> {
    render_sql_schema_with_foreign_keys(kind, table, structure, selected_tables, true)
}

fn render_sql_schema_without_foreign_keys(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
    selected_tables: &[TableRef],
) -> Result<String> {
    render_sql_schema_with_foreign_keys(kind, table, structure, selected_tables, false)
}

fn render_sql_schema_with_foreign_keys(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
    selected_tables: &[TableRef],
    include_foreign_keys: bool,
) -> Result<String> {
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "render_sql_schema".to_owned(),
            kind,
        });
    }
    if structure.columns.is_empty() {
        return Err(DbxError::Parse(format!(
            "table `{}` has no columns",
            table.name
        )));
    }

    let mut definitions = Vec::new();
    for column in &structure.columns {
        let mut definition = format!(
            "{} {}",
            quote_identifier(kind, &column.name)?,
            safe_schema_type(&column.data_type)?
        );
        if !column.nullable {
            definition.push_str(" NOT NULL");
        }
        definitions.push(definition);
    }

    let primary_keys: Vec<String> = structure
        .columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| quote_identifier(kind, &column.name))
        .collect::<Result<Vec<_>>>()?;
    if !primary_keys.is_empty() {
        definitions.push(format!("PRIMARY KEY ({})", primary_keys.join(", ")));
    }

    for foreign_key in &structure.foreign_keys {
        let definition =
            render_sql_foreign_key_definition(kind, table, foreign_key, selected_tables)?;
        if include_foreign_keys {
            let Some(definition) = definition else {
                continue;
            };
            definitions.push(definition);
        }
    }

    let mut statement = format!(
        "CREATE TABLE IF NOT EXISTS {} (\n",
        quote_table(kind, table)?
    );
    for (index, definition) in definitions.iter().enumerate() {
        if index > 0 {
            statement.push_str(",\n");
        }
        statement.push_str("  ");
        statement.push_str(definition);
    }
    statement.push_str("\n)");
    Ok(statement)
}

fn render_sql_foreign_key_definition(
    kind: DatabaseKind,
    table: &TableRef,
    foreign_key: &crate::ForeignKeyInfo,
    selected_tables: &[TableRef],
) -> Result<Option<String>> {
    if foreign_key.columns.is_empty()
        || foreign_key.columns.len() != foreign_key.referenced_columns.len()
    {
        return Err(DbxError::Parse(format!(
            "foreign key on `{}` has mismatched column metadata",
            table.name
        )));
    }
    let referenced_table = TableRef {
        schema: foreign_key.referenced_schema.clone(),
        name: foreign_key.referenced_table.clone(),
    };
    // A selected subset should not emit a constraint whose target is not in
    // the export. That keeps a partial schema dump executable.
    if !selected_tables.is_empty()
        && !selected_tables
            .iter()
            .any(|selected| selected == &referenced_table)
    {
        return Ok(None);
    }
    let local_columns = foreign_key
        .columns
        .iter()
        .map(|column| quote_identifier(kind, column))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    let referenced_columns = foreign_key
        .referenced_columns
        .iter()
        .map(|column| quote_identifier(kind, column))
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    let mut definition = String::new();
    if let Some(constraint_name) = &foreign_key.constraint_name {
        definition.push_str("CONSTRAINT ");
        definition.push_str(&quote_identifier(kind, constraint_name)?);
        definition.push(' ');
    }
    definition.push_str("FOREIGN KEY (");
    definition.push_str(&local_columns);
    definition.push_str(") REFERENCES ");
    definition.push_str(&quote_table(kind, &referenced_table)?);
    definition.push_str(" (");
    definition.push_str(&referenced_columns);
    definition.push(')');
    if let Some(action) = foreign_key.on_update {
        definition.push_str(" ON UPDATE ");
        definition.push_str(referential_action_sql(action));
    }
    if let Some(action) = foreign_key.on_delete {
        definition.push_str(" ON DELETE ");
        definition.push_str(referential_action_sql(action));
    }
    Ok(Some(definition))
}

fn append_sql_foreign_keys(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
    selected_tables: &[TableRef],
    output: &mut Vec<u8>,
) -> Result<()> {
    for foreign_key in &structure.foreign_keys {
        let Some(definition) =
            render_sql_foreign_key_definition(kind, table, foreign_key, selected_tables)?
        else {
            continue;
        };
        output.extend_from_slice(
            format!(
                "ALTER TABLE {} ADD {};\n",
                quote_table(kind, table)?,
                definition
            )
            .as_bytes(),
        );
    }
    Ok(())
}

fn table_export_order(tables: &[PreparedExportTable]) -> Vec<usize> {
    let mut dependencies = vec![Vec::new(); tables.len()];
    for (index, export_table) in tables.iter().enumerate() {
        for foreign_key in &export_table.structure.foreign_keys {
            let referenced_table = TableRef {
                schema: foreign_key.referenced_schema.clone(),
                name: foreign_key.referenced_table.clone(),
            };
            let Some(referenced_index) = tables
                .iter()
                .position(|candidate| candidate.table == referenced_table)
            else {
                continue;
            };
            if referenced_index != index && !dependencies[index].contains(&referenced_index) {
                dependencies[index].push(referenced_index);
            }
        }
    }

    let mut dependents = vec![Vec::new(); tables.len()];
    let mut dependency_counts: Vec<usize> = dependencies.iter().map(Vec::len).collect();
    for (index, table_dependencies) in dependencies.iter().enumerate() {
        for &dependency in table_dependencies {
            dependents[dependency].push(index);
        }
    }

    let mut ready = VecDeque::new();
    for (index, &count) in dependency_counts.iter().enumerate() {
        if count == 0 {
            ready.push_back(index);
        }
    }

    let mut order = Vec::with_capacity(tables.len());
    let mut emitted = vec![false; tables.len()];
    while let Some(index) = ready.pop_front() {
        emitted[index] = true;
        order.push(index);
        for &dependent in &dependents[index] {
            dependency_counts[dependent] -= 1;
            if dependency_counts[dependent] == 0 {
                ready.push_back(dependent);
            }
        }
    }

    // Cyclic foreign-key graphs have no topological order. Keep those tables
    // stable rather than dropping them; PostgreSQL/MySQL add their constraints
    // after data, while SQLite accepts forward references in CREATE TABLE.
    for (index, was_emitted) in emitted.into_iter().enumerate() {
        if !was_emitted {
            order.push(index);
        }
    }
    order
}

async fn append_sql_table_data(
    engine: &DatabaseEngine,
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[ColumnInfo],
    output: &mut Vec<u8>,
) -> Result<u64> {
    let column_names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();
    let mut rows_exported = 0u64;
    let mut offset = 0u64;
    loop {
        let result = engine
            .query_table(
                table,
                &[],
                &[],
                &[],
                Some(Page {
                    limit: EXPORT_PAGE_SIZE as u32,
                    offset,
                }),
                QueryOptions { max_rows: None },
            )
            .await?;
        for row in &result.rows {
            output.extend_from_slice(
                render_sql_insert(kind, table, &column_names, &row.values)?.as_bytes(),
            );
            output.extend_from_slice(b";\n");
        }
        let page_rows = result.rows.len();
        rows_exported += page_rows as u64;
        offset += page_rows as u64;
        if page_rows < EXPORT_PAGE_SIZE {
            break;
        }
    }
    Ok(rows_exported)
}

fn validate_output_directory(directory: &Path) -> Result<()> {
    if !directory.is_dir() {
        return Err(DbxError::Io(format!(
            "export destination `{}` is not a directory",
            directory.display()
        )));
    }
    Ok(())
}

fn normalize_output_stem(name: &str) -> Result<String> {
    let mut stem = name.trim().to_owned();
    if stem.is_empty() || stem.contains('/') || stem.contains('\\') || stem.contains('\0') {
        return Err(DbxError::Parse(
            "export name must be a non-empty file name without path separators".into(),
        ));
    }
    if stem.to_ascii_lowercase().ends_with(".gz") {
        stem.truncate(stem.len().saturating_sub(3));
    }
    for extension in [".sql", ".csv", ".tsv"] {
        if stem.to_ascii_lowercase().ends_with(extension) {
            stem.truncate(stem.len().saturating_sub(extension.len()));
            break;
        }
    }
    if stem.is_empty() {
        return Err(DbxError::Parse(
            "export name cannot be only an extension".into(),
        ));
    }
    Ok(stem)
}

fn with_extension(stem: &str, format: DumpFormat, gzipped: bool) -> String {
    if gzipped {
        format!("{stem}.{}.gz", format.extension())
    } else {
        format!("{stem}.{}", format.extension())
    }
}

fn transfer_file_stem(table: &TableRef) -> String {
    let raw = match &table.schema {
        Some(schema) => format!("{schema}_{}", table.name),
        None => table.name.clone(),
    };
    let sanitized: String = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "table".to_owned()
    } else {
        sanitized
    }
}

async fn write_export_file(path: PathBuf, bytes: Vec<u8>) -> Result<()> {
    tokio::task::spawn_blocking(move || fs::write(path, bytes))
        .await
        .map_err(|error| DbxError::Io(error.to_string()))?
        .map_err(|error| DbxError::Io(error.to_string()))
}

fn safe_schema_type(data_type: &str) -> Result<String> {
    let data_type = data_type.trim();
    if data_type.is_empty()
        || data_type.contains(';')
        || data_type.contains('\\')
        || data_type.contains('\0')
        || data_type.contains("--")
        || data_type.contains("/*")
        || data_type.contains("*/")
        || data_type.contains('\n')
        || data_type.contains('\r')
        || !data_type.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '_' | '(' | ')' | ',' | ' ' | '.' | '[' | ']')
        })
    {
        return Err(DbxError::Parse(format!(
            "invalid metadata column type `{data_type}`"
        )));
    }
    Ok(data_type.to_owned())
}

fn referential_action_sql(action: crate::ReferentialAction) -> &'static str {
    match action {
        crate::ReferentialAction::NoAction => "NO ACTION",
        crate::ReferentialAction::Restrict => "RESTRICT",
        crate::ReferentialAction::Cascade => "CASCADE",
        crate::ReferentialAction::SetNull => "SET NULL",
        crate::ReferentialAction::SetDefault => "SET DEFAULT",
    }
}

fn write_sql_dump_header(output: &mut Vec<u8>, kind: DatabaseKind, table: &TableRef) -> Result<()> {
    let qualified = quote_table(kind, table)?;
    output.extend_from_slice(b"-- DBX table dump\n");
    output.extend_from_slice(format!("-- Source: {qualified}\n").as_bytes());
    // Statements deliberately stay outside an explicit transaction: imports
    // replay through a connection pool where BEGIN/COMMIT would land on
    // whichever connection executes each statement.
    Ok(())
}

/// Render one row as the body of an `INSERT` statement without its trailing
/// semicolon. Values become literals; identifiers go through the shared
/// quoting rules.
pub fn render_sql_insert(
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[String],
    values: &[CellValue],
) -> Result<String> {
    if columns.len() != values.len() {
        return Err(DbxError::Parse(
            "SQL dump row does not match the table's column count".into(),
        ));
    }
    let mut statement = format!("INSERT INTO {} (", quote_table(kind, table)?);
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        statement.push_str(&quote_identifier(kind, column)?);
    }
    statement.push_str(") VALUES (");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            statement.push_str(", ");
        }
        statement.push_str(&render_sql_literal(kind, value)?);
    }
    statement.push(')');
    Ok(statement)
}

fn render_sql_literal(kind: DatabaseKind, value: &CellValue) -> Result<String> {
    match value {
        CellValue::Null => Ok("NULL".into()),
        CellValue::Boolean(value) => Ok(if *value { "TRUE" } else { "FALSE" }.to_owned()),
        CellValue::Integer(value) => Ok(value.to_string()),
        CellValue::Unsigned(value) => Ok(value.to_string()),
        CellValue::Real(value) => {
            if value.is_finite() {
                Ok(value.to_string())
            } else {
                Err(DbxError::Parse(
                    "cannot render a non-finite float in a SQL dump".into(),
                ))
            }
        }
        CellValue::Text(value) => Ok(quote_sql_text(kind, value)),
        CellValue::Bytes(bytes) => {
            let mut hex = String::with_capacity(bytes.len() * 2 + 3);
            hex.push_str("X'");
            for byte in bytes {
                use std::fmt::Write;
                let _ = write!(hex, "{byte:02x}");
            }
            hex.push('\'');
            Ok(hex)
        }
        CellValue::Json(value) => {
            let text = serde_json::to_string(value).map_err(|error| {
                DbxError::Parse(format!("JSON value could not be rendered: {error}"))
            })?;
            Ok(quote_sql_text(kind, &text))
        }
    }
}

fn quote_sql_text(kind: DatabaseKind, value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        // MySQL treats backslash as an escape character inside strings, so
        // backslashes must be doubled there in addition to quotes.
        if character == '\\' && kind == DatabaseKind::MySQL {
            quoted.push('\\');
        }
        if character == '\'' {
            quoted.push('\'');
        }
        quoted.push(character);
    }
    quoted.push('\'');
    quoted
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Import a transfer file through the connection.
///
/// SQL dumps are executed statement-by-statement, so they can create or
/// replace tables themselves; `target` is unused for them. CSV and TSV files
/// append rows to `target`, which must be provided.
///
/// The whole (decompressed) file is held in memory. That keeps the MVP honest
/// about scale: very large loads should go through the database's native
/// bulk loader instead.
pub async fn import_file(
    engine: &DatabaseEngine,
    target: Option<&TableRef>,
    path: &Path,
) -> Result<ImportReport> {
    let started = Instant::now();
    let kind = engine.kind();
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "import_file".to_owned(),
            kind,
        });
    }
    let file_format = detect_file_format(path)?;
    let source: PathBuf = path.to_owned();
    let raw = tokio::task::spawn_blocking(move || fs::read(source))
        .await
        .map_err(|error| DbxError::Io(error.to_string()))?
        .map_err(|error| DbxError::Io(error.to_string()))?;
    let script = decode_input(raw, file_format.gzipped)?;

    match file_format.format {
        DumpFormat::Sql => {
            let statements = split_sql_statements(&script);
            let mut executed = 0u64;
            for statement in &statements {
                engine.execute_sql(statement).await?;
                executed += 1;
            }
            Ok(ImportReport {
                statements_executed: executed,
                rows_inserted: 0,
                elapsed_ms: elapsed_ms_since(started),
            })
        }
        DumpFormat::Csv | DumpFormat::Tsv => {
            let target = target.ok_or_else(|| {
                DbxError::Parse("CSV and TSV imports require a target table".into())
            })?;
            import_delimited(engine, target, &script, file_format.format)
                .await
                .map(|report| ImportReport {
                    elapsed_ms: elapsed_ms_since(started),
                    ..report
                })
        }
    }
}

async fn import_delimited(
    engine: &DatabaseEngine,
    target: &TableRef,
    script: &str,
    format: DumpFormat,
) -> Result<ImportReport> {
    let kind = engine.kind();
    let columns = engine.describe_table(target).await?;
    let column_names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();

    let delimiter = format.delimiter().unwrap_or(b',');
    let mut reader = DelimitedReader::new(script.as_bytes(), delimiter);
    let header = reader
        .next_record()
        .map_err(io_error)?
        .ok_or_else(|| DbxError::Io("the file contains no header row".into()))?;
    let header_len = header.len();
    let mapped = map_header_columns(&header, &column_names)?;

    let max_params = max_params_per_statement(kind);
    let columns_per_row = mapped.len().max(1);
    let batch_limit = IMPORT_ROWS_PER_BATCH
        .min(max_params / columns_per_row)
        .max(1);

    let mut rows_inserted = 0u64;
    let mut pending: Vec<Vec<CellValue>> = Vec::with_capacity(batch_limit);
    while let Some(record) = reader.next_record().map_err(io_error)? {
        if record.len() != header_len {
            return Err(DbxError::Parse(format!(
                "row {} has {} field(s) but the header declares {}",
                rows_inserted + pending.len() as u64 + 1,
                record.len(),
                header_len
            )));
        }
        // Reorder each record from file/header order into table column
        // order so the multi-row insert pairs values with names correctly.
        pending.push(
            mapped
                .iter()
                .map(|(position, _)| delimited_field_to_cell(record[*position].clone()))
                .collect(),
        );
        if pending.len() >= batch_limit {
            flush_batch(engine, target, &mapped, &pending).await?;
            rows_inserted += pending.len() as u64;
            pending.clear();
        }
    }
    if !pending.is_empty() {
        flush_batch(engine, target, &mapped, &pending).await?;
        rows_inserted += pending.len() as u64;
    }

    Ok(ImportReport {
        statements_executed: 0,
        rows_inserted,
        elapsed_ms: 0,
    })
}

async fn flush_batch(
    engine: &DatabaseEngine,
    target: &TableRef,
    columns: &[(usize, String)],
    rows: &[Vec<CellValue>],
) -> Result<()> {
    let names: Vec<String> = columns.iter().map(|(_, name)| name.clone()).collect();
    let statement = build_multi_row_insert(engine.kind(), target, &names, rows)?;
    engine.execute(&statement).await?;
    Ok(())
}

/// Map header fields onto table columns. Exact names win; a case-insensitive
/// fallback covers files produced with different casing. Extra file columns
/// are ignored; every table column must be present. Returns
/// `(header_index, column_name)` pairs in table order.
fn map_header_columns(
    header: &[Option<String>],
    columns: &[String],
) -> Result<Vec<(usize, String)>> {
    let mut mapped = Vec::with_capacity(columns.len());
    for column in columns {
        let position = header
            .iter()
            .position(|field| field.as_deref() == Some(column.as_str()))
            .or_else(|| {
                header.iter().position(|field| {
                    field
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(column))
                })
            })
            .ok_or_else(|| {
                let available: Vec<&str> =
                    header.iter().filter_map(|field| field.as_deref()).collect();
                DbxError::Parse(format!(
                    "the file has no column named `{column}`; header contains: {}",
                    if available.is_empty() {
                        "(none)".to_owned()
                    } else {
                        available.join(", ")
                    }
                ))
            })?;
        mapped.push((position, column.clone()));
    }
    Ok(mapped)
}

fn delimited_field_to_cell(field: Option<String>) -> CellValue {
    // Unquoted empty fields arrive as `None` (NULL); every other value stays
    // text so numeric-looking data cannot lose leading zeros or formatting.
    // The target column coerces types deterministically.
    match field {
        None => CellValue::Null,
        Some(value) => CellValue::Text(value),
    }
}

fn max_params_per_statement(kind: DatabaseKind) -> usize {
    match kind {
        // Conservative bound for SQLite's historical 999-parameter default.
        DatabaseKind::SQLite => 900,
        DatabaseKind::PostgreSQL => 60_000,
        DatabaseKind::MySQL => 65_000,
        DatabaseKind::Redis => 0,
    }
}

// ---------------------------------------------------------------------------
// Delimited (CSV/TSV) reading and writing
// ---------------------------------------------------------------------------

/// Streaming RFC4180-style record reader over an in-memory byte slice.
///
/// Records are returned as `Vec<Option<String>>` where `None` marks an
/// unquoted empty field (`NULL`) and `Some("")` a quoted empty field.
/// Quoted fields may contain the delimiter, quotes, and line breaks; blank
/// lines between records are skipped; a UTF-8 BOM at the start is ignored.
pub struct DelimitedReader<'a> {
    input: &'a [u8],
    position: usize,
    delimiter: u8,
}

#[derive(Clone, Copy, PartialEq)]
enum FieldState {
    Start,
    Unquoted,
    Quoted,
    QuoteClosed,
}

impl<'a> DelimitedReader<'a> {
    pub fn new(input: &'a [u8], delimiter: u8) -> Self {
        Self {
            input,
            position: 0,
            delimiter,
        }
    }

    /// Return the next record, or `None` at end of input.
    pub fn next_record(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        if self.position == 0 && self.input.starts_with(&[0xEF, 0xBB, 0xBF]) {
            self.position = 3;
        }
        'records: loop {
            if self.position >= self.input.len() {
                return Ok(None);
            }
            let mut fields: Vec<Option<String>> = Vec::new();
            let mut field: Vec<u8> = Vec::new();
            let mut quoted_field = false;
            let mut state = FieldState::Start;
            let mut index = self.position;
            while index < self.input.len() {
                let byte = self.input[index];
                match state {
                    FieldState::Start => match byte {
                        b'"' => {
                            quoted_field = true;
                            state = FieldState::Quoted;
                            index += 1;
                        }
                        _ if byte == self.delimiter => {
                            fields.push(None);
                            index += 1;
                        }
                        b'\n' | b'\r' => {
                            index = skip_line_terminator(self.input, index);
                            if fields.is_empty() && field.is_empty() && !quoted_field {
                                // A blank line between records is skipped.
                                self.position = index;
                                continue 'records;
                            }
                            fields.push(take_field(&mut field, &mut quoted_field));
                            self.position = index;
                            return Ok(Some(fields));
                        }
                        _ => {
                            field.push(byte);
                            state = FieldState::Unquoted;
                            index += 1;
                        }
                    },
                    FieldState::Unquoted => match byte {
                        _ if byte == self.delimiter => {
                            fields.push(take_field(&mut field, &mut quoted_field));
                            state = FieldState::Start;
                            index += 1;
                        }
                        b'\n' | b'\r' => {
                            index = skip_line_terminator(self.input, index);
                            fields.push(take_field(&mut field, &mut quoted_field));
                            self.position = index;
                            return Ok(Some(fields));
                        }
                        _ => {
                            field.push(byte);
                            index += 1;
                        }
                    },
                    FieldState::Quoted => {
                        if byte == b'"' {
                            state = FieldState::QuoteClosed;
                        } else {
                            field.push(byte);
                        }
                        index += 1;
                    }
                    FieldState::QuoteClosed => {
                        if byte == b'"' {
                            // A doubled quote inside a quoted field is an
                            // escaped literal quote.
                            field.push(b'"');
                            state = FieldState::Quoted;
                            index += 1;
                        } else if byte == self.delimiter {
                            fields.push(take_field(&mut field, &mut quoted_field));
                            state = FieldState::Start;
                            index += 1;
                        } else if byte == b'\n' || byte == b'\r' {
                            index = skip_line_terminator(self.input, index);
                            fields.push(take_field(&mut field, &mut quoted_field));
                            self.position = index;
                            return Ok(Some(fields));
                        } else {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "malformed CSV/TSV field: unexpected text after closing quote",
                            ));
                        }
                    }
                }
            }
            // End of input with a record still open.
            if state == FieldState::Quoted {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "malformed CSV/TSV field: unterminated quote",
                ));
            }
            if state != FieldState::Start || !fields.is_empty() || !field.is_empty() {
                fields.push(take_field(&mut field, &mut quoted_field));
                self.position = self.input.len();
                return Ok(Some(fields));
            }
            self.position = self.input.len();
            return Ok(None);
        }
    }
}

fn skip_line_terminator(input: &[u8], index: usize) -> usize {
    if input[index] == b'\r' && input.get(index + 1) == Some(&b'\n') {
        index + 2
    } else {
        index + 1
    }
}

fn take_field(field: &mut Vec<u8>, quoted_field: &mut bool) -> Option<String> {
    let was_quoted = *quoted_field;
    *quoted_field = false;
    let value = String::from_utf8_lossy(field).into_owned();
    field.clear();
    if was_quoted || !value.is_empty() {
        Some(value)
    } else {
        None
    }
}

/// Write one delimiter-separated record followed by a newline.
///
/// `None` fields are written bare and therefore read back as `NULL`; quoted
/// fields are escaped by doubling embedded quotes. A field is quoted whenever
/// it contains the delimiter, a quote, or a line break, or when it is an
/// explicitly non-NULL empty string.
fn write_delimited_record(
    output: &mut Vec<u8>,
    delimiter: u8,
    fields: &[Option<&str>],
) -> Result<()> {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            output.push(delimiter);
        }
        let Some(field) = field else {
            continue;
        };
        if field.is_empty()
            || field.contains(delimiter as char)
            || field.contains('"')
            || field.contains('\n')
            || field.contains('\r')
        {
            output.push(b'"');
            for character in field.chars() {
                if character == '"' {
                    output.extend_from_slice(b"\"\"");
                } else {
                    let mut buffer = [0u8; 4];
                    output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
                }
            }
            output.push(b'"');
        } else {
            output.extend_from_slice(field.as_bytes());
        }
    }
    output.push(b'\n');
    Ok(())
}

/// Convert one cell into its delimited-file representation: `None` for NULL
/// (an unquoted empty field), hex text for binary values, display text for
/// everything else.
fn delimited_value_field(value: &CellValue) -> Option<String> {
    match value {
        CellValue::Null => None,
        CellValue::Bytes(bytes) => {
            let mut hex = String::with_capacity(bytes.len() * 2);
            for byte in bytes {
                use std::fmt::Write;
                let _ = write!(hex, "{byte:02x}");
            }
            Some(hex)
        }
        other => Some(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// SQL script splitting
// ---------------------------------------------------------------------------

/// Split a SQL script into individual statements.
///
/// Understands single/double-quoted strings and backtick identifiers,
/// `--`/`#` line comments, nested block comments, PostgreSQL dollar-quoted
/// strings, and MySQL-style `DELIMITER` directives so routine bodies from
/// real-world dumps survive intact. Trailing content without a terminator is
/// returned as one final statement when it is not blank. The function never
/// fails: malformed scripts simply produce statements the engine will
/// reject with its own diagnostics.
pub fn split_sql_statements(script: &str) -> Vec<String> {
    #[derive(Clone)]
    enum State {
        Normal,
        LineComment,
        BlockComment(usize),
        SingleQuote,
        DoubleQuote,
        Backtick,
        Dollar(String),
    }

    let characters: Vec<char> = script.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut delimiter = ";".to_owned();
    let mut state = State::Normal;
    let mut at_line_start = true;
    let mut index = 0usize;

    while index < characters.len() {
        let character = characters[index];
        let matches_here = |needle: &str, at: usize| {
            characters[at..].starts_with(needle.chars().collect::<Vec<_>>().as_slice())
        };
        match state.clone() {
            State::Normal => {
                if at_line_start
                    && current.trim().is_empty()
                    && matches_here_ci(&characters, index, "delimiter")
                    && characters
                        .get(index + "delimiter".len())
                        .is_some_and(|next| next.is_whitespace())
                {
                    index += "delimiter".len();
                    let mut token = String::new();
                    while let Some(&next) = characters.get(index) {
                        if next == '\n' || next == '\r' {
                            break;
                        }
                        token.push(next);
                        index += 1;
                    }
                    let trimmed = token.trim();
                    if !trimmed.is_empty() {
                        delimiter = trimmed.to_owned();
                    }
                    continue;
                }
                if character == '-' && matches_here("--", index) {
                    state = State::LineComment;
                    index += 2;
                    continue;
                }
                if character == '#' {
                    state = State::LineComment;
                    index += 1;
                    continue;
                }
                if character == '/' && matches_here("/*", index) {
                    state = State::BlockComment(1);
                    index += 2;
                    continue;
                }
                if matches_here(&delimiter, index) {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() {
                        statements.push(trimmed.to_owned());
                    }
                    current.clear();
                    at_line_start = true;
                    index += delimiter.chars().count();
                    continue;
                }
                match character {
                    '\'' => {
                        state = State::SingleQuote;
                        current.push(character);
                        index += 1;
                    }
                    '"' => {
                        state = State::DoubleQuote;
                        current.push(character);
                        index += 1;
                    }
                    '`' => {
                        state = State::Backtick;
                        current.push(character);
                        index += 1;
                    }
                    '$' => {
                        if let Some(tag) = parse_dollar_tag(&characters[index..]) {
                            let token_length = tag.chars().count() + 2;
                            current.extend(characters[index..index + token_length].iter());
                            index += token_length;
                            state = State::Dollar(tag);
                        } else {
                            current.push(character);
                            index += 1;
                        }
                    }
                    _ => {
                        if !character.is_whitespace() {
                            at_line_start = false;
                        }
                        current.push(character);
                        index += 1;
                    }
                }
            }
            State::LineComment => {
                if character == '\n' {
                    state = State::Normal;
                    at_line_start = true;
                    current.push(character);
                }
                index += 1;
            }
            State::BlockComment(depth) => {
                if character == '/' && matches_here("/*", index) {
                    state = State::BlockComment(depth + 1);
                    index += 2;
                } else if character == '*' && matches_here("*/", index) {
                    state = if depth <= 1 {
                        State::Normal
                    } else {
                        State::BlockComment(depth - 1)
                    };
                    index += 2;
                } else {
                    index += 1;
                }
            }
            State::SingleQuote => {
                current.push(character);
                if character == '\\' {
                    if let Some(&next) = characters.get(index + 1) {
                        current.push(next);
                        index += 2;
                        continue;
                    }
                } else if character == '\'' {
                    if characters.get(index + 1) == Some(&'\'') {
                        current.push('\'');
                        index += 2;
                        continue;
                    }
                    state = State::Normal;
                }
                index += 1;
            }
            State::DoubleQuote => {
                current.push(character);
                if character == '"' {
                    if characters.get(index + 1) == Some(&'"') {
                        current.push('"');
                        index += 2;
                        continue;
                    }
                    state = State::Normal;
                }
                index += 1;
            }
            State::Backtick => {
                current.push(character);
                if character == '`' {
                    if characters.get(index + 1) == Some(&'`') {
                        current.push('`');
                        index += 2;
                        continue;
                    }
                    state = State::Normal;
                }
                index += 1;
            }
            State::Dollar(tag) => {
                let closing = format!("${tag}$");
                if matches_here(&closing, index) {
                    current.push_str(&closing);
                    index += closing.chars().count();
                    state = State::Normal;
                } else {
                    current.push(character);
                    index += 1;
                }
            }
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        statements.push(trimmed.to_owned());
    }
    statements
}

fn matches_here_ci(characters: &[char], index: usize, needle: &str) -> bool {
    needle.chars().enumerate().all(|(offset, expected)| {
        characters
            .get(index + offset)
            .is_some_and(|found| found.eq_ignore_ascii_case(&expected))
    })
}

/// Parse `$tag$` starting at `characters[0]`, returning the inner tag when
/// the shape matches. The empty tag (`$$`) is valid PostgreSQL syntax.
fn parse_dollar_tag(characters: &[char]) -> Option<String> {
    let mut tag = String::new();
    for &character in characters.iter().skip(1) {
        match character {
            '$' => return Some(tag),
            _ if character.is_ascii_alphanumeric() || character == '_' => tag.push(character),
            _ => return None,
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Compression helpers
// ---------------------------------------------------------------------------

fn decode_input(raw: Vec<u8>, gzipped: bool) -> Result<String> {
    if gzipped {
        let mut decoder = GzDecoder::new(&raw[..]);
        let mut text = String::new();
        decoder
            .read_to_string(&mut text)
            .map_err(|error| DbxError::Io(format!("could not decompress gzip input: {error}")))?;
        Ok(text)
    } else {
        String::from_utf8(raw)
            .map_err(|error| DbxError::Io(format!("file is not valid UTF-8: {error}")))
    }
}

fn gzip_encode(bytes: Vec<u8>) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&bytes)
        .and_then(|_| encoder.finish())
        .map_err(|error| DbxError::Io(format!("could not compress gzip output: {error}")))
}

fn io_error(error: io::Error) -> DbxError {
    DbxError::Io(error.to_string())
}

fn elapsed_ms_since(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_file_format_handles_plain_and_gzipped_extensions() {
        let cases = [
            ("dump.sql", DumpFormat::Sql, false),
            ("dump.SQL.gz", DumpFormat::Sql, true),
            ("rows.csv", DumpFormat::Csv, false),
            ("rows.CSV.GZ", DumpFormat::Csv, true),
            ("rows.tsv", DumpFormat::Tsv, false),
            ("rows.tsv.gz", DumpFormat::Tsv, true),
        ];
        for (name, format, gzipped) in cases {
            let detected = detect_file_format(Path::new(name)).unwrap();
            assert_eq!(detected.format, format, "{name}");
            assert_eq!(detected.gzipped, gzipped, "{name}");
        }
        assert!(detect_file_format(Path::new("rows.xlsx")).is_err());
        assert!(detect_file_format(Path::new("noext")).is_err());
    }

    #[test]
    fn delimited_reader_parses_quoted_embedded_and_null_fields() {
        let input = b"\xEF\xBB\xBFid,name,note\r\n1,\"plain\",x\r\n2,\"has, comma\",\"line\nbreak\"\r\n3,\"doubled \"\" quote\",,,\r\n4,\"\",\r\n\r\n5,last\r\n";
        let mut reader = DelimitedReader::new(input, b',');
        let header = reader.next_record().unwrap().unwrap();
        assert_eq!(header[0].as_deref(), Some("id"));
        let first = reader.next_record().unwrap().unwrap();
        assert_eq!(
            first,
            vec![Some("1".into()), Some("plain".into()), Some("x".into())]
        );
        let second = reader.next_record().unwrap().unwrap();
        assert_eq!(
            second,
            vec![
                Some("2".into()),
                Some("has, comma".into()),
                Some("line\nbreak".into())
            ]
        );
        let third = reader.next_record().unwrap().unwrap();
        assert_eq!(third[0], Some("3".into()));
        assert_eq!(third[1], Some("doubled \" quote".into()));
        // Trailing delimiter produces NULL fields.
        assert_eq!(third[2], None);
        assert_eq!(third[3], None);
        let fourth = reader.next_record().unwrap().unwrap();
        assert_eq!(fourth, vec![Some("4".into()), Some(String::new()), None]);
        let fifth = reader.next_record().unwrap().unwrap();
        assert_eq!(fifth, vec![Some("5".into()), Some("last".into())]);
        assert!(reader.next_record().unwrap().is_none());
    }

    #[test]
    fn delimited_reader_rejects_unterminated_quotes() {
        let mut reader = DelimitedReader::new(b"a,\"open".as_slice(), b',');
        assert!(reader.next_record().is_err());
        let mut malformed = DelimitedReader::new(b"\"closed\"junk".as_slice(), b',');
        assert!(malformed.next_record().is_err());
    }

    #[test]
    fn delimited_records_round_trip_through_the_writer() {
        let fields = vec![
            Some("plain"),
            Some("has,comma"),
            Some("quote\"inside"),
            Some("line\nbreak"),
            None,
            Some(""),
        ];
        let mut output = Vec::new();
        write_delimited_record(&mut output, b',', &fields).unwrap();
        let mut reader = DelimitedReader::new(&output, b',');
        let parsed = reader.next_record().unwrap().unwrap();
        assert_eq!(parsed.len(), fields.len());
        for (original, restored) in fields.iter().zip(&parsed) {
            assert_eq!(restored.as_deref(), *original);
        }
        // TSV quoting follows the active delimiter only.
        let tsv_output = {
            let mut out = Vec::new();
            write_delimited_record(&mut out, b'\t', &fields).unwrap();
            out
        };
        assert!(String::from_utf8_lossy(&tsv_output).contains("has,comma"));
        let mut tsv_reader = DelimitedReader::new(&tsv_output, b'\t');
        let tsv_parsed = tsv_reader.next_record().unwrap().unwrap();
        assert_eq!(tsv_parsed[1], Some("has,comma".into()));
    }

    #[test]
    fn sql_statements_split_across_strings_comments_and_delimiters() {
        let script = "-- leading comment;\nSELECT 'a;b' FROM t; /* block ; comment */\nSELECT 2;# trailing\nUPDATE t SET x = 'it''s';\n";
        let statements = split_sql_statements(script);
        assert_eq!(statements.len(), 3);
        assert_eq!(statements[0], "SELECT 'a;b' FROM t");
        assert_eq!(statements[1], "SELECT 2");
        assert_eq!(statements[2], "UPDATE t SET x = 'it''s'");
    }

    #[test]
    fn sql_statements_survive_dollar_quoting_and_nested_block_comments() {
        let script = "CREATE FUNCTION f() RETURNS void AS $body$\nBEGIN\n  PERFORM 1; /* nested /* comment */ still inside */\nEND;\n$body$ LANGUAGE plpgsql;\nSELECT 1;";
        let statements = split_sql_statements(script);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].contains("PERFORM 1;"));
        assert!(statements[0].ends_with("$body$ LANGUAGE plpgsql"));
        assert_eq!(statements[1], "SELECT 1");
    }

    #[test]
    fn sql_statements_honor_mysql_delimiter_directives() {
        let script = "DELIMITER ;;\nCREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END;;\nDELIMITER ;\nCALL p();\n";
        let statements = split_sql_statements(script);
        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements[0],
            "CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END"
        );
        assert_eq!(statements[1], "CALL p()");
    }

    #[test]
    fn sql_statements_keep_backtick_identifiers_intact() {
        let statements = split_sql_statements("SELECT `weird;name` FROM `t``ick`;");
        assert_eq!(statements, vec!["SELECT `weird;name` FROM `t``ick`"]);
    }

    #[test]
    fn sql_insert_rendering_quotes_and_escapes_per_dialect() {
        let table = TableRef::in_schema("public", "events");
        let columns = vec!["name".to_owned(), "payload".to_owned()];
        let values = vec![
            CellValue::Text("O'Reilly \\ N".into()),
            CellValue::Json(serde_json::json!({ "ok": true })),
        ];
        let postgres =
            render_sql_insert(DatabaseKind::PostgreSQL, &table, &columns, &values).unwrap();
        assert_eq!(
            postgres,
            "INSERT INTO \"public\".\"events\" (\"name\", \"payload\") VALUES ('O''Reilly \\ N', '{\"ok\":true}')"
        );
        let mysql = render_sql_insert(
            DatabaseKind::MySQL,
            &TableRef::new("events"),
            &columns,
            &values,
        )
        .unwrap();
        assert!(mysql.contains("'O''Reilly \\\\ N'"), "{mysql}");

        // A column/value count mismatch fails instead of emitting broken SQL.
        let mismatch = render_sql_insert(
            DatabaseKind::SQLite,
            &TableRef::new("t"),
            &["v".to_owned()],
            &[CellValue::Null, CellValue::Null],
        );
        assert!(mismatch.is_err());

        let literals = render_sql_insert(
            DatabaseKind::SQLite,
            &TableRef::new("t"),
            &["v".to_owned()],
            &[CellValue::Null],
        )
        .unwrap();
        assert_eq!(literals, "INSERT INTO \"t\" (\"v\") VALUES (NULL)");
    }

    #[test]
    fn sql_schema_rendering_keeps_selected_foreign_keys_and_primary_keys() {
        let parent = TableRef::in_schema("public", "accounts");
        let child = TableRef::in_schema("public", "events");
        let structure = TableStructure {
            columns: vec![
                ColumnInfo {
                    name: "id".into(),
                    data_type: "integer".into(),
                    enum_values: Vec::new(),
                    nullable: false,
                    ordinal: 1,
                    primary_key: true,
                },
                ColumnInfo {
                    name: "account_id".into(),
                    data_type: "integer".into(),
                    enum_values: Vec::new(),
                    nullable: false,
                    ordinal: 2,
                    primary_key: false,
                },
            ],
            foreign_keys: vec![crate::ForeignKeyInfo {
                constraint_name: Some("events_account_id_fkey".into()),
                columns: vec!["account_id".into()],
                referenced_schema: Some("public".into()),
                referenced_table: "accounts".into(),
                referenced_columns: vec!["id".into()],
                on_update: Some(crate::ReferentialAction::Cascade),
                on_delete: Some(crate::ReferentialAction::SetNull),
            }],
        };

        let sql =
            render_sql_schema(DatabaseKind::PostgreSQL, &child, &structure, &[parent]).unwrap();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"public\".\"events\""));
        assert!(sql.contains("PRIMARY KEY (\"id\")"));
        assert!(sql.contains("CONSTRAINT \"events_account_id_fkey\""));
        assert!(sql.contains("ON UPDATE CASCADE ON DELETE SET NULL"));
    }

    #[test]
    fn non_sqlite_schema_rendering_can_defer_foreign_keys() {
        let parent = TableRef::in_schema("public", "accounts");
        let child = TableRef::in_schema("public", "events");
        let structure = TableStructure {
            columns: vec![
                ColumnInfo {
                    name: "id".into(),
                    data_type: "integer".into(),
                    enum_values: Vec::new(),
                    nullable: false,
                    ordinal: 1,
                    primary_key: true,
                },
                ColumnInfo {
                    name: "account_id".into(),
                    data_type: "integer".into(),
                    enum_values: Vec::new(),
                    nullable: false,
                    ordinal: 2,
                    primary_key: false,
                },
            ],
            foreign_keys: vec![crate::ForeignKeyInfo {
                constraint_name: Some("events_account_id_fkey".into()),
                columns: vec!["account_id".into()],
                referenced_schema: Some("public".into()),
                referenced_table: "accounts".into(),
                referenced_columns: vec!["id".into()],
                on_update: Some(crate::ReferentialAction::Cascade),
                on_delete: Some(crate::ReferentialAction::SetNull),
            }],
        };
        let selected = [parent];

        let schema = render_sql_schema_without_foreign_keys(
            DatabaseKind::PostgreSQL,
            &child,
            &structure,
            &selected,
        )
        .unwrap();
        assert!(!schema.contains("FOREIGN KEY"));

        let mut output = Vec::new();
        append_sql_foreign_keys(
            DatabaseKind::PostgreSQL,
            &child,
            &structure,
            &selected,
            &mut output,
        )
        .unwrap();
        let constraints = String::from_utf8(output).unwrap();
        assert_eq!(
            constraints,
            "ALTER TABLE \"public\".\"events\" ADD CONSTRAINT \"events_account_id_fkey\" FOREIGN KEY (\"account_id\") REFERENCES \"public\".\"accounts\" (\"id\") ON UPDATE CASCADE ON DELETE SET NULL;\n"
        );
    }

    #[tokio::test]
    async fn sqlite_database_export_emits_all_schema_before_foreign_key_data() {
        let source = DatabaseEngine::connect(crate::ConnectionConfig::new(
            crate::DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        source
            .execute_sql("PRAGMA foreign_keys = ON")
            .await
            .unwrap();
        source
            .execute_sql(
                "CREATE TABLE dbx_transfer_parent (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            )
            .await
            .unwrap();
        source
            .execute_sql(
                "CREATE TABLE dbx_transfer_child (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL, FOREIGN KEY (parent_id) REFERENCES dbx_transfer_parent (id))",
            )
            .await
            .unwrap();
        source
            .execute_sql("INSERT INTO dbx_transfer_parent (id, name) VALUES (1, 'Ada')")
            .await
            .unwrap();
        source
            .execute_sql("INSERT INTO dbx_transfer_child (id, parent_id) VALUES (7, 1)")
            .await
            .unwrap();

        let directory = tempfile::tempdir().unwrap();
        let request = DatabaseExportRequest {
            // Reverse the dependency order to prove the exporter calculates a
            // safe order instead of trusting the navigator selection order.
            tables: vec![
                TableRef::new("dbx_transfer_child"),
                TableRef::new("dbx_transfer_parent"),
            ],
            output_directory: directory.path().to_owned(),
            output_name: "foreign-keys".into(),
            format: DumpFormat::Sql,
            schema_only: false,
            gzipped: false,
        };
        let summary = export_database(&source, &request).await.unwrap();
        assert_eq!(summary.rows_exported, 2);

        let path = directory.path().join("foreign-keys.sql");
        let dump = fs::read_to_string(&path).unwrap();
        let first_insert = dump.find("INSERT INTO").unwrap();
        let parent_create = dump
            .find("CREATE TABLE IF NOT EXISTS \"dbx_transfer_parent\"")
            .unwrap();
        let child_create = dump
            .find("CREATE TABLE IF NOT EXISTS \"dbx_transfer_child\"")
            .unwrap();
        let parent_insert = dump.find("INSERT INTO \"dbx_transfer_parent\"").unwrap();
        let child_insert = dump.find("INSERT INTO \"dbx_transfer_child\"").unwrap();
        assert!(parent_create < child_create);
        assert!(child_create < first_insert);
        assert!(parent_insert < child_insert);
        assert!(dump.contains("FOREIGN KEY (\"parent_id\")"));

        let restored = DatabaseEngine::connect(crate::ConnectionConfig::new(
            crate::DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        restored
            .execute_sql("PRAGMA foreign_keys = ON")
            .await
            .unwrap();
        let report = import_database(&restored, &path).await.unwrap();
        assert_eq!(report.statements_executed, 4);
        let violations = restored
            .query("PRAGMA foreign_key_check", QueryOptions { max_rows: None })
            .await
            .unwrap();
        assert!(violations.rows.is_empty());
    }

    #[tokio::test]
    async fn sqlite_database_export_supports_selected_tables_and_schema_only() {
        let engine = DatabaseEngine::connect(crate::ConnectionConfig::new(
            crate::DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .execute_sql(
                "CREATE TABLE dbx_transfer_accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            )
            .await
            .unwrap();
        engine
            .execute_sql("CREATE TABLE dbx_transfer_events (id INTEGER PRIMARY KEY, account_id INTEGER, note TEXT)")
            .await
            .unwrap();
        engine
            .execute_sql("INSERT INTO dbx_transfer_accounts (id, name) VALUES (1, 'Ada')")
            .await
            .unwrap();
        engine
            .execute_sql(
                "INSERT INTO dbx_transfer_events (id, account_id, note) VALUES (7, 1, 'created')",
            )
            .await
            .unwrap();

        let directory = tempfile::tempdir().unwrap();
        let tables = vec![
            TableRef::new("dbx_transfer_accounts"),
            TableRef::new("dbx_transfer_events"),
        ];
        let data_request = DatabaseExportRequest {
            tables: tables.clone(),
            output_directory: directory.path().to_owned(),
            output_name: "database.sql".into(),
            format: DumpFormat::Sql,
            schema_only: false,
            gzipped: false,
        };
        let summary = export_database(&engine, &data_request).await.unwrap();
        assert_eq!(summary.tables_exported, 2);
        assert_eq!(summary.files_written, 1);
        assert_eq!(summary.rows_exported, 2);
        let data = fs::read_to_string(directory.path().join("database.sql")).unwrap();
        assert!(data.contains("CREATE TABLE IF NOT EXISTS \"dbx_transfer_accounts\""));
        assert!(data.contains("INSERT INTO \"dbx_transfer_events\""));

        let delimited_request = DatabaseExportRequest {
            output_name: "rows".into(),
            format: DumpFormat::Csv,
            ..data_request.clone()
        };
        let delimited_summary = export_database(&engine, &delimited_request).await.unwrap();
        assert_eq!(delimited_summary.files_written, 2);
        assert!(
            directory
                .path()
                .join("rows_dbx_transfer_accounts.csv")
                .is_file()
        );
        assert!(
            directory
                .path()
                .join("rows_dbx_transfer_events.csv")
                .is_file()
        );

        let schema_request = DatabaseExportRequest {
            output_name: "schema-only".into(),
            schema_only: true,
            ..data_request
        };
        let schema_summary = export_database(&engine, &schema_request).await.unwrap();
        assert_eq!(schema_summary.rows_exported, 0);
        let schema = fs::read_to_string(directory.path().join("schema-only.sql")).unwrap();
        assert!(schema.contains("CREATE TABLE IF NOT EXISTS"));
        assert!(!schema.contains("INSERT INTO"));
    }

    #[tokio::test]
    async fn sqlite_round_trips_a_gzipped_sql_dump() {
        let engine = DatabaseEngine::connect(crate::ConnectionConfig::new(
            crate::DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        seed_events_table(&engine).await;

        let directory = tempfile::tempdir().unwrap();
        let dump_path = directory.path().join("events.sql.gz");
        let export = export_table(&engine, &TableRef::new("dbx_transfer_events"), &dump_path)
            .await
            .unwrap();
        assert_eq!(export.rows_exported, 3);
        assert!(export.gzipped);
        let raw = fs::read(&dump_path).unwrap();
        assert_eq!(&raw[..2], &[0x1f, 0x8b]);

        engine
            .execute_sql("DELETE FROM dbx_transfer_events")
            .await
            .unwrap();
        let report = import_file(
            &engine,
            Some(&TableRef::new("dbx_transfer_events")),
            &dump_path,
        )
        .await
        .unwrap();
        assert_eq!(report.statements_executed, 3);

        let result = engine
            .query_table(
                &TableRef::new("dbx_transfer_events"),
                &[],
                &[],
                &[],
                None,
                QueryOptions { max_rows: None },
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 3);
        let title = &result.rows[0].values[1];
        assert_eq!(
            *title,
            CellValue::Text("O'Reilly says \"hi\"\nagain\ttabs".into())
        );
    }

    #[tokio::test]
    async fn sqlite_round_trips_csv_with_null_semantics_and_column_reordering() {
        let engine = DatabaseEngine::connect(crate::ConnectionConfig::new(
            crate::DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .execute_sql(
                "CREATE TABLE dbx_transfer_people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT)",
            )
            .await
            .unwrap();

        let directory = tempfile::tempdir().unwrap();
        let csv_path = directory.path().join("people.csv");
        fs::write(
            &csv_path,
            "note,name,id\nempty-quote,\"Ann, Lee\",1\n,Kept name,2\n\"line\nbreak\",Bo,3\n",
        )
        .unwrap();
        let report = import_file(
            &engine,
            Some(&TableRef::new("dbx_transfer_people")),
            &csv_path,
        )
        .await
        .unwrap();
        assert_eq!(report.rows_inserted, 3);

        let result = engine
            .query_table(
                &TableRef::new("dbx_transfer_people"),
                &[],
                &[],
                &[crate::Order {
                    column: "id".into(),
                    direction: crate::OrderDirection::Ascending,
                }],
                None,
                QueryOptions { max_rows: None },
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.rows[0].values[0], CellValue::Integer(1));
        assert_eq!(result.rows[0].values[1], CellValue::Text("Ann, Lee".into()));
        assert_eq!(
            result.rows[0].values[2],
            CellValue::Text("empty-quote".into())
        );
        assert_eq!(result.rows[1].values[0], CellValue::Integer(2));
        assert_eq!(
            result.rows[1].values[1],
            CellValue::Text("Kept name".into())
        );
        assert_eq!(result.rows[1].values[2], CellValue::Null);
        assert_eq!(
            result.rows[2].values[2],
            CellValue::Text("line\nbreak".into())
        );

        // Export back to TSV and confirm NULL renders as an empty unquoted
        // field while the embedded newline stays quoted.
        let tsv_path = directory.path().join("people.tsv");
        let export = export_table(&engine, &TableRef::new("dbx_transfer_people"), &tsv_path)
            .await
            .unwrap();
        assert_eq!(export.rows_exported, 3);
        assert_eq!(export.format, DumpFormat::Tsv);
        let text = fs::read_to_string(&tsv_path).unwrap();
        assert!(text.starts_with("id\tname\tnote\n"));
        assert!(text.contains("3\tBo\t\"line\nbreak\"\n"));

        // A header that does not cover every table column fails loudly.
        let bad_path = directory.path().join("bad.csv");
        fs::write(&bad_path, "name,id\nX,9\n").unwrap();
        let error = import_file(
            &engine,
            Some(&TableRef::new("dbx_transfer_people")),
            &bad_path,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("no column named `note`"));

        // SQL dumps do not need a target table.
        let script_path = directory.path().join("setup.sql");
        fs::write(&script_path, "CREATE TABLE dbx_transfer_direct (a INT);\nINSERT INTO dbx_transfer_direct VALUES (7);\n").unwrap();
        let report = import_database(&engine, &script_path).await.unwrap();
        assert_eq!(report.statements_executed, 2);
        let direct = engine
            .query(
                "SELECT a FROM dbx_transfer_direct",
                QueryOptions { max_rows: None },
            )
            .await
            .unwrap();
        assert_eq!(direct.rows[0].values[0], CellValue::Integer(7));
    }

    async fn seed_events_table(engine: &DatabaseEngine) {
        engine
            .execute_sql(
                "CREATE TABLE dbx_transfer_events (id INTEGER PRIMARY KEY, title TEXT NOT NULL, note TEXT, score REAL)",
            )
            .await
            .unwrap();
        let rows = [
            (
                1i64,
                "O'Reilly says \"hi\"\nagain\ttabs",
                Some("keep, this"),
                1.5f64,
            ),
            (2, "plain", None, 2.0),
            (3, "unicode ✓ ✓", Some(""), -0.25),
        ];
        for (id, title, note, score) in rows {
            engine
                .execute(&crate::SqlStatement::new(
                    "INSERT INTO dbx_transfer_events (id, title, note, score) VALUES (?, ?, ?, ?)",
                    vec![
                        CellValue::Integer(id),
                        CellValue::Text(title.into()),
                        note.map(|text| CellValue::Text(text.into()))
                            .unwrap_or(CellValue::Null),
                        CellValue::Real(score),
                    ],
                ))
                .await
                .unwrap();
        }
    }
}
