use std::{collections::HashMap, sync::Arc};

use dbx_core::{CellValue, ColumnInfo, ForeignKeyInfo, QueryResult, TableInfo};
use gpui::{
    App, Context, Div, IntoElement, Pixels, SharedString, Stateful, Window, div, prelude::*, px,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    table::{Column as DataColumn, TableDelegate, TableEvent, TableState},
};

use crate::theme::{Icon, icon, theme};

const ROW_NUMBER_COLUMN_KEY: &str = "__dbx_row_number";
const AUTO_WIDTH_SAMPLE_ROWS: usize = 200;

/// Shared, virtualized backing model for both table browsing and ad-hoc query results.
///
/// `QueryResult` stays owned by the session/tab through an `Arc`, while DataTable only
/// asks this delegate to render cells that are currently visible.
pub(super) struct ResultTableDelegate {
    result: Option<Arc<QueryResult>>,
    columns: Vec<DataColumn>,
    foreign_keys: Vec<ForeignKeyInfo>,
}

impl Default for ResultTableDelegate {
    fn default() -> Self {
        Self {
            result: None,
            columns: vec![Self::row_number_column()],
            foreign_keys: Vec::new(),
        }
    }
}

impl ResultTableDelegate {
    /// Return the underlying value for a data column (not the synthetic row-number column).
    ///
    /// Keeping this at the delegate boundary means callers can add selection, copy, or export
    /// controls without reaching through the virtualized table implementation.
    pub(super) fn cell_value(&self, row_ix: usize, data_column_ix: usize) -> Option<&CellValue> {
        self.result
            .as_ref()?
            .rows
            .get(row_ix)?
            .values
            .get(data_column_ix)
    }

    /// Return a complete underlying row, preserving `NULL` values and duplicate column names.
    pub(super) fn row_values(&self, row_ix: usize) -> Option<&[CellValue]> {
        Some(self.result.as_ref()?.rows.get(row_ix)?.values.as_slice())
    }

    /// Return a data column in result order, preserving `NULL` values.
    pub(super) fn column_values(&self, data_column_ix: usize) -> Option<Vec<&CellValue>> {
        let result = self.result.as_ref()?;
        result.columns.get(data_column_ix)?;
        Some(
            result
                .rows
                .iter()
                .filter_map(|row| row.values.get(data_column_ix))
                .collect(),
        )
    }

    /// Render one cell for a plain-text clipboard target. `NULL` is intentionally visible,
    /// while an empty text value remains empty.
    pub(super) fn cell_as_plain_text(
        &self,
        row_ix: usize,
        data_column_ix: usize,
    ) -> Option<String> {
        self.cell_value(row_ix, data_column_ix).map(plain_cell_text)
    }

    /// Render a single row as TSV, using quoted empty strings and a bare `NULL` sentinel so
    /// downstream consumers can distinguish database NULL from an empty text value.
    pub(super) fn row_as_tsv(&self, row_ix: usize) -> Option<String> {
        self.row_values(row_ix).map(|row| delimited_row(row, '\t'))
    }

    /// Render one data column as a headered TSV document. The header makes a copied column
    /// useful on its own, while `NULL` and empty text retain the same representation as rows
    /// and full-result exports.
    pub(super) fn column_as_tsv(&self, data_column_ix: usize) -> Option<String> {
        let result = self.result.as_deref()?;
        let column = result.columns.get(data_column_ix)?;
        let values = self.column_values(data_column_ix)?;

        Some(delimited_column(
            column.name.as_str(),
            values.into_iter(),
            '\t',
        ))
    }

    /// Render the complete result as a headered TSV document.
    pub(super) fn result_as_tsv(&self) -> Option<String> {
        self.result
            .as_deref()
            .map(|result| delimited_result(result, '\t'))
    }

    /// Render the complete result as a headered RFC 4180-compatible CSV document.
    pub(super) fn result_as_csv(&self) -> Option<String> {
        self.result
            .as_deref()
            .map(|result| delimited_result(result, ','))
    }

    /// Render a lossless JSON result envelope.
    ///
    /// A columns-plus-rows shape preserves duplicate SQL aliases and keeps JSON `null` distinct
    /// from an empty string, unlike a name-keyed object per row.
    pub(super) fn result_as_json(&self) -> Option<String> {
        self.result.as_deref().map(json_result)
    }

    fn row_number_column() -> DataColumn {
        DataColumn::new(ROW_NUMBER_COLUMN_KEY, "#")
            .width(44.)
            .fixed_left()
            .resizable(false)
            .movable(false)
            .selectable(false)
            .min_width(44.)
            .max_width(44.)
            .p_0()
    }

