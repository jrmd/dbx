//! Table data transfer between a connection and local files.
//!
//! Supported formats are SQL dumps (`.sql`), CSV (`.csv`), and TSV
//! (`.tsv`), each optionally gzip-compressed with a `.gz` suffix. SQL dumps
//! contain dialect-aware `INSERT` statements; the importer owns its transaction. CSV
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
    collections::{BTreeSet, HashMap, VecDeque},
    fs,
    io::{self, BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use crate::console::SqlTransaction;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Default)]
pub struct TransferControl {
    cancellation: crate::QueryCancellation,
    rows: Arc<AtomicU64>,
    statements: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    log: Arc<std::sync::Mutex<String>>,
}
impl TransferControl {
    pub(crate) async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }
    pub(crate) fn set_bytes(&self, bytes: u64) {
        self.bytes.store(bytes, Ordering::Relaxed);
    }
    pub fn byte_progress(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    pub fn log(&self) -> String {
        self.log
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    pub(crate) fn add_log(&self, text: &str) {
        let mut log = self.log.lock().unwrap_or_else(|error| error.into_inner());
        log.push_str(text);
        log.push('\n');
        if log.len() > 64 * 1024 {
            let mut start = log.len() - 64 * 1024;
            while !log.is_char_boundary(start) {
                start += 1;
            }
            log.drain(..start);
        }
    }
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn progress(&self) -> (u64, u64) {
        (
            self.rows.load(Ordering::Relaxed),
            self.statements.load(Ordering::Relaxed),
        )
    }
}
tokio::task_local! { static TRANSFER_CONTROL: TransferControl; }
pub async fn with_transfer_control<T>(
    control: TransferControl,
    work: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    TRANSFER_CONTROL.scope(control.clone(), async move {
        tokio::select! {
            biased;
            _ = control.cancellation.cancelled() => Err(DbxError::Interrupted("Transfer cancelled. Uncommitted imports roll back and unfinished temporary files are removed. Previously completed export files remain.".into())),
            result = work => result,
        }
    }).await
}
pub(crate) fn report_transfer(rows: u64, statements: u64) {
    let _ = TRANSFER_CONTROL.try_with(|control| {
        control.rows.fetch_add(rows, Ordering::Relaxed);
        control.statements.fetch_add(statements, Ordering::Relaxed);
    });
}
use flate2::{Compression, read::GzDecoder, write::GzEncoder};

use crate::script::{MAX_TRANSFER_RECORD_BYTES, SqlScriptReader, checked_split_sql_for};
use crate::{
    CellValue, ColumnInfo, DatabaseEngine, DatabaseKind, DbxError, Filter, MutationValue, Page,
    QueryOptions, Result, RowChange, RowData, TableRef, TableStructure,
    sql::{build_multi_row_insert_with_columns, quote_identifier, quote_table},
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
    pub consistent_snapshot: bool,
}

/// Streaming query export executes exactly one read statement on a fresh snapshot.
/// It never resumes or executes an editor's pending transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueryExportFormat {
    Csv,
    Tsv,
    JsonLines,
}

pub async fn export_query(
    engine: &DatabaseEngine,
    sql: &str,
    path: &Path,
    format: QueryExportFormat,
) -> Result<u64> {
    let statements = checked_split_sql_for(Some(engine.kind()), sql)?;
    if statements.len() != 1 {
        return Err(DbxError::Query(
            "Full query export requires exactly one read statement".into(),
        ));
    }
    crate::protected::ensure_query(engine.kind(), &statements[0])?;
    let operation =
        crate::sqlx_engine::top_level_operation_keyword(&statements[0]).unwrap_or_default();
    if ![
        "SELECT", "VALUES", "TABLE", "SHOW", "DESCRIBE", "DESC", "EXPLAIN",
    ]
    .contains(&operation.to_ascii_uppercase().as_str())
    {
        return Err(DbxError::Query(
            "Full query export accepts read statements only".into(),
        ));
    }
    let mut transaction = SqlTransaction::begin(engine, true).await?;
    let mut output = ExportFile::new(path, false)?;
    let rows = transaction
        .stream_rows(&statements[0], |columns, values| {
            match (format, values) {
                (QueryExportFormat::Csv | QueryExportFormat::Tsv, None) => {
                    let delimiter = if format == QueryExportFormat::Csv {
                        ','
                    } else {
                        '\t'
                    };
                    let header = columns
                        .iter()
                        .map(|column| delimited_text(&column.name, delimiter))
                        .collect::<Vec<_>>()
                        .join(&delimiter.to_string());
                    writeln!(output, "{header}").map_err(io_error)?;
                }
                (QueryExportFormat::Csv | QueryExportFormat::Tsv, Some(values)) => {
                    let delimiter = if format == QueryExportFormat::Csv {
                        ','
                    } else {
                        '\t'
                    };
                    let fields = values
                        .iter()
                        .map(|value| match value {
                            CellValue::Null => String::new(),
                            _ => delimited_text(&value.to_string(), delimiter),
                        })
                        .collect::<Vec<_>>();
                    writeln!(output, "{}", fields.join(&delimiter.to_string()))
                        .map_err(io_error)?;
                    report_transfer(1, 0);
                }
                (QueryExportFormat::JsonLines, Some(values)) => {
                    // An array retains duplicate column names and exact typed CellValues.
                    serde_json::to_writer(&mut output, &RowData::new(values))
                        .map_err(|error| DbxError::Io(error.to_string()))?;
                    writeln!(output).map_err(io_error)?;
                    report_transfer(1, 0);
                }
                (QueryExportFormat::JsonLines, None) => {
                    serde_json::to_writer(&mut output, &serde_json::json!({"columns": columns}))
                        .map_err(|error| DbxError::Io(error.to_string()))?;
                    writeln!(output).map_err(io_error)?;
                }
            }
            Ok(())
        })
        .await?;
    transaction.commit().await?;
    output.finish()?;
    report_transfer(0, 1);
    Ok(rows)
}

