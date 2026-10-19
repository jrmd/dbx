//! Typed loaded-result exports and atomic, bounded-memory full read exports.
use dbx_core::{CellValue, QueryResult};
use rust_xlsxwriter::{Format, Formula, Workbook, Worksheet};

fn plain(value: &CellValue) -> String {
    match value {
        CellValue::Null => "NULL".into(),
        CellValue::Boolean(value) => value.to_string(),
        CellValue::Integer(value) => value.to_string(),
        CellValue::Unsigned(value) => value.to_string(),
        CellValue::Real(value) => value.to_string(),
        CellValue::Text(value) => value.clone(),
        CellValue::Bytes(value) => {
            let mut output = String::from("0x");
            for byte in value {
                use std::fmt::Write;
                let _ = write!(output, "{byte:02x}");
            }
            output
        }
        CellValue::Json(value) => value.to_string(),
    }
}

fn markdown_cell(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "&#92;")
        .replace('|', "&#124;")
        .replace('`', "&#96;")
        .replace('[', "&#91;")
        .replace(']', "&#93;")
        .replace('*', "&#42;")
        .replace('_', "&#95;")
        .replace('~', "&#126;")
        .replace('\r', "")
        .replace('\n', "<br>")
}

pub fn markdown(result: &QueryResult) -> String {
    let mut output = String::new();
    let mut line = |cells: Vec<String>| {
        output.push_str("| ");
        output.push_str(&cells.join(" | "));
        output.push_str(" |\n");
    };
    line(
        result
            .columns
            .iter()
            .map(|column| markdown_cell(&column.name))
            .collect(),
    );
    line(result.columns.iter().map(|_| "---".into()).collect());
    for row in &result.rows {
        line(
            (0..result.columns.len())
                .map(|column| {
                    markdown_cell(
                        &row.values
                            .get(column)
                            .map(plain)
                            .unwrap_or_else(|| "NULL".into()),
                    )
                })
                .collect(),
        );
    }
    output
}

fn write_cell(
    sheet: &mut Worksheet,
    row: u32,
    column: u16,
    value: &CellValue,
) -> Result<(), String> {
    let write = match value {
        CellValue::Null => return Ok(()),
        // XLSX has no literal empty string in this writer. A fixed, data-independent
        // expression preserves empty text while leaving SQL NULL genuinely blank.
        CellValue::Text(value) if value.is_empty() => {
            sheet.write_formula(row, column, Formula::new("=\"\"").set_result(""))
        }
        CellValue::Boolean(value) => sheet.write_boolean(row, column, *value),
        CellValue::Integer(value) if value.unsigned_abs() <= 999_999_999_999_999 => {
            sheet.write_number(row, column, *value as f64)
        }
        CellValue::Unsigned(value) if *value <= 999_999_999_999_999 => {
            sheet.write_number(row, column, *value as f64)
        }
        CellValue::Real(value) if value.is_finite() => sheet.write_number(row, column, *value),
        value => sheet.write_string(row, column, plain(value)),
    };
    write.map_err(|error| format!("Row {}, column {}: {error}", row + 1, column + 1))?;
    Ok(())
}

/// The explicit string methods ensure formula-looking values and URLs remain text.
/// Integers beyond Excel's 15 significant digits are text to retain every digit.
pub fn xlsx(result: &QueryResult) -> Result<Vec<u8>, String> {
    if result.columns.is_empty() {
        return Err("No columns to export".into());
    }
    if result.columns.len() > 16_384 || result.rows.len() >= 1_048_576 {
        return Err("Loaded result exceeds Excel's row or column limit".into());
    }
    let mut workbook = Workbook::new();
    let header = Format::new().set_bold();
    let sheet = workbook.add_worksheet();
    sheet
        .set_name("Loaded rows")
        .map_err(|error| error.to_string())?;
    for (column, info) in result.columns.iter().enumerate() {
        sheet
            .write_string_with_format(0, column as u16, &info.name, &header)
            .map_err(|error| error.to_string())?;
    }
    for (row, record) in result.rows.iter().enumerate() {
        for (column, value) in record.values.iter().take(result.columns.len()).enumerate() {
            let (row, column) = ((row + 1) as u32, column as u16);
            write_cell(sheet, row, column, value)?;
        }
    }
    sheet
        .set_freeze_panes(1, 0)
        .map_err(|error| error.to_string())?;
    let notes = workbook.add_worksheet();
    notes
        .set_name("Column types")
        .map_err(|error| error.to_string())?;
    for (row, text) in [
        format!("{} loaded rows only; staged edits are excluded.{}", result.rows.len(), if result.truncated { " The query result was truncated." } else { "" }),
        "SQL NULL is blank. Text stays text, including formula prefixes and URLs. Empty text uses the constant formula =\"\" to remain distinct from SQL NULL.".into(),
        "Integers exceeding 15 digits and non-finite numbers are text to preserve their exact representation. Bytes are hexadecimal; JSON is text.".into(),
    ].iter().enumerate() { notes.write_string(row as u32, 0, text).map_err(|error| error.to_string())?; }
    notes
        .write_string(4, 0, "Column")
        .map_err(|error| error.to_string())?;
    notes
        .write_string(4, 1, "Database type")
        .map_err(|error| error.to_string())?;
    for (row, column) in result.columns.iter().enumerate() {
        notes
            .write_string((row + 5) as u32, 0, &column.name)
            .map_err(|error| error.to_string())?;
        notes
            .write_string((row + 5) as u32, 1, &column.data_type)
            .map_err(|error| error.to_string())?;
    }
    workbook.save_to_buffer().map_err(|error| error.to_string())
}