    fn data_column_key(index: usize, column: &ColumnInfo) -> String {
        // Query results may legally contain duplicate column names, so the ordinal is
        // part of the key. Humanity has already made SQL aliases difficult enough.
        format!("column:{index}:{}", column.name)
    }

    fn auto_width(result: &QueryResult, column_index: usize, column: &ColumnInfo) -> Pixels {
        let header_chars = format!("{}  {}", column.name, column.data_type)
            .chars()
            .count();
        let value_chars = result
            .rows
            .iter()
            .take(AUTO_WIDTH_SAMPLE_ROWS)
            .filter_map(|row| row.values.get(column_index))
            .map(|value| value.to_string().chars().count())
            .max()
            .unwrap_or_default();

        // This is an initial width, not a prison sentence. The user can resize it.
        px(((header_chars.max(value_chars) as f32 * 7.0) + 20.0).clamp(80.0, 420.0))
    }

    pub(super) fn set_result(
        &mut self,
        result: Option<Arc<QueryResult>>,
        remembered_widths: &HashMap<String, Pixels>,
        foreign_keys: &[ForeignKeyInfo],
        tables: &[TableInfo],
    ) {
        let mut columns = vec![Self::row_number_column()];

        if let Some(result) = result.as_deref() {
            columns.extend(result.columns.iter().enumerate().map(|(index, column)| {
                let key = Self::data_column_key(index, column);
                let width = remembered_widths
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| Self::auto_width(result, index, column));

                DataColumn::new(key, format!("{}  {}", column.name, column.data_type))
                    .width(width)
                    .resizable(true)
                    .movable(false)
                    .min_width(80.)
                    .max_width(600.)
                    .p_0()
            }));
        }

        self.result = result;
        self.columns = columns;
        self.foreign_keys = foreign_keys
            .iter()
            .filter(|foreign_key| foreign_key_target_table(tables, foreign_key).is_some())
            .cloned()
            .collect();
    }

    pub(super) fn widths_by_key(
        result: Option<&QueryResult>,
        widths: &[Pixels],
    ) -> HashMap<String, Pixels> {
        let mut remembered = HashMap::new();

        if let Some(width) = widths.first().copied() {
            remembered.insert(ROW_NUMBER_COLUMN_KEY.to_owned(), width);
        }

        if let Some(result) = result {
            for (index, column) in result.columns.iter().enumerate() {
                if let Some(width) = widths.get(index + 1).copied() {
                    remembered.insert(Self::data_column_key(index, column), width);
                }
            }
        }

        remembered
    }

    fn foreign_key_for_cell(&self, row_ix: usize, col_ix: usize) -> Option<ForeignKeyInfo> {
        if col_ix == 0 {
            return None;
        }
        let result = self.result.as_ref()?;
        let row = result.rows.get(row_ix)?;
        let column = result.columns.get(col_ix - 1)?;

        self.foreign_keys
            .iter()
            .find(|foreign_key| {
                foreign_key.columns.first() == Some(&column.name)
                    && foreign_key.columns.iter().all(|local_column| {
                        let Some(index) = result
                            .columns
                            .iter()
                            .position(|result_column| result_column.name == *local_column)
                        else {
                            return false;
                        };
                        row.values
                            .get(index)
                            .is_some_and(|value| !matches!(value, CellValue::Null))
                    })
            })
            .cloned()
    }
}

const NULL_SENTINEL: &str = "NULL";

fn plain_cell_text(value: &CellValue) -> String {
    value.to_string()
}

fn delimited_row(values: &[CellValue], delimiter: char) -> String {
    let mut output = String::new();
    append_delimited_row(&mut output, values, delimiter);
    output
}

fn delimited_result(result: &QueryResult, delimiter: char) -> String {
    let mut output = String::new();
    append_delimited_text_row(
        &mut output,
        result.columns.iter().map(|column| column.name.as_str()),
        delimiter,
    );
    for row in &result.rows {
        output.push('\n');
        append_delimited_row(&mut output, &row.values, delimiter);
    }
    output
}

fn delimited_column<'a>(
    header: &str,
    values: impl Iterator<Item = &'a CellValue>,
    delimiter: char,
) -> String {
    let mut output = String::new();
    append_quoted_delimited_text(&mut output, header, delimiter, false);
    for value in values {
        output.push('\n');
        append_delimited_cell(&mut output, value, delimiter);
    }
    output
}