fn delimited_text(value: &str, delimiter: char) -> String {
    if value.is_empty() || value.contains([delimiter, '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.into()
    }
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
    pub consistent_snapshot: bool,
}

/// Output becomes visible only after the complete stream and gzip footer have
/// been flushed. A failed/cancelled export leaves the previous file untouched.
struct ExportFile {
    temporary: tempfile::NamedTempFile,
    destination: PathBuf,
    writer: ExportWriter,
}
enum ExportWriter {
    Plain(BufWriter<fs::File>),
    Gzip(GzEncoder<BufWriter<fs::File>>),
}
impl ExportFile {
    fn new(path: &Path, gzipped: bool) -> Result<Self> {
        let temporary = tempfile::NamedTempFile::new_in(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .map_err(io_error)?;
        let buffer = BufWriter::new(temporary.as_file().try_clone().map_err(io_error)?);
        let writer = if gzipped {
            ExportWriter::Gzip(GzEncoder::new(buffer, Compression::default()))
        } else {
            ExportWriter::Plain(buffer)
        };
        Ok(Self {
            temporary,
            destination: path.to_owned(),
            writer,
        })
    }
    fn finish(self) -> Result<()> {
        let mut buffer = match self.writer {
            ExportWriter::Plain(buffer) => buffer,
            ExportWriter::Gzip(encoder) => encoder.finish().map_err(io_error)?,
        };
        buffer.flush().map_err(io_error)?;
        buffer.get_ref().sync_all().map_err(io_error)?;
        drop(buffer);
        self.temporary
            .persist(&self.destination)
            .map_err(|error| io_error(error.error))?;
        Ok(())
    }
}
impl Write for ExportFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match &mut self.writer {
            ExportWriter::Plain(writer) => writer.write(bytes),
            ExportWriter::Gzip(writer) => writer.write(bytes),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match &mut self.writer {
            ExportWriter::Plain(writer) => writer.flush(),
            ExportWriter::Gzip(writer) => writer.flush(),
        }
    }
}

/// Commit an export only after its writer succeeds and its bytes are durable.
/// Failures leave an existing destination untouched, including partial writes.
pub fn atomic_export(path: &Path, write: impl FnOnce(&mut dyn Write) -> Result<()>) -> Result<()> {
    let mut output = ExportFile::new(path, false)?;
    write(&mut output)?;
    output.finish()
}

pub fn write_atomic_export(path: &Path, bytes: &[u8]) -> Result<()> {
    atomic_export(path, |output| output.write_all(bytes).map_err(io_error))
}

struct ExportReader<'a> {
    engine: &'a DatabaseEngine,
    snapshot: Option<SqlTransaction>,
    structures: Vec<(TableRef, TableStructure)>,
}
impl<'a> ExportReader<'a> {
    async fn new(engine: &'a DatabaseEngine, tables: &[TableRef]) -> Result<Self> {
        let mut structures = Vec::new();
        for table in tables {
            structures.push((table.clone(), engine.table_structure(table).await?));
        }
        let transactional = if engine.kind() == DatabaseKind::MySQL {
            let mut all_transactional = true;
            for table in tables {
                all_transactional &= mysql_table_transactional(engine, table).await?;
            }
            all_transactional
        } else {
            true
        };
        let snapshot = if transactional && matches!(engine, DatabaseEngine::Sql(_)) {
            Some(SqlTransaction::begin(engine, true).await?)
        } else {
            None
        };
        Ok(Self {
            engine,
            snapshot,
            structures,
        })
    }
    fn structure(&self, table: &TableRef) -> TableStructure {
        self.structures
            .iter()
            .find(|(source, _)| source == table)
            .unwrap()
            .1
            .clone()
    }
    async fn page(
        &mut self,
        table: &TableRef,
        columns: &[ColumnInfo],
        offset: u64,
    ) -> Result<crate::QueryResult> {
        let kind = self.engine.kind();
        let mut order: Vec<crate::Order> = columns
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| crate::Order {
                column: column.name.clone(),
                direction: crate::OrderDirection::Ascending,
            })
            .collect();
        let has_primary_key = !order.is_empty();
        if !has_primary_key {
            order = columns
                .iter()
                .map(|column| crate::Order {
                    column: column.name.clone(),
                    direction: crate::OrderDirection::Ascending,
                })
                .collect();
        }
        let names = columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let page = Some(Page {
            limit: EXPORT_PAGE_SIZE as u32,
            offset,
        });
        if let Some(snapshot) = &mut self.snapshot {
            let mut statement =
                crate::build_select_with_columns(kind, table, &names, &[], &order, page, columns)?;
            if !has_primary_key && kind.dialect() == DatabaseKind::PostgreSQL {
                for column in columns {
                    let identifier = quote_identifier(kind, &column.name)?;
                    statement.sql = statement.sql.replace(
                        &format!("{identifier} ASC"),
                        &format!("CAST({identifier} AS text) ASC"),
                    );
                }
            }
            snapshot.query(&statement).await
        } else {
            self.engine
                .query_table_with_columns(
                    table,
                    &names,
                    &[],
                    &order,
                    page,
                    QueryOptions { max_rows: None },
                    Some(columns),
                )
                .await
        }
    }
    async fn finish(mut self) -> Result<()> {
        if let Some(snapshot) = self.snapshot.take() {
            snapshot.commit().await?;
        }
        Ok(())
    }
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
    let mut reader = ExportReader::new(engine, std::slice::from_ref(table)).await?;
    let summary = export_table_with_reader(&mut reader, table, path).await?;
    reader.finish().await?;
    Ok(summary)
}

