//! Exports of an already-loaded database snapshot. No query is executed here.
use dbx_core::{CellValue, QueryResult};
use rust_xlsxwriter::{Format, Formula, Workbook};

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
            let write = match value {
                CellValue::Null => continue,
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
                CellValue::Real(value) if value.is_finite() => {
                    sheet.write_number(row, column, *value)
                }
                value => sheet.write_string(row, column, plain(value)),
            };
            write.map_err(|error| format!("Row {}, column {}: {error}", row + 1, column + 1))?;
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