/// Cancels a blocking ZIP finalizer if its owning async task is aborted/closed.
struct CancelOnDrop(dbx_core::TransferControl);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct CancelWriter<'a> {
    file: &'a mut std::fs::File,
    control: dbx_core::TransferControl,
    bytes: u64,
}
impl std::io::Write for CancelWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.control.is_cancelled() {
            return Err(std::io::Error::other("XLSX export cancelled"));
        }
        let written = std::io::Write::write(self.file, bytes)?;
        self.bytes += written as u64;
        self.control.set_bytes(self.bytes);
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(self.file)
    }
}

struct StreamingWorkbook {
    workbook: Workbook,
    columns: Vec<dbx_core::ColumnInfo>,
    rows: u64,
    row_limit: u64,
    _spool: tempfile::TempDir,
}
impl StreamingWorkbook {
    fn new(parent: &std::path::Path) -> Result<Self, String> {
        let spool = tempfile::Builder::new()
            .prefix(".dbx-xlsx-")
            .tempdir_in(parent)
            .map_err(|e| e.to_string())?;
        let mut workbook = Workbook::new();
        workbook
            .set_tempdir(spool.path())
            .map_err(|e| e.to_string())?;
        workbook
            .add_worksheet_with_constant_memory()
            .map_err(|e| e.to_string())?
            .set_name("Full result")
            .map_err(|e| e.to_string())?
            .set_freeze_panes(1, 0)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            workbook,
            columns: Vec::new(),
            rows: 0,
            row_limit: 1_048_575,
            _spool: spool,
        })
    }
    fn consume(
        &mut self,
        columns: &[dbx_core::ColumnInfo],
        values: Option<Vec<CellValue>>,
    ) -> Result<(), String> {
        let sheet = self
            .workbook
            .worksheet_from_index(0)
            .map_err(|e| e.to_string())?;
        if let Some(values) = values {
            if self.rows >= self.row_limit {
                return Err("Excel supports at most 1,048,575 data rows plus the header. Use CSV or JSONL for larger results; the destination was not replaced.".into());
            }
            if values.len() != self.columns.len() {
                return Err("Export row does not match its column metadata".into());
            }
            // Preserve trailing/all-NULL row positions without turning NULL into text.
            if values.iter().all(|value| matches!(value, CellValue::Null)) {
                sheet
                    .write_blank((self.rows + 1) as u32, 0, &Format::new())
                    .map_err(|e| e.to_string())?;
            }
            for (column, value) in values.iter().enumerate() {
                write_cell(sheet, (self.rows + 1) as u32, column as u16, value)?;
            }
            self.rows += 1;
        } else {
            if columns.is_empty() || columns.len() > 16_384 {
                return Err("Excel requires between 1 and 16,384 columns".into());
            }
            self.columns = columns.to_vec();
            let header = Format::new().set_bold();
            for (column, info) in columns.iter().enumerate() {
                sheet
                    .write_string_with_format(0, column as u16, &info.name, &header)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    fn finish(
        mut self,
        destination: &std::path::Path,
        scope: &str,
        control: dbx_core::TransferControl,
    ) -> Result<u64, String> {
        if control.is_cancelled() {
            return Err("XLSX export cancelled".into());
        }
        let notes = self
            .workbook
            .add_worksheet_with_constant_memory()
            .map_err(|e| e.to_string())?;
        notes.set_name("Column types").map_err(|e| e.to_string())?;
        for (row, text) in [
            format!("{} rows. {scope}. Fresh read-only export; consistency depends on the database/table engine. Staged edits excluded.", self.rows),
            "SQL NULL is blank; empty text uses the fixed formula =\"\". Formula-like values and URLs stay literal text.".into(),
            "Integers beyond 15 digits are exact text. Non-finite numbers are text; bytes are hexadecimal; JSON is text.".into(),
        ].iter().enumerate() { notes.write_string(row as u32, 0, text).map_err(|e| e.to_string())?; }
        for (row, column) in self.columns.iter().enumerate() {
            notes
                .write_string((row + 4) as u32, 0, &column.name)
                .map_err(|e| e.to_string())?;
            notes
                .write_string((row + 4) as u32, 1, &column.data_type)
                .map_err(|e| e.to_string())?;
        }
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut output = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        control.add_log("Finalizing XLSX workbook…");
        self.workbook
            .save_to_writer(CancelWriter {
                file: output.as_file_mut(),
                control: control.clone(),
                bytes: 0,
            })
            .map_err(|e| e.to_string())?;
        output.as_file().sync_all().map_err(|e| e.to_string())?;
        if control.is_cancelled() {
            return Err("XLSX export cancelled".into());
        }
        output.persist(destination).map_err(|e| e.to_string())?;
        Ok(self.rows)
    }
}

/// Stream a validated single read through the existing core read-only transaction.
/// Constant-memory worksheets spool to private files; ZIP finalization never blocks UI/runtime.
pub async fn full_xlsx(
    engine: &dbx_core::DatabaseEngine,
    sql: &str,
    destination: std::path::PathBuf,
    scope: String,
    control: dbx_core::TransferControl,
) -> dbx_core::Result<u64> {
    let _cancel_on_drop = CancelOnDrop(control.clone());
    dbx_core::validate_query_export(engine.kind(), sql)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut workbook = StreamingWorkbook::new(parent).map_err(dbx_core::DbxError::Io)?;
    dbx_core::stream_query_export(engine, sql, |columns, values| {
        if control.is_cancelled() {
            return Err(dbx_core::DbxError::Interrupted(
                "XLSX export cancelled".into(),
            ));
        }
        workbook
            .consume(columns, values)
            .map_err(dbx_core::DbxError::Io)
    })
    .await?;
    tokio::task::spawn_blocking(move || workbook.finish(&destination, &scope, control))
        .await
        .map_err(|e| dbx_core::DbxError::Io(e.to_string()))?
        .map_err(dbx_core::DbxError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::{ColumnInfo, RowData};
    use std::io::Read;
    fn fixture() -> QueryResult {
        QueryResult {
            columns: ["nullable", "empty", "formula", "flag", "integer", "large"]
                .into_iter()
                .map(|name| ColumnInfo {
                    name: name.into(),
                    data_type: "fixture".into(),
                    nullable: true,
                    primary_key: false,
                    ordinal: 0,
                    default_value: None,
                    enum_values: Vec::new(),
                })
                .collect(),
            rows: vec![RowData {
                values: vec![
                    CellValue::Null,
                    CellValue::Text(String::new()),
                    CellValue::Text("=HYPERLINK(\"https://example.test\")".into()),
                    CellValue::Boolean(true),
                    CellValue::Integer(42),
                    CellValue::Unsigned(u64::MAX),
                ],
            }],
            rows_affected: None,
            truncated: true,
            elapsed_ms: 0,
        }
    }
    #[test]
    fn workbook_preserves_types_null_empty_and_literal_formula_text() {
        let bytes = xlsx(&fixture()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut sheet = String::new();
        zip.by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();
        assert_eq!(
            sheet.matches("<f>").count(),
            1,
            "Only the fixed empty-string formula may exist"
        );
        assert!(sheet.contains("<f>&quot;&quot;</f>") || sheet.contains("<f>\"\"</f>"));
        assert!(!sheet.contains("r=\"A2\""));
        assert!(sheet.contains("r=\"B2\""));
        assert!(sheet.contains("r=\"C2\" t=\"s\""));
        assert!(sheet.contains("r=\"D2\" t=\"b\"><v>1</v>"));
        assert!(sheet.contains("r=\"E2\"><v>42</v>"));
        assert!(sheet.contains("r=\"F2\" t=\"s\""));
        let mut strings = String::new();
        zip.by_name("xl/sharedStrings.xml")
            .unwrap()
            .read_to_string(&mut strings)
            .unwrap();
        assert!(strings.contains("18446744073709551615"));
        assert!(strings.contains("HYPERLINK"));
        assert!(strings.contains("truncated"));
    }
    fn sheet_xml(path: &std::path::Path) -> String {
        let file = std::fs::File::open(path).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut xml = String::new();
        zip.by_name("xl/worksheets/sheet1.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    #[test]
    fn streaming_workbook_retains_trailing_all_null_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nulls.xlsx");
        let result = fixture();
        let mut workbook = StreamingWorkbook::new(directory.path()).unwrap();
        workbook.consume(&result.columns, None).unwrap();
        for _ in 0..3 {
            workbook
                .consume(
                    &result.columns,
                    Some(vec![CellValue::Null; result.columns.len()]),
                )
                .unwrap();
        }
        workbook
            .finish(&path, "All NULL rows", dbx_core::TransferControl::default())
            .unwrap();
        let xml = sheet_xml(&path);
        assert_eq!(xml.matches("<row ").count(), 4);
        assert!(xml.contains("<row r=\"4\""));
        assert!(!xml.contains("r=\"A4\""), "NULL stays an empty cell");
    }

    #[test]
    fn row_limit_and_cancelled_finalization_preserve_destination_and_remove_spools() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.xlsx");
        std::fs::write(&path, b"original destination").unwrap();
        let result = fixture();
        {
            let mut workbook = StreamingWorkbook::new(directory.path()).unwrap();
            workbook.row_limit = 1;
            workbook.consume(&result.columns, None).unwrap();
            workbook
                .consume(&result.columns, Some(result.rows[0].values.clone()))
                .unwrap();
            let error = workbook
                .consume(&result.columns, Some(result.rows[0].values.clone()))
                .unwrap_err();
            assert!(error.contains("1,048,575"));
        }
        let mut workbook = StreamingWorkbook::new(directory.path()).unwrap();
        workbook.consume(&result.columns, None).unwrap();
        let control = dbx_core::TransferControl::default();
        control.cancel();
        assert!(
            workbook
                .finish(&path, "Cancelled", control)
                .unwrap_err()
                .contains("cancelled")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original destination");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        let mut file = tempfile::tempfile().unwrap();
        let control = dbx_core::TransferControl::default();
        let mut writer = CancelWriter {
            file: &mut file,
            control: control.clone(),
            bytes: 0,
        };
        std::io::Write::write_all(&mut writer, b"first ZIP chunk").unwrap();
        control.cancel();
        assert!(std::io::Write::write_all(&mut writer, b"next ZIP chunk").is_err());
    }

    async fn database(directory: &std::path::Path) -> dbx_core::DatabaseEngine {
        let url = format!(
            "sqlite://{}?mode=rwc",
            directory.join("source.sqlite").display()
        );
        dbx_core::DatabaseEngine::connect(dbx_core::ConnectionConfig::new(
            dbx_core::DatabaseKind::SQLite,
            url,
        ))
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn full_xlsx_streams_beyond_grid_limit_and_preserves_existing_output_on_failure() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let engine = database(source.path()).await;
        let path = output.path().join("all.xlsx");
        let control = dbx_core::TransferControl::default();
        let sql = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<20001) SELECT x, NULL AS missing, '' AS empty, '=1+1' AS literal FROM n";
        let rows = dbx_core::with_transfer_control(
            control.clone(),
            full_xlsx(
                &engine,
                sql,
                path.clone(),
                "Full query rerun".into(),
                control.clone(),
            ),
        )
        .await
        .unwrap();
        assert_eq!(rows, 20001);
        assert_eq!(control.progress().0, 20001);
        assert!(control.byte_progress() > 0);
        let xml = sheet_xml(&path);
        assert_eq!(xml.matches("<row ").count(), 20002);
        assert!(xml.contains("r=\"A20002\"><v>20001</v>"));
        assert!(xml.contains("=1+1"));
        let previous = std::fs::read(&path).unwrap();
        for sql in [
            "CREATE TABLE forbidden(id int)",
            "SELECT 1; SELECT 2",
            "SELECT printf('%.*c', 32768, 'x') AS too_long",
        ] {
            assert!(
                full_xlsx(
                    &engine,
                    sql,
                    path.clone(),
                    "Rejected".into(),
                    dbx_core::TransferControl::default()
                )
                .await
                .is_err(),
                "{sql}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), previous);
            assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 1);
        }
        let control = dbx_core::TransferControl::default();
        control.cancel();
        assert!(
            full_xlsx(
                &engine,
                "SELECT 1",
                path.clone(),
                "Cancelled".into(),
                control
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 1);
    }

    #[test]
    fn markdown_escapes_pipes_markup_and_embedded_lines() {
        let mut result = fixture();
        result.columns[0].name = "a|b".into();
        result.rows[0].values[0] = CellValue::Text("<script>|`x`\nline".into());
        let text = markdown(&result);
        assert!(text.contains("a&#124;b"));
        assert!(text.contains("&lt;script&gt;&#124;&#96;x&#96;<br>line"));
    }
}