async fn export_table_with_reader(
    reader: &mut ExportReader<'_>,
    table: &TableRef,
    path: &Path,
) -> Result<ExportSummary> {
    let engine = reader.engine;
    let kind = engine.kind();
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "export_table".to_owned(),
            kind,
        });
    }
    let file_format = detect_file_format(path)?;
    if matches!(kind, DatabaseKind::ClickHouse | DatabaseKind::SqlServer)
        && file_format.format == DumpFormat::Sql
    {
        return Err(DbxError::Unsupported {
            operation: format!("{kind} SQL dump export; use CSV or TSV"),
            kind,
        });
    }
    let columns = reader.structure(table).columns;
    let column_names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();

    let mut output = ExportFile::new(path, file_format.gzipped)?;
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
        let result = reader.page(table, &columns, offset).await?;
        for row in &result.rows {
            match file_format.format {
                DumpFormat::Sql => {
                    let statement = render_sql_insert(kind, table, &column_names, &row.values)?;
                    output.write_all(statement.as_bytes()).map_err(io_error)?;
                    output.write_all(b";\n").map_err(io_error)?;
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
        report_transfer(page_rows as u64, 0);
        offset += page_rows as u64;
        if page_rows < EXPORT_PAGE_SIZE {
            break;
        }
    }

    let gzipped = file_format.gzipped;
    output.finish()?;

    Ok(ExportSummary {
        rows_exported,
        format: file_format.format,
        gzipped,
        consistent_snapshot: reader.snapshot.is_some(),
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
    if matches!(kind, DatabaseKind::ClickHouse | DatabaseKind::SqlServer)
        && request.format == DumpFormat::Sql
    {
        return Err(DbxError::Unsupported {
            operation: format!("{kind} SQL dump export; use CSV or TSV"),
            kind,
        });
    }
    if request.schema_only && request.format != DumpFormat::Sql {
        return Err(DbxError::Parse(
            "schema-only exports require the SQL format".into(),
        ));
    }

    validate_output_directory(&request.output_directory)?;
    let stem = normalize_output_stem(&request.output_name)?;

    let schema_objects = if request.format == DumpFormat::Sql {
        engine.schema_objects().await?
    } else {
        Vec::new()
    };
    let schema_objects = schema_objects
        .into_iter()
        .filter(|object| {
            request
                .tables
                .iter()
                .any(|table| table.schema == object.schema || object.schema.is_none())
                && object
                    .table
                    .as_ref()
                    .is_none_or(|name| request.tables.iter().any(|table| table.name == *name))
        })
        .collect::<Vec<_>>();
    let mut view_definitions = Vec::new();
    if request.format == DumpFormat::Sql {
        for view in engine.list_tables().await?.iter().filter(|table| {
            table.kind == crate::EntityKind::View
                && request
                    .tables
                    .iter()
                    .any(|selected| selected.schema == table.schema)
        }) {
            let table = TableRef {
                schema: view.schema.clone(),
                name: view.name.clone(),
            };
            let structure = engine.table_structure(&table).await?;
            let definition = structure.definition.ok_or_else(|| {
                DbxError::Query(format!(
                    "Cannot export the definition of view {}",
                    view.name
                ))
            })?;
            let definition = if definition
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE ")
            {
                definition
            } else {
                format!("CREATE VIEW {} AS {definition}", quote_table(kind, &table)?)
            };
            view_definitions.push((table, definition));
        }
    }
    let view_definitions = order_export_views(engine, view_definitions, &request.tables).await?;
    let mut reader = ExportReader::new(engine, &request.tables).await?;
    let consistent_snapshot = reader.snapshot.is_some();
    if request.format == DumpFormat::Sql {
        // Snapshot every structure before writing any output. This lets the
        // dump emit a complete schema phase before it starts querying rows.
        let mut export_tables = Vec::with_capacity(request.tables.len());
        for table in &request.tables {
            export_tables.push(PreparedExportTable {
                table: table.clone(),
                structure: reader.structure(table),
            });
        }
        let table_order = table_export_order(&export_tables);
        let path =
            request
                .output_directory
                .join(with_extension(&stem, DumpFormat::Sql, request.gzipped));
        let mut output = ExportFile::new(&path, request.gzipped)?;
        output
            .write_all(b"-- DBX database dump\n")
            .map_err(io_error)?;
        output
            .write_all(
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
            )
            .map_err(io_error)?;
        append_database_prelude(kind, &request.tables, &table_order, &mut output)?;
        if kind == DatabaseKind::PostgreSQL {
            output
                .write_all(b"SET check_function_bodies = false;\n")
                .map_err(io_error)?;
        }
        for object in schema_objects.iter().filter(|object| {
            matches!(
                object.kind,
                crate::SchemaObjectKind::Sequence
                    | crate::SchemaObjectKind::Function
                    | crate::SchemaObjectKind::Procedure
            )
        }) {
            append_schema_object(kind, object, &mut output)?;
        }
        output.write_all(b"\n-- Schema\n").map_err(io_error)?;
        for &index in &table_order {
            let export_table = &export_tables[index];
            output
                .write_all(
                    format!(
                        "-- Table: {}\n",
                        comment_label(&quote_table(kind, &export_table.table)?)
                    )
                    .as_bytes(),
                )
                .map_err(io_error)?;
            let schema = if kind.dialect() == DatabaseKind::SQLite {
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
            output.write_all(schema.as_bytes()).map_err(io_error)?;
            output.write_all(b";\n").map_err(io_error)?;
            for statement in render_sql_indexes(kind, &export_table.table, &export_table.structure)?
            {
                output.write_all(statement.as_bytes()).map_err(io_error)?;
                if !statement.starts_with("--") {
                    output.write_all(b";").map_err(io_error)?;
                }
                output.write_all(b"\n").map_err(io_error)?;
            }
            output.write_all(b"\n").map_err(io_error)?;
        }

        let mut rows_exported = 0u64;
        if !request.schema_only {
            output.write_all(b"-- Data\n").map_err(io_error)?;
            for &index in &table_order {
                let export_table = &export_tables[index];
                rows_exported += append_sql_table_data(
                    &mut reader,
                    kind,
                    &export_table.table,
                    &export_table.structure.columns,
                    &mut output,
                )
                .await?;
                for statement in
                    render_sql_sequence_resets(kind, &export_table.table, &export_table.structure)?
                {
                    output.write_all(statement.as_bytes()).map_err(io_error)?;
                    output.write_all(b";\n").map_err(io_error)?;
                }
                output.write_all(b"\n").map_err(io_error)?;
            }
        }

        // SQLite cannot add a constraint with ALTER TABLE, so its selected
        // foreign keys remain in CREATE TABLE. PostgreSQL and MySQL can add
        // them after the data phase, which also handles cycles and arbitrary
        // selection order without disabling referential checks.
        if kind.dialect() != DatabaseKind::SQLite {
            output
                .write_all(b"-- Foreign-key constraints\n")
                .map_err(io_error)?;
            for &index in &table_order {
                append_sql_foreign_keys(
                    kind,
                    &export_tables[index].table,
                    &export_tables[index].structure,
                    &request.tables,
                    &mut output,
                )?;
            }
            output.write_all(b"\n").map_err(io_error)?;
        }

        // Install triggers after copying data so restore does not fire them.
        for object in schema_objects
            .iter()
            .filter(|object| object.kind == crate::SchemaObjectKind::Trigger)
        {
            append_schema_object(kind, object, &mut output)?;
        }
        for (_, definition) in view_definitions {
            output
                .write_all(format!("\n{definition};\n").as_bytes())
                .map_err(io_error)?;
        }
        reader.finish().await?;
        output.finish()?;
        return Ok(DatabaseExportSummary {
            tables_exported: request.tables.len() as u64,
            files_written: 1,
            rows_exported,
            format: request.format,
            gzipped: request.gzipped,
            schema_only: request.schema_only,
            consistent_snapshot,
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
        let summary = export_table_with_reader(&mut reader, table, &path).await?;
        rows_exported += summary.rows_exported;
    }

    reader.finish().await?;
    Ok(DatabaseExportSummary {
        tables_exported: request.tables.len() as u64,
        files_written: request.tables.len() as u64,
        rows_exported,
        format: request.format,
        gzipped: request.gzipped,
        schema_only: false,
        consistent_snapshot,
    })
}

fn append_database_prelude(
    kind: DatabaseKind,
    tables: &[TableRef],
    table_order: &[usize],
    output: &mut impl Write,
) -> Result<()> {
    if kind.dialect() == DatabaseKind::PostgreSQL {
        let schemas: BTreeSet<&str> = tables
            .iter()
            .filter_map(|table| table.schema.as_deref())
            .collect();
        if !schemas.is_empty() {
            output.write_all(b"\n-- Schemas\n").map_err(io_error)?;
            for schema in schemas {
                output
                    .write_all(
                        format!(
                            "CREATE SCHEMA IF NOT EXISTS {};\n",
                            quote_identifier(kind, schema)?
                        )
                        .as_bytes(),
                    )
                    .map_err(io_error)?;
            }
        }
    }

    output
        .write_all(b"\n-- Replace existing tables\n")
        .map_err(io_error)?;
    let reverse_order: Vec<&TableRef> = table_order
        .iter()
        .rev()
        .map(|index| &tables[*index])
        .collect();
    if kind.dialect() == DatabaseKind::SQLite {
        for table in reverse_order {
            output
                .write_all(
                    format!("DROP TABLE IF EXISTS {};\n", quote_table(kind, table)?).as_bytes(),
                )
                .map_err(io_error)?;
        }
    } else {
        let quoted = reverse_order
            .into_iter()
            .map(|table| quote_table(kind, table))
            .collect::<Result<Vec<_>>>()?;
        output
            .write_all(format!("DROP TABLE IF EXISTS {};\n", quoted.join(", ")).as_bytes())
            .map_err(io_error)?;
    }
    Ok(())
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
/// The statement includes columns, defaults, primary keys, check constraints,
/// and foreign keys. Indexes are separate statements from
/// [`render_sql_indexes`]. Generated expressions, triggers, and grants are not
/// part of [`TableStructure`].
pub fn render_sql_schema(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
    selected_tables: &[TableRef],
) -> Result<String> {
    render_sql_schema_with_foreign_keys(kind, table, structure, selected_tables, true)
}

pub(crate) fn render_sql_schema_without_foreign_keys(
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
        let (data_type, default) = match postgres_serial_type(kind, column) {
            Some(serial) => (serial.to_owned(), None),
            None => (
                safe_schema_type(&column.data_type)?,
                column.default_value.as_deref(),
            ),
        };
        let mut definition = format!("{} {}", quote_identifier(kind, &column.name)?, data_type);
        if let Some(default) = default {
            definition.push_str(" DEFAULT ");
            definition.push_str(safe_schema_expression(default)?);
        }
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

    for check in &structure.checks {
        let mut definition = String::new();
        if let Some(name) = &check.name {
            definition.push_str("CONSTRAINT ");
            definition.push_str(&quote_identifier(kind, name)?);
            definition.push(' ');
        }
        definition.push_str("CHECK (");
        definition.push_str(safe_schema_expression(&check.expression)?);
        definition.push(')');
        definitions.push(definition);
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

/// PostgreSQL `nextval(...)` defaults point at sequences owned by the column,
/// which a dump's `DROP TABLE` removes. Recreate them as serial types.
fn postgres_serial_type(kind: DatabaseKind, column: &ColumnInfo) -> Option<&'static str> {
    if kind != DatabaseKind::PostgreSQL
        || !column
            .default_value
            .as_deref()
            .is_some_and(|default| default.trim_start().starts_with("nextval("))
    {
        return None;
    }
    match column.data_type.trim().to_ascii_lowercase().as_str() {
        "integer" | "int" | "int4" => Some("serial"),
        "bigint" | "int8" => Some("bigserial"),
        "smallint" | "int2" => Some("smallserial"),
        _ => None,
    }
}

/// Statements that move recreated serial sequences past the imported rows.
pub(crate) fn render_sql_sequence_resets(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
) -> Result<Vec<String>> {
    let quoted_table = quote_table(kind, table)?;
    let literal_table = quoted_table.replace('\'', "''");
    structure
        .columns
        .iter()
        .filter(|column| postgres_serial_type(kind, column).is_some())
        .map(|column| {
            let column_name = quote_identifier(kind, &column.name)?;
            Ok(format!(
                "SELECT setval(pg_get_serial_sequence('{literal_table}', '{}'), COALESCE((SELECT MAX({column_name}) FROM {quoted_table}), 0) + 1, false)",
                column.name.replace('\'', "''")
            ))
        })
        .collect()
}

/// Render `CREATE INDEX` statements for a table's secondary indexes. Primary
/// keys are part of `CREATE TABLE`. MySQL functional indexes cannot be
/// reconstructed from its catalog and are emitted as comments.
pub fn render_sql_indexes(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
) -> Result<Vec<String>> {
    let mut statements = Vec::new();
    for index in structure.indexes.iter().filter(|index| !index.primary) {
        if let Some(definition) = index.definition.as_deref() {
            let definition = safe_schema_expression(definition)?;
            if kind.dialect() == DatabaseKind::MySQL {
                statements.push(definition.to_owned());
                continue;
            }
            let statement = ["CREATE UNIQUE INDEX ", "CREATE INDEX "]
                .into_iter()
                .find_map(|prefix| {
                    let rest = definition.strip_prefix(prefix)?;
                    let rest = rest.strip_prefix("IF NOT EXISTS ").unwrap_or(rest);
                    Some(format!("{prefix}IF NOT EXISTS {rest}"))
                })
                .ok_or_else(|| {
                    DbxError::Parse(format!("unexpected index definition for `{}`", index.name))
                })?;
            statements.push(statement);
            continue;
        }
        let mut parts = Vec::with_capacity(index.columns.len());
        for part in &index.columns {
            let (column, descending) = match part.strip_suffix(" DESC") {
                Some(column) => (column, true),
                None => (part.as_str(), false),
            };
            let (column, length) = match column
                .strip_suffix(')')
                .and_then(|rest| rest.rsplit_once('('))
                .filter(|(_, length)| {
                    !length.is_empty() && length.bytes().all(|byte| byte.is_ascii_digit())
                }) {
                Some((column, length)) => (column, Some(length)),
                None => (column, None),
            };
            if column == "(expression)" {
                parts.clear();
                break;
            }
            let mut rendered = quote_identifier(kind, column)?;
            if let Some(length) = length {
                rendered.push_str(&format!("({length})"));
            }
            if descending {
                rendered.push_str(" DESC");
            }
            parts.push(rendered);
        }
        if parts.is_empty() {
            statements.push(format!(
                "-- Index {} uses expressions; recreate it manually",
                comment_label(&index.name)
            ));
            continue;
        }
        // SQLite reserves `sqlite_` names for its automatic UNIQUE indexes.
        let name = match index.name.strip_prefix("sqlite_autoindex_") {
            Some(_) => format!(
                "{}_{}_key",
                table.name,
                index.columns.join("_").replace(" DESC", "")
            ),
            None => index.name.clone(),
        };
        let kind_prefix = match index.method.as_deref() {
            Some("FULLTEXT") if kind.dialect() == DatabaseKind::MySQL => "FULLTEXT ",
            Some("SPATIAL") if kind.dialect() == DatabaseKind::MySQL => "SPATIAL ",
            _ if index.unique => "UNIQUE ",
            _ => "",
        };
        // MySQL has no `IF NOT EXISTS` for indexes; the table is new anyway.
        let if_not_exists = if kind.dialect() == DatabaseKind::MySQL {
            ""
        } else {
            "IF NOT EXISTS "
        };
        statements.push(format!(
            "CREATE {kind_prefix}INDEX {if_not_exists}{} ON {} ({})",
            quote_identifier(kind, &name)?,
            quote_table(kind, table)?,
            parts.join(", ")
        ));
    }
    Ok(statements)
}

/// Accept a catalog-produced SQL expression only when it is one statement
/// fragment: balanced quotes and parentheses, with no terminator or comment
/// outside a literal.
pub(crate) fn safe_schema_expression(expression: &str) -> Result<&str> {
    let expression = expression.trim();
    let invalid = || DbxError::Parse(format!("invalid metadata expression `{expression}`"));
    if expression.is_empty() || expression.contains('\0') {
        return Err(invalid());
    }
    let mut quote = None;
    let mut depth = 0i32;
    let mut previous = '\0';
    for character in expression.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None => match character {
                '\'' | '"' | '`' => quote = Some(character),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return Err(invalid());
                    }
                }
                ';' => return Err(invalid()),
                '-' if previous == '-' => return Err(invalid()),
                '*' if previous == '/' => return Err(invalid()),
                _ => {}
            },
        }
        previous = character;
    }
    if quote.is_some() || depth != 0 {
        return Err(invalid());
    }
    Ok(expression)
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

pub(crate) fn append_sql_foreign_keys(
    kind: DatabaseKind,
    table: &TableRef,
    structure: &TableStructure,
    selected_tables: &[TableRef],
    output: &mut impl Write,
) -> Result<()> {
    for foreign_key in &structure.foreign_keys {
        let Some(definition) =
            render_sql_foreign_key_definition(kind, table, foreign_key, selected_tables)?
        else {
            continue;
        };
        output
            .write_all(
                format!(
                    "ALTER TABLE {} ADD {};\n",
                    quote_table(kind, table)?,
                    definition
                )
                .as_bytes(),
            )
            .map_err(io_error)?;
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
    reader: &mut ExportReader<'_>,
    kind: DatabaseKind,
    table: &TableRef,
    columns: &[ColumnInfo],
    output: &mut impl Write,
) -> Result<u64> {
    let column_names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();
    let mut rows_exported = 0u64;
    let mut offset = 0u64;
    loop {
        let result = reader.page(table, columns, offset).await?;
        for row in &result.rows {
            output
                .write_all(render_sql_insert(kind, table, &column_names, &row.values)?.as_bytes())
                .map_err(io_error)?;
            output.write_all(b";\n").map_err(io_error)?;
        }
        let page_rows = result.rows.len();
        rows_exported += page_rows as u64;
        report_transfer(page_rows as u64, 0);
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

/// Accept a catalog type spelling (`numeric(10,2)`, `int unsigned`,
/// `enum('a','b')`, `"Mixed Case"`) for a generated `CREATE TABLE`. Quoted
/// labels and identifiers may contain spaces and punctuation, but nothing
/// outside them may end the statement or open a comment.
pub(crate) fn safe_schema_type(data_type: &str) -> Result<String> {
    let data_type = data_type.trim();
    let invalid = || DbxError::Parse(format!("invalid metadata column type `{data_type}`"));
    if data_type.is_empty()
        || data_type.contains(';')
        || data_type.contains('\\')
        || data_type.contains('\0')
        || data_type.contains('\n')
        || data_type.contains('\r')
    {
        return Err(invalid());
    }
    let mut quote = None;
    for character in data_type.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None if matches!(character, '\'' | '"') => quote = Some(character),
            None if character.is_ascii_alphanumeric()
                || matches!(character, '_' | '(' | ')' | ',' | ' ' | '.' | '[' | ']') => {}
            None => return Err(invalid()),
        }
    }
    // A doubled quote (`''`) closes and reopens, so balance is all that
    // needs checking at the end.
    if quote.is_some() {
        return Err(invalid());
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

fn write_sql_dump_header(
    output: &mut impl Write,
    kind: DatabaseKind,
    table: &TableRef,
) -> Result<()> {
    let qualified = quote_table(kind, table)?;
    output.write_all(b"-- DBX table dump\n").map_err(io_error)?;
    output
        .write_all(format!("-- Source: {}\n", comment_label(&qualified)).as_bytes())
        .map_err(io_error)?;
    // DBX owns the transaction when importing this dump.
    Ok(())
}

fn comment_label(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
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

/// Render a staged row change as readable SQL for review. Values are shown as
/// literals; the applied statement binds them and also checks the row still
/// holds its original values.
pub fn render_row_change(kind: DatabaseKind, change: &RowChange) -> Result<String> {
    let value = |value: &MutationValue| match value {
        MutationValue::Parameter(value) => render_sql_literal(kind, value),
        MutationValue::Expression(expression) => Ok(expression.clone()),
    };
    let predicate = |filters: &[Filter]| -> Result<String> {
        let mut parts = Vec::with_capacity(filters.len());
        for filter in filters {
            let column = quote_identifier(kind, &filter.column)?;
            parts.push(match &filter.value {
                None | Some(CellValue::Null) => format!("{column} IS NULL"),
                Some(literal) => format!("{column} = {}", render_sql_literal(kind, literal)?),
            });
        }
        Ok(parts.join(" AND "))
    };
    Ok(match change {
        RowChange::Insert(request) => {
            let table = quote_table(kind, &request.table)?;
            if request.columns.is_empty() {
                format!("INSERT INTO {table} DEFAULT VALUES;")
            } else {
                let columns = request
                    .columns
                    .iter()
                    .map(|column| quote_identifier(kind, column))
                    .collect::<Result<Vec<_>>>()?;
                let values = request
                    .values
                    .iter()
                    .map(value)
                    .collect::<Result<Vec<_>>>()?;
                format!(
                    "INSERT INTO {table} ({}) VALUES ({});",
                    columns.join(", "),
                    values.join(", ")
                )
            }
        }
        RowChange::Update { request, .. } => {
            let assignments = request
                .assignments
                .iter()
                .map(|(column, assigned)| {
                    Ok(format!(
                        "{} = {}",
                        quote_identifier(kind, column)?,
                        value(assigned)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            format!(
                "UPDATE {} SET {} WHERE {};",
                quote_table(kind, &request.table)?,
                assignments.join(", "),
                predicate(&request.filters)?
            )
        }
        RowChange::Delete { table, filters, .. } => format!(
            "DELETE FROM {} WHERE {};",
            quote_table(kind, table)?,
            predicate(filters)?
        ),
    })
}

async fn order_export_views(
    engine: &DatabaseEngine,
    mut views: Vec<(TableRef, String)>,
    selected: &[TableRef],
) -> Result<Vec<(TableRef, String)>> {
    if views.is_empty() {
        return Ok(views);
    }
    let sql = match engine.kind() {
        DatabaseKind::PostgreSQL => {
            "SELECT vn.nspname, v.relname, rn.nspname, r.relname FROM pg_rewrite rw JOIN pg_class v ON v.oid=rw.ev_class JOIN pg_namespace vn ON vn.oid=v.relnamespace JOIN pg_depend d ON d.objid=rw.oid AND d.classid='pg_rewrite'::regclass AND d.refclassid='pg_class'::regclass JOIN pg_class r ON r.oid=d.refobjid JOIN pg_namespace rn ON rn.oid=r.relnamespace WHERE v.relkind='v' AND r.oid<>v.oid AND r.relkind IN ('r','p','v','m') AND vn.nspname NOT IN ('pg_catalog','information_schema')"
        }
        DatabaseKind::MySQL => {
            "SELECT VIEW_SCHEMA, VIEW_NAME, TABLE_SCHEMA, TABLE_NAME FROM information_schema.VIEW_TABLE_USAGE WHERE VIEW_SCHEMA=DATABASE()"
        }
        _ => return Ok(views),
    };
    let result = engine.query(sql, QueryOptions { max_rows: None }).await?;
    let mut dependencies: HashMap<TableRef, Vec<TableRef>> = HashMap::new();
    for row in result.rows {
        if row.values.len() != 4 {
            return Err(DbxError::Decode("Invalid view dependency metadata".into()));
        }
        dependencies
            .entry(TableRef::in_schema(
                row.values[0].to_string(),
                row.values[1].to_string(),
            ))
            .or_default()
            .push(TableRef::in_schema(
                row.values[2].to_string(),
                row.values[3].to_string(),
            ));
    }
    let included = views
        .iter()
        .map(|(view, _)| view.clone())
        .collect::<Vec<_>>();
    // A table-scoped dump cannot recreate views referring to omitted tables.
    // Full database selections retain every view with captured dependencies.
    loop {
        let previous = views.len();
        let current = views
            .iter()
            .map(|(view, _)| view.clone())
            .collect::<Vec<_>>();
        views.retain(|(view, _)| {
            dependencies.get(view).is_none_or(|references| {
                references.iter().all(|reference| {
                    selected.contains(reference)
                        || current.contains(reference)
                        || reference.schema.as_deref() == Some("pg_catalog")
                })
            })
        });
        if views.len() == previous {
            break;
        }
    }
    let mut ordered = Vec::new();
    while !views.is_empty() {
        let ready = views.iter().position(|(view, _)| {
            dependencies.get(view).is_none_or(|references| {
                references.iter().all(|reference| {
                    !included.contains(reference)
                        || ordered.iter().any(|(ready, _)| ready == reference)
                })
            })
        });
        let Some(index) = ready else {
            return Err(DbxError::Query(
                "View dependencies cannot be ordered; export the views separately".into(),
            ));
        };
        ordered.push(views.remove(index));
    }
    Ok(ordered)
}

fn append_schema_object(
    kind: DatabaseKind,
    object: &crate::SchemaObject,
    output: &mut ExportFile,
) -> Result<()> {
    let definition = object.definition.as_ref().ok_or_else(|| {
        DbxError::Query(format!("Cannot export the definition of {}", object.name))
    })?;
    if kind == DatabaseKind::MySQL {
        let delimiter = "__DBX_ROUTINE_END__";
        if definition.contains(delimiter) {
            return Err(DbxError::Parse(
                "Routine contains the dump delimiter".into(),
            ));
        }
        output
            .write_all(
                format!(
                    "\nDELIMITER {delimiter}\n{}{delimiter}\nDELIMITER ;\n",
                    definition.trim_end_matches(';')
                )
                .as_bytes(),
            )
            .map_err(io_error)?;
    } else {
        output
            .write_all(format!("\n{};\n", definition.trim_end_matches(';')).as_bytes())
            .map_err(io_error)?;
    }
    Ok(())
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
/// Files and gzip input stream one bounded record/statement at a time.
/// DBX owns the transaction; failed imports discard their connection.
pub async fn import_file(
    engine: &DatabaseEngine,
    target: Option<&TableRef>,
    path: &Path,
) -> Result<ImportReport> {
    let started = Instant::now();
    let kind = engine.kind();
    if kind == DatabaseKind::ClickHouse {
        return Err(DbxError::Unsupported {
            operation: "ClickHouse file import; use SQL or the native bulk loader".into(),
            kind,
        });
    }
    if !kind.is_sql() {
        return Err(DbxError::Unsupported {
            operation: "import_file".to_owned(),
            kind,
        });
    }
    let file_format = detect_file_format(path)?;
    let file = fs::File::open(path).map_err(io_error)?;
    let input: Box<dyn BufRead + Send> = if file_format.gzipped {
        Box::new(BufReader::new(GzDecoder::new(BufReader::new(file))))
    } else {
        Box::new(BufReader::new(file))
    };

    match file_format.format {
        DumpFormat::Sql => {
            if kind == DatabaseKind::MySQL {
                for table in engine
                    .list_tables()
                    .await?
                    .into_iter()
                    .filter(|table| table.kind == crate::EntityKind::Table)
                {
                    if !mysql_table_transactional(
                        engine,
                        &TableRef {
                            name: table.name,
                            schema: table.schema,
                        },
                    )
                    .await?
                    {
                        return Err(DbxError::Query("Atomic SQL imports require InnoDB storage for the active database tables".into()));
                    }
                }
            }
            let mut reader = SqlScriptReader::with_kind(input, Some(kind));
            let mut transaction = SqlTransaction::begin(engine, false).await?;
            let mut statements_executed = 0;
            while let Some(mut statement) = reader.next_statement().map_err(io_error)? {
                let function_check_setting = kind == DatabaseKind::PostgreSQL
                    && statement
                        .trim()
                        .eq_ignore_ascii_case("SET check_function_bodies = false");
                if function_check_setting {
                    statement = "SET LOCAL check_function_bodies = false".into();
                }
                let keyword = crate::sqlx_engine::top_level_operation_keyword(&statement)
                    .unwrap_or_default()
                    .to_ascii_uppercase();
                if matches!(
                    keyword.as_str(),
                    "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "SAVEPOINT" | "RELEASE"
                ) {
                    return Err(DbxError::Parse("dump contains transaction controls; remove them so DBX can own the import transaction".into()));
                }
                if !function_check_setting
                    && matches!(
                        keyword.as_str(),
                        "ATTACH" | "DETACH" | "VACUUM" | "PRAGMA" | "SET" | "RESET"
                    )
                {
                    return Err(DbxError::Parse("dump changes session or transaction settings; remove those statements so DBX can preserve atomic rollback".into()));
                }
                if engine.kind() == DatabaseKind::MySQL
                    && !matches!(
                        keyword.as_str(),
                        "INSERT" | "UPDATE" | "DELETE" | "REPLACE" | "SELECT"
                    )
                {
                    return Err(DbxError::Parse("MySQL dump contains statements that may implicitly commit. DBX accepts data-only dumps to preserve atomic rollback; apply schema changes separately in a query tab.".into()));
                }
                transaction
                    .query(&crate::SqlStatement::new(statement, Vec::new()))
                    .await?;
                statements_executed += 1;
                report_transfer(0, 1);
            }
            transaction.commit().await?;
            Ok(ImportReport {
                statements_executed,
                rows_inserted: 0,
                elapsed_ms: elapsed_ms_since(started),
            })
        }
        DumpFormat::Csv | DumpFormat::Tsv => {
            let target = target.ok_or_else(|| {
                DbxError::Parse("CSV and TSV imports require a target table".into())
            })?;
            import_delimited(engine, target, input, file_format.format)
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
    input: Box<dyn BufRead + Send>,
    format: DumpFormat,
) -> Result<ImportReport> {
    let kind = engine.kind();
    if kind == DatabaseKind::MySQL && !mysql_table_transactional(engine, target).await? {
        return Err(DbxError::Query("Atomic imports require an InnoDB table; this table uses a nontransactional storage engine".into()));
    }
    let columns = engine.describe_table(target).await?;
    let column_names: Vec<String> = columns.iter().map(|column| column.name.clone()).collect();

    let delimiter = format.delimiter().unwrap_or(b',');
    let mut reader = DelimitedStream::new(input, delimiter);
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

    let mut transaction = SqlTransaction::begin(engine, false).await?;
    let mut rows_inserted = 0u64;
    let mut pending: Vec<Vec<CellValue>> = Vec::with_capacity(batch_limit);
    let mut pending_bytes = 0usize;
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
        pending_bytes =
            pending_bytes.saturating_add(record.iter().flatten().map(String::len).sum::<usize>());
        if pending.len() >= batch_limit || pending_bytes >= 4 * 1024 * 1024 {
            flush_batch(&mut transaction, kind, target, &mapped, &pending, &columns).await?;
            rows_inserted += pending.len() as u64;
            report_transfer(pending.len() as u64, 0);
            pending.clear();
            pending_bytes = 0;
        }
    }
    if !pending.is_empty() {
        flush_batch(&mut transaction, kind, target, &mapped, &pending, &columns).await?;
        rows_inserted += pending.len() as u64;
        report_transfer(pending.len() as u64, 0);
    }

    transaction.commit().await?;
    Ok(ImportReport {
        statements_executed: 0,
        rows_inserted,
        elapsed_ms: 0,
    })
}

pub(crate) async fn mysql_table_transactional(
    engine: &DatabaseEngine,
    table: &TableRef,
) -> Result<bool> {
    let sql = if table.schema.is_some() {
        "SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=? AND TABLE_NAME=?"
    } else {
        "SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME=?"
    };
    let mut params = Vec::new();
    if let Some(schema) = &table.schema {
        params.push(CellValue::Text(schema.clone()));
    }
    params.push(CellValue::Text(table.name.clone()));
    let result = engine
        .query_statement(
            &crate::SqlStatement::new(sql, params),
            QueryOptions::default(),
        )
        .await?;
    Ok(result.rows.first().and_then(|row| row.values.first()).is_some_and(|value| matches!(value, CellValue::Text(engine) if engine.eq_ignore_ascii_case("InnoDB"))))
}

async fn flush_batch(
    transaction: &mut SqlTransaction,
    kind: DatabaseKind,
    target: &TableRef,
    columns: &[(usize, String)],
    rows: &[Vec<CellValue>],
    metadata: &[ColumnInfo],
) -> Result<()> {
    let names: Vec<String> = columns.iter().map(|(_, name)| name.clone()).collect();
    let statement = build_multi_row_insert_with_columns(kind, target, &names, rows, metadata)?;
    transaction.query(&statement).await?;
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
    match kind.dialect() {
        // Conservative bound for SQLite's historical 999-parameter default.
        DatabaseKind::SQLite => 900,
        DatabaseKind::PostgreSQL => 60_000,
        DatabaseKind::MySQL => 65_000,
        DatabaseKind::DuckDB | DatabaseKind::BigQuery => 1000,
        _ => 0,
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

/// Retain one complete CSV/TSV record, preserving quoted NULL/empty semantics.
struct DelimitedStream<R> {
    input: R,
    delimiter: u8,
    first: bool,
}
impl<R: BufRead> DelimitedStream<R> {
    fn new(input: R, delimiter: u8) -> Self {
        Self {
            input,
            delimiter,
            first: true,
        }
    }
    fn next_record(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
        loop {
            let mut record = Vec::new();
            let mut state = FieldState::Start;
            loop {
                let mut byte = [0];
                if self.input.read(&mut byte)? == 0 {
                    break;
                }
                let byte = byte[0];
                record.push(byte);
                if self.first && record == [0xEF, 0xBB, 0xBF] {
                    record.clear();
                    state = FieldState::Start;
                    self.first = false;
                    continue;
                }
                if record.len() >= 3 || byte == b'\n' || byte == b'\r' {
                    self.first = false;
                }
                if record.len() > MAX_TRANSFER_RECORD_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "CSV/TSV record exceeds the 64 MiB transfer budget",
                    ));
                }
                if (byte == b'\n' || byte == b'\r') && state != FieldState::Quoted {
                    break;
                }
                state = match (state, byte) {
                    (FieldState::Start, b'"') => FieldState::Quoted,
                    (FieldState::Quoted, b'"') => FieldState::QuoteClosed,
                    (FieldState::QuoteClosed, b'"') => FieldState::Quoted,
                    (FieldState::Quoted, _) => FieldState::Quoted,
                    (_, byte) if byte == self.delimiter => FieldState::Start,
                    (FieldState::Start, _) => FieldState::Unquoted,
                    (state, _) => state,
                };
            }
            if record.is_empty() {
                return Ok(None);
            }
            std::str::from_utf8(&record)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if let Some(fields) = (DelimitedReader {
                input: &record,
                position: 0,
                delimiter: self.delimiter,
            })
            .next_record()?
            {
                return Ok(Some(fields));
            }
        }
    }
}

impl<'a> DelimitedReader<'a> {
    pub fn new(input: &'a [u8], delimiter: u8) -> Self {
        Self {
            input: input.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(input),
            position: 0,
            delimiter,
        }
    }

    /// Return the next record, or `None` at end of input.
    pub fn next_record(&mut self) -> io::Result<Option<Vec<Option<String>>>> {
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
    output: &mut impl Write,
    delimiter: u8,
    fields: &[Option<&str>],
) -> Result<()> {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            output.write_all(&[delimiter]).map_err(io_error)?;
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
            output.write_all(b"\"").map_err(io_error)?;
            for character in field.chars() {
                if character == '"' {
                    output.write_all(b"\"\"").map_err(io_error)?;
                } else {
                    let mut buffer = [0u8; 4];
                    output
                        .write_all(character.encode_utf8(&mut buffer).as_bytes())
                        .map_err(io_error)?;
                }
            }
            output.write_all(b"\"").map_err(io_error)?;
        } else {
            output.write_all(field.as_bytes()).map_err(io_error)?;
        }
    }
    output.write_all(b"\n").map_err(io_error)?;
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
// Compression helpers
// ---------------------------------------------------------------------------

fn io_error(error: io::Error) -> DbxError {
    DbxError::Io(error.to_string())
}

fn elapsed_ms_since(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::split_sql_statements;

    #[test]
    fn dump_header_keeps_identifier_line_breaks_inside_comments() {
        let mut output = Vec::new();
        write_sql_dump_header(
            &mut output,
            DatabaseKind::SQLite,
            &TableRef::new("odd\nSELECT 1;\rtable"),
        )
        .unwrap();
        let header = String::from_utf8(output).unwrap();
        assert!(split_sql_statements(&header).is_empty());
        assert_eq!(header.lines().count(), 2);
    }

    #[tokio::test]
    async fn export_pages_keep_one_snapshot_during_concurrent_writes() {
        let directory = tempfile::tempdir().unwrap();
        let engine = DatabaseEngine::connect(crate::ConnectionConfig::new(
            DatabaseKind::SQLite,
            format!(
                "sqlite://{}?mode=rwc",
                directory.path().join("snapshot.sqlite").display()
            ),
        ))
        .await
        .unwrap();
        engine.execute_sql("PRAGMA journal_mode=WAL").await.unwrap();
        engine
            .execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY)")
            .await
            .unwrap();
        engine.execute_sql("WITH RECURSIVE n(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM n WHERE id<1001) INSERT INTO items SELECT id FROM n").await.unwrap();
        let table = TableRef::new("items");
        let mut reader = ExportReader::new(&engine, std::slice::from_ref(&table))
            .await
            .unwrap();
        let columns = reader.structure(&table).columns;
        assert_eq!(
            reader.page(&table, &columns, 0).await.unwrap().rows.len(),
            1000
        );
        engine
            .execute_sql("DELETE FROM items WHERE id=1001; INSERT INTO items VALUES(1002)")
            .await
            .unwrap();
        let second = reader.page(&table, &columns, 1000).await.unwrap();
        assert_eq!(second.rows.len(), 1);
        assert_eq!(second.rows[0].values[0], CellValue::Integer(1001));
        reader.finish().await.unwrap();
        let current = engine
            .query("SELECT max(id) FROM items", QueryOptions::default())
            .await
            .unwrap();
        assert_eq!(current.rows[0].values[0], CellValue::Integer(1002));
    }

    #[test]
    fn schema_types_allow_quoted_labels_but_nothing_that_escapes_them() {
        for valid in [
            "numeric(10,2)",
            "int unsigned",
            "integer[]",
            "enum('a b','it''s','x--y')",
            "\"Mixed Case\"",
        ] {
            assert_eq!(safe_schema_type(valid).unwrap(), valid);
        }
        for invalid in [
            "enum('open",
            "int -- comment",
            "int /* c */",
            "int; DROP TABLE t",
            "enum('a');",
            "text\nNOT NULL",
        ] {
            assert!(safe_schema_type(invalid).is_err(), "{invalid}");
        }
    }

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
    fn streamed_records_preserve_multiline_bom_headers_and_data() {
        let input = "\u{feff}\"multi\nline\",value\n\u{feff}data,\"\"\n";
        let mut reader = DelimitedStream::new(BufReader::with_capacity(1, input.as_bytes()), b',');
        assert_eq!(
            reader.next_record().unwrap().unwrap(),
            vec![Some("multi\nline".into()), Some("value".into())]
        );
        assert_eq!(
            reader.next_record().unwrap().unwrap(),
            vec![Some("\u{feff}data".into()), Some("".into())]
        );
        assert!(reader.next_record().unwrap().is_none());
    }
    #[test]
    fn hash_is_a_comment_only_where_the_dialect_says_so() {
        let script = "SELECT doc #>> '{a,b}', 5 # 3 FROM t; SELECT 2";
        for kind in [
            DatabaseKind::PostgreSQL,
            DatabaseKind::CockroachDB,
            DatabaseKind::SQLite,
            DatabaseKind::DuckDB,
        ] {
            assert_eq!(
                checked_split_sql_for(Some(kind), script).unwrap(),
                ["SELECT doc #>> '{a,b}', 5 # 3 FROM t", "SELECT 2"],
                "{kind:?}"
            );
        }
        for kind in [DatabaseKind::MySQL, DatabaseKind::BigQuery] {
            assert_eq!(
                checked_split_sql_for(Some(kind), "SELECT 1; # a;b\nSELECT 2").unwrap(),
                ["SELECT 1", "SELECT 2"],
                "{kind:?}"
            );
        }
    }
    #[test]
    fn dialect_splitting_preserves_backslashes_hints_and_token_boundaries() {
        let pg = checked_split_sql_for(Some(DatabaseKind::PostgreSQL), "SELECT 'C:\\'; SELECT 2")
            .unwrap();
        assert_eq!(pg.len(), 2);
        let sqlite =
            checked_split_sql_for(Some(DatabaseKind::SQLite), "SELECT 'C:\\'; SELECT 2").unwrap();
        assert_eq!(sqlite.len(), 2);
        let escaped = checked_split_sql_for(
            Some(DatabaseKind::PostgreSQL),
            "SELECT E'it\\'s;here'; SELECT 2",
        )
        .unwrap();
        assert_eq!(escaped.len(), 2);
        let hint = checked_split_sql_for(
            Some(DatabaseKind::MySQL),
            "SELECT /*+ MAX_EXECUTION_TIME(1000) */ 1",
        )
        .unwrap();
        assert!(hint[0].contains("/*+ MAX_EXECUTION_TIME(1000) */"));
        assert!(
            checked_split_sql_for(Some(DatabaseKind::MySQL), "/*!50000 DELETE FROM items */")
                .is_err()
        );
        assert_eq!(
            split_sql_statements("SELECT/* comment */1"),
            vec!["SELECT 1"]
        );
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
                    default_value: None,
                },
                ColumnInfo {
                    name: "account_id".into(),
                    data_type: "integer".into(),
                    enum_values: Vec::new(),
                    nullable: false,
                    ordinal: 2,
                    primary_key: false,
                    default_value: None,
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
            ..Default::default()
        };

        let sql =
            render_sql_schema(DatabaseKind::PostgreSQL, &child, &structure, &[parent]).unwrap();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"public\".\"events\""));
        assert!(sql.contains("PRIMARY KEY (\"id\")"));
        assert!(sql.contains("CONSTRAINT \"events_account_id_fkey\""));
        assert!(sql.contains("ON UPDATE CASCADE ON DELETE SET NULL"));
    }

    #[test]
    fn schema_expressions_reject_terminators_and_comments_outside_literals() {
        assert_eq!(
            safe_schema_expression(" 'a;b'::text ").unwrap(),
            "'a;b'::text"
        );
        assert!(safe_schema_expression("now()").is_ok());
        assert!(safe_schema_expression("-1").is_ok());
        for bad in [
            "1); DROP TABLE x",
            "1 -- x",
            "1 /* x */",
            "(1",
            "'open",
            "1)",
        ] {
            assert!(safe_schema_expression(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn schema_rendering_includes_defaults_checks_serials_and_indexes() {
        let table = TableRef::in_schema("public", "items");
        let column = |name: &str, data_type: &str, default: Option<&str>| ColumnInfo {
            name: name.into(),
            data_type: data_type.into(),
            enum_values: Vec::new(),
            nullable: false,
            ordinal: 1,
            primary_key: name == "id",
            default_value: default.map(str::to_owned),
        };
        let structure = TableStructure {
            columns: vec![
                column("id", "integer", Some("nextval('items_id_seq'::regclass)")),
                column("code", "text", Some("'new'::text")),
            ],
            checks: vec![crate::CheckConstraintInfo {
                name: Some("code_length".into()),
                expression: "length(code) > 0".into(),
            }],
            indexes: vec![
                crate::IndexInfo {
                    name: "items_pkey".into(),
                    columns: vec!["id".into()],
                    unique: true,
                    primary: true,
                    method: None,
                    predicate: None,
                    definition: Some(
                        "CREATE UNIQUE INDEX items_pkey ON public.items USING btree (id)".into(),
                    ),
                },
                crate::IndexInfo {
                    name: "items_code".into(),
                    columns: vec!["code".into()],
                    unique: true,
                    primary: false,
                    method: Some("btree".into()),
                    predicate: None,
                    definition: Some(
                        "CREATE UNIQUE INDEX items_code ON public.items USING btree (code)".into(),
                    ),
                },
            ],
            ..Default::default()
        };
        let sql = render_sql_schema(DatabaseKind::PostgreSQL, &table, &structure, &[]).unwrap();
        assert!(sql.contains("\"id\" serial NOT NULL"), "{sql}");
        assert!(
            sql.contains("\"code\" text DEFAULT 'new'::text NOT NULL"),
            "{sql}"
        );
        assert!(
            sql.contains("CONSTRAINT \"code_length\" CHECK (length(code) > 0)"),
            "{sql}"
        );
        assert_eq!(
            render_sql_indexes(DatabaseKind::PostgreSQL, &table, &structure).unwrap(),
            ["CREATE UNIQUE INDEX IF NOT EXISTS items_code ON public.items USING btree (code)"]
        );
        assert_eq!(
            render_sql_sequence_resets(DatabaseKind::PostgreSQL, &table, &structure).unwrap(),
            [
                "SELECT setval(pg_get_serial_sequence('\"public\".\"items\"', 'id'), COALESCE((SELECT MAX(\"id\") FROM \"public\".\"items\"), 0) + 1, false)"
            ]
        );

        let mysql = TableStructure {
            indexes: vec![
                crate::IndexInfo {
                    name: "by_name".into(),
                    columns: vec!["name(10)".into(), "created DESC".into()],
                    unique: false,
                    primary: false,
                    method: Some("BTREE".into()),
                    predicate: None,
                    definition: None,
                },
                crate::IndexInfo {
                    name: "by_expression".into(),
                    columns: vec!["(expression)".into()],
                    unique: false,
                    primary: false,
                    method: Some("BTREE".into()),
                    predicate: None,
                    definition: None,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            render_sql_indexes(DatabaseKind::MySQL, &TableRef::new("people"), &mysql).unwrap(),
            [
                "CREATE INDEX `by_name` ON `people` (`name`(10), `created` DESC)",
                "-- Index by_expression uses expressions; recreate it manually",
            ]
        );
    }

    #[tokio::test]
    async fn sqlite_dump_round_trips_defaults_and_indexes() {
        let connect = || {
            DatabaseEngine::connect(crate::ConnectionConfig::new(
                crate::DatabaseKind::SQLite,
                "sqlite::memory:",
            ))
        };
        let source = connect().await.unwrap();
        source
            .execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, code TEXT NOT NULL UNIQUE, qty INTEGER NOT NULL DEFAULT 1 CHECK (qty >= 0))")
            .await
            .unwrap();
        source
            .execute_sql("CREATE INDEX items_qty ON items (qty DESC)")
            .await
            .unwrap();
        source
            .execute_sql("INSERT INTO items (id, code) VALUES (1, 'a')")
            .await
            .unwrap();
        source.execute_sql("CREATE TRIGGER items_insert AFTER INSERT ON items BEGIN UPDATE items SET qty=qty+1 WHERE id=NEW.id; END").await.unwrap();
        source
            .execute_sql("CREATE VIEW items_view AS SELECT code, qty FROM items")
            .await
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let request = DatabaseExportRequest {
            tables: vec![TableRef::new("items")],
            output_directory: directory.path().to_owned(),
            output_name: "items.sql".into(),
            format: DumpFormat::Sql,
            schema_only: false,
            gzipped: false,
        };
        export_database(&source, &request).await.unwrap();

        let target = connect().await.unwrap();
        import_database(&target, &directory.path().join("items.sql"))
            .await
            .unwrap();
        let structure = target
            .table_structure(&TableRef::new("items"))
            .await
            .unwrap();
        assert_eq!(structure.checks.len(), 1);
        assert!(
            target
                .execute_sql("INSERT INTO items(id, code, qty) VALUES (2,'invalid',-1)")
                .await
                .is_err()
        );
        target
            .execute_sql("INSERT INTO items(id, code) VALUES (2,'trigger')")
            .await
            .unwrap();
        let view = target
            .query(
                "SELECT qty FROM items_view WHERE code='trigger'",
                QueryOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(view.rows[0].values[0], CellValue::Integer(2));
        assert!(
            target
                .schema_objects()
                .await
                .unwrap()
                .iter()
                .any(|object| object.name == "items_insert")
        );
        let qty = structure
            .columns
            .iter()
            .find(|column| column.name == "qty")
            .unwrap();
        assert_eq!(qty.default_value.as_deref(), Some("1"));
        assert!(
            structure
                .indexes
                .iter()
                .any(|index| index.name == "items_qty" && index.columns == ["qty DESC"])
        );
        assert!(
            structure
                .indexes
                .iter()
                .any(|index| index.unique && index.columns == ["code"])
        );
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
                    default_value: None,
                },
                ColumnInfo {
                    name: "account_id".into(),
                    data_type: "integer".into(),
                    enum_values: Vec::new(),
                    nullable: false,
                    ordinal: 2,
                    primary_key: false,
                    default_value: None,
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
            ..Default::default()
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
        let child_drop = dump
            .find("DROP TABLE IF EXISTS \"dbx_transfer_child\"")
            .unwrap();
        let parent_drop = dump
            .find("DROP TABLE IF EXISTS \"dbx_transfer_parent\"")
            .unwrap();
        assert!(child_drop < parent_drop);
        assert!(parent_drop < parent_create);
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
        restored
            .execute_sql(
                "CREATE TABLE dbx_transfer_parent (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            )
            .await
            .unwrap();
        restored
            .execute_sql(
                "CREATE TABLE dbx_transfer_child (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL, FOREIGN KEY (parent_id) REFERENCES dbx_transfer_parent (id))",
            )
            .await
            .unwrap();
        restored
            .execute_sql("INSERT INTO dbx_transfer_parent (id, name) VALUES (99, 'stale')")
            .await
            .unwrap();
        restored
            .execute_sql("INSERT INTO dbx_transfer_child (id, parent_id) VALUES (99, 99)")
            .await
            .unwrap();
        let report = import_database(&restored, &path).await.unwrap();
        assert_eq!(report.statements_executed, 6);
        let violations = restored
            .query("PRAGMA foreign_key_check", QueryOptions { max_rows: None })
            .await
            .unwrap();
        assert!(violations.rows.is_empty());
        let restored_rows = restored
            .query(
                "SELECT id FROM dbx_transfer_parent ORDER BY id",
                QueryOptions { max_rows: None },
            )
            .await
            .unwrap();
        assert_eq!(restored_rows.rows.len(), 1);
        assert_eq!(restored_rows.rows[0].values[0], CellValue::Integer(1));
    }

    #[tokio::test]
    async fn sql_import_rolls_back_every_statement_when_one_fails() {
        let engine = DatabaseEngine::connect(crate::ConnectionConfig::new(
            crate::DatabaseKind::SQLite,
            "sqlite::memory:",
        ))
        .await
        .unwrap();
        engine
            .execute_sql("CREATE TABLE dbx_atomic (id INTEGER PRIMARY KEY)")
            .await
            .unwrap();
        engine
            .execute_sql("INSERT INTO dbx_atomic (id) VALUES (1)")
            .await
            .unwrap();

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("atomic.sql");
        fs::write(
            &path,
            "DELETE FROM dbx_atomic; INSERT INTO dbx_atomic (id) VALUES (2); INSERT INTO missing_table (id) VALUES (3);",
        )
        .unwrap();

        assert!(import_database(&engine, &path).await.is_err());
        let rows = engine
            .query("SELECT id FROM dbx_atomic", QueryOptions { max_rows: None })
            .await
            .unwrap();
        assert_eq!(rows.rows.len(), 1);
        assert_eq!(rows.rows[0].values[0], CellValue::Integer(1));
    }

    #[test]
    fn postgres_database_prelude_creates_unique_schemas_before_replacing_tables() {
        let tables = vec![
            TableRef::in_schema("tenant", "events"),
            TableRef::in_schema("tenant", "accounts"),
        ];
        let mut output = Vec::new();
        append_database_prelude(DatabaseKind::PostgreSQL, &tables, &[1, 0], &mut output).unwrap();
        let sql = String::from_utf8(output).unwrap();

        assert_eq!(
            sql.matches("CREATE SCHEMA IF NOT EXISTS \"tenant\"")
                .count(),
            1
        );
        let create_schema = sql.find("CREATE SCHEMA").unwrap();
        let drop_tables = sql.find("DROP TABLE").unwrap();
        assert!(create_schema < drop_tables);
        assert!(
            sql.contains("DROP TABLE IF EXISTS \"tenant\".\"events\", \"tenant\".\"accounts\"")
        );
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