fn append_delimited_row(output: &mut String, values: &[CellValue], delimiter: char) {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push(delimiter);
        }
        append_delimited_cell(output, value, delimiter);
    }
}

fn append_delimited_cell(output: &mut String, value: &CellValue, delimiter: char) {
    match value {
        CellValue::Null => output.push_str(NULL_SENTINEL),
        value => append_quoted_delimited_text(output, &plain_cell_text(value), delimiter, true),
    }
}

fn append_delimited_text_row<'a>(
    output: &mut String,
    values: impl Iterator<Item = &'a str>,
    delimiter: char,
) {
    for (index, value) in values.enumerate() {
        if index > 0 {
            output.push(delimiter);
        }
        append_quoted_delimited_text(output, value, delimiter, false);
    }
}

fn append_quoted_delimited_text(
    output: &mut String,
    value: &str,
    delimiter: char,
    protect_null_sentinel: bool,
) {
    let needs_quotes = value.is_empty()
        || (protect_null_sentinel && value == NULL_SENTINEL)
        || value.contains(delimiter)
        || value.contains('"')
        || value.contains('\r')
        || value.contains('\n');
    if !needs_quotes {
        output.push_str(value);
        return;
    }

    output.push('"');
    for character in value.chars() {
        if character == '"' {
            output.push('"');
        }
        output.push(character);
    }
    output.push('"');
}

fn json_result(result: &QueryResult) -> String {
    // Serialize directly into the final buffer. Building a serde_json::Value
    // tree first duplicates every text/JSON cell until the final string is
    // produced, which is a large and avoidable peak for result exports.
    let mut output = Vec::new();
    output.extend_from_slice(br#"{"columns":["#);
    for (index, column) in result.columns.iter().enumerate() {
        if index > 0 {
            output.push(b',');
        }
        output.extend_from_slice(br#"{"data_type":"#);
        write_json_value(&mut output, &column.data_type);
        output.extend_from_slice(br#","name":"#);
        write_json_value(&mut output, &column.name);
        output.push(b'}');
    }
    output.extend_from_slice(br#"],"rows":["#);
    for (row_index, row) in result.rows.iter().enumerate() {
        if row_index > 0 {
            output.push(b',');
        }
        output.push(b'[');
        for (value_index, value) in row.values.iter().enumerate() {
            if value_index > 0 {
                output.push(b',');
            }
            write_json_cell_value(&mut output, value);
        }
        output.push(b']');
    }
    output.extend_from_slice(b"]}");
    String::from_utf8(output).expect("serde_json always writes UTF-8")
}

fn write_json_value<T: serde::Serialize>(output: &mut Vec<u8>, value: &T) {
    serde_json::to_writer(output, value).expect("writing JSON to Vec cannot fail");
}

fn write_json_cell_value(output: &mut Vec<u8>, value: &CellValue) {
    match value {
        CellValue::Null => output.extend_from_slice(b"null"),
        CellValue::Boolean(value) => write_json_value(output, value),
        CellValue::Integer(value) => write_json_value(output, value),
        CellValue::Unsigned(value) => write_json_value(output, value),
        CellValue::Real(value) => {
            if let Some(number) = serde_json::Number::from_f64(*value) {
                write_json_value(output, &number);
            } else {
                // JSON has no NaN or infinity; keeping their text avoids
                // silently turning a real database value into a NULL export.
                write_json_value(output, &value.to_string());
            }
        }
        CellValue::Text(value) => write_json_value(output, value),
        CellValue::Bytes(_) => {
            let text = plain_cell_text(value);
            write_json_value(output, &text);
        }
        CellValue::Json(value) => write_json_value(output, value),
    }
}

impl TableDelegate for ResultTableDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.result
            .as_ref()
            .map(|result| result.rows.len())
            .unwrap_or_default()
    }

    fn column(&self, col_ix: usize, _cx: &App) -> DataColumn {
        self.columns[col_ix].clone()
    }

    fn render_header(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div()
            .id("dbx-result-header")
            .bg(theme().panel_raised)
            .border_color(theme().border_strong)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px(px(8.))
            .text_size(px(10.))
            .text_color(theme().text_muted)
            .truncate()
            .child(self.columns[col_ix].name.clone())
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div()
            .id(("dbx-result-row", row_ix))
            .border_color(theme().border)
            .bg(if row_ix.is_multiple_of(2) {
                theme().canvas
            } else {
                theme().grid_alternate
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let (text, text_color) = if col_ix == 0 {
            ((row_ix + 1).to_string(), theme().text_muted)
        } else {
            self.result
                .as_ref()
                .and_then(|result| result.rows.get(row_ix))
                .and_then(|row| row.values.get(col_ix - 1))
                .map(|value| {
                    if matches!(value, CellValue::Null) {
                        ("NULL".to_owned(), theme().text_muted)
                    } else {
                        (value.to_string(), theme().text)
                    }
                })
                .unwrap_or_else(|| ("—".to_owned(), theme().text_muted))
        };
        let foreign_key = self.foreign_key_for_cell(row_ix, col_ix);

        let mut cell = div()
            .size_full()
            .flex()
            .items_center()
            .px(px(8.))
            .whitespace_nowrap()
            .truncate()
            .text_size(px(11.))
            .text_color(text_color);
        if foreign_key.is_some() {
            cell = cell
                .child(div().flex_1().min_w_0().truncate().child(text))
                .child(
                    Button::new(SharedString::from(format!(
                        "foreign-key-link-{row_ix}-{col_ix}"
                    )))
                    .with_size(gpui_component::Size::XSmall)
                    .compact()
                    .ghost()
                    .tooltip("Open referenced row")
                    .text_color(theme().accent)
                    .child(icon(Icon::ArrowRight, theme().accent))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.stop_propagation();
                        cx.emit(TableEvent::DoubleClickedCell(row_ix, col_ix));
                    })),
                );
        } else {
            cell = cell.child(text);
        }
        cell
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme().text_muted)
            .child("No rows returned")
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        if col_ix == 0 {
            return (row_ix + 1).to_string();
        }

        self.result
            .as_ref()
            .and_then(|result| result.rows.get(row_ix))
            .and_then(|row| row.values.get(col_ix - 1))
            .map(ToString::to_string)
            .unwrap_or_default()
    }
}

pub(super) fn foreign_key_target_table(
    tables: &[TableInfo],
    foreign_key: &ForeignKeyInfo,
) -> Option<TableInfo> {
    tables
        .iter()
        .find(|table| {
            table.name == foreign_key.referenced_table
                && match foreign_key.referenced_schema.as_deref() {
                    Some(schema) => table.schema.as_deref() == Some(schema),
                    None => true,
                }
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::RowData;

    fn export_result() -> QueryResult {
        QueryResult {
            columns: vec![
                ColumnInfo::result("id", 0, "INTEGER"),
                ColumnInfo::result("note", 1, "TEXT"),
                ColumnInfo::result("note", 2, "TEXT"),
            ],
            rows: vec![
                RowData::new(vec![
                    CellValue::Integer(7),
                    CellValue::Null,
                    CellValue::Text(String::new()),
                ]),
                RowData::new(vec![
                    CellValue::Integer(8),
                    CellValue::Text("comma, tab\t quote\" newline\n".into()),
                    CellValue::Text("NULL".into()),
                ]),
            ],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        }
    }

    fn delegate_with_export_result() -> ResultTableDelegate {
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(Some(Arc::new(export_result())), &HashMap::new(), &[], &[]);
        delegate
    }

    #[test]
    fn foreign_key_target_resolves_the_referenced_schema() {
        let tables = vec![
            TableInfo::table("users", Some("analytics".into())),
            TableInfo::table("users", Some("public".into())),
        ];
        let foreign_key = ForeignKeyInfo {
            constraint_name: Some("events_user_id_fkey".into()),
            columns: vec!["user_id".into()],
            referenced_schema: Some("analytics".into()),
            referenced_table: "users".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };

        assert_eq!(
            foreign_key_target_table(&tables, &foreign_key),
            Some(TableInfo::table("users", Some("analytics".into())))
        );
    }

    #[test]
    fn foreign_key_target_is_unavailable_when_the_table_is_not_listed() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: None,
            columns: vec!["owner_id".into()],
            referenced_schema: None,
            referenced_table: "owners".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };

        assert_eq!(foreign_key_target_table(&[], &foreign_key), None);
    }

    #[test]
    fn result_grid_marks_populated_foreign_key_cells_as_navigable() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: Some("orders_customer_id_fkey".into()),
            columns: vec!["customer_id".into()],
            referenced_schema: Some("public".into()),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };
        let tables = vec![TableInfo::table("customers", Some("public".into()))];
        let result = QueryResult {
            columns: vec![
                ColumnInfo::result("id", 0, "INTEGER"),
                ColumnInfo::result("customer_id", 1, "INTEGER"),
            ],
            rows: vec![RowData::new(vec![
                CellValue::Integer(1),
                CellValue::Integer(42),
            ])],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        };
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(result)),
            &HashMap::new(),
            &[foreign_key],
            &tables,
        );

        assert!(delegate.foreign_key_for_cell(0, 2).is_some());
    }

    #[test]
    fn result_grid_hides_foreign_key_action_for_null_values() {
        let foreign_key = ForeignKeyInfo {
            constraint_name: None,
            columns: vec!["customer_id".into()],
            referenced_schema: Some("public".into()),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        };
        let tables = vec![TableInfo::table("customers", Some("public".into()))];
        let result = QueryResult {
            columns: vec![ColumnInfo::result("customer_id", 0, "INTEGER")],
            rows: vec![RowData::new(vec![CellValue::Null])],
            rows_affected: None,
            truncated: false,
            elapsed_ms: 0,
        };
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(result)),
            &HashMap::new(),
            &[foreign_key],
            &tables,
        );

        assert!(delegate.foreign_key_for_cell(0, 1).is_none());
    }

    #[test]
    fn result_accessors_retain_database_nulls_and_data_column_order() {
        let delegate = delegate_with_export_result();

        assert_eq!(delegate.cell_value(0, 1), Some(&CellValue::Null));
        assert_eq!(delegate.cell_as_plain_text(0, 1).as_deref(), Some("NULL"));
        assert_eq!(delegate.cell_as_plain_text(0, 2).as_deref(), Some(""));
        assert_eq!(delegate.row_values(1).unwrap()[0], CellValue::Integer(8));
        assert_eq!(
            delegate.column_values(0).unwrap(),
            vec![&CellValue::Integer(7), &CellValue::Integer(8)]
        );
        assert!(delegate.cell_value(8, 0).is_none());
        assert!(delegate.column_values(8).is_none());
    }

    #[test]
    fn delimited_exports_escape_controls_and_preserve_null_vs_empty_text() {
        let delegate = delegate_with_export_result();

        assert_eq!(delegate.row_as_tsv(0).as_deref(), Some("7\tNULL\t\"\""));
        assert_eq!(
            delegate.result_as_csv().as_deref(),
            Some("id,note,note\n7,NULL,\"\"\n8,\"comma, tab\t quote\"\" newline\n\",\"NULL\"")
        );
        assert_eq!(
            delegate.result_as_tsv().as_deref(),
            Some(
                "id\tnote\tnote\n7\tNULL\t\"\"\n8\t\"comma, tab\t quote\"\" newline\n\"\t\"NULL\""
            )
        );
    }

    #[test]
    fn column_tsv_export_includes_its_header_and_preserves_null_vs_empty_text() {
        let delegate = delegate_with_export_result();

        assert_eq!(
            delegate.column_as_tsv(1).as_deref(),
            Some("note\nNULL\n\"comma, tab\t quote\"\" newline\n\"")
        );
        assert_eq!(
            delegate.column_as_tsv(2).as_deref(),
            Some("note\n\"\"\n\"NULL\"")
        );
    }

    #[test]
    fn column_tsv_export_returns_none_without_a_result_or_for_an_invalid_column() {
        let empty_delegate = ResultTableDelegate::default();
        assert!(empty_delegate.column_as_tsv(0).is_none());

        let delegate = delegate_with_export_result();
        assert!(delegate.column_as_tsv(8).is_none());
    }

    #[test]
    fn json_export_is_positional_to_preserve_duplicate_aliases_and_nulls() {
        let delegate = delegate_with_export_result();
        let exported: serde_json::Value =
            serde_json::from_str(&delegate.result_as_json().unwrap()).unwrap();

        assert_eq!(exported["columns"][1]["name"], "note");
        assert_eq!(exported["columns"][2]["name"], "note");
        assert!(exported["rows"][0][1].is_null());
        assert_eq!(exported["rows"][0][2], "");
        assert_eq!(exported["rows"][1][2], "NULL");
    }

    #[test]
    fn empty_result_exports_its_headers_instead_of_disappearing() {
        let mut delegate = ResultTableDelegate::default();
        delegate.set_result(
            Some(Arc::new(QueryResult {
                columns: vec![ColumnInfo::result("id", 0, "INTEGER")],
                rows: Vec::new(),
                rows_affected: None,
                truncated: false,
                elapsed_ms: 0,
            })),
            &HashMap::new(),
            &[],
            &[],
        );

        assert_eq!(delegate.result_as_csv().as_deref(), Some("id"));
        assert_eq!(delegate.result_as_tsv().as_deref(), Some("id"));
    }
}
