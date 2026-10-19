//! Keyboard copy and paste for table data tabs. Copy follows the most
//! specific selection; paste stages values into the changeset and never
//! writes to the database by itself.

use dbx_core::MutationValue;

use super::*;
use crate::row_drafts::parse_field_value;

impl DbxApp {
    pub(super) fn copy_data_markdown_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = self
            .data_tab(session_id, tab_id)
            .and_then(|data| data.result.clone())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(crate::result_export::markdown(
            &result,
        )));
        self.show_toast(
            ToastKind::Success,
            format!(
                "Copied {} loaded rows as Markdown; staged edits excluded",
                result.rows.len()
            ),
            cx,
        );
    }

    pub(super) fn export_data_xlsx_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        if let Some(result) = self
            .data_tab(session_id, tab_id)
            .and_then(|data| data.result.clone())
        {
            self.export_loaded_xlsx(result, "table-loaded-rows.xlsx".into(), cx);
        }
    }

    pub(super) fn export_loaded_xlsx(
        &mut self,
        result: Arc<QueryResult>,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));
        let runtime = self.runtime.clone();
        let rows = result.rows.len();
        let partial = result.truncated;
        cx.spawn(async move |this, cx| {
            let outcome = match receiver.await {
                Ok(Ok(Some(path))) => runtime
                    .spawn_blocking(move || {
                        let bytes = crate::result_export::xlsx(&result)?;
                        dbx_core::write_atomic_export(&path, &bytes)
                            .map_err(|error| error.to_string())
                    })
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|result| result),
                Ok(Ok(None)) => return Ok::<(), anyhow::Error>(()),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| match outcome {
                Ok(()) => this.show_toast(
                    ToastKind::Success,
                    format!(
                        "Exported {rows} loaded rows to XLSX{}; staged edits excluded",
                        if partial { " (partial result)" } else { "" }
                    ),
                    cx,
                ),
                Err(error) => {
                    this.show_toast(ToastKind::Error, format!("XLSX export: {error}"), cx)
                }
            })?;
            Ok(())
        })
        .detach();
    }

    /// Copy the marked rows, else the selected cell, row or column, as TSV.
    pub(super) fn copy_data_selection_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        let grid = data.data_grid.read(cx);
        let delegate = grid.delegate();
        let copied = if data.marked_rows.len() > 1 {
            let rows = data
                .marked_rows
                .iter()
                .filter_map(|row| delegate.row_as_tsv(*row))
                .collect::<Vec<_>>();
            let label = counted(rows.len(), "row", "rows");
            Some((rows.join("\n"), label))
        } else if let Some((row, column)) = grid.selected_cell() {
            delegate.result_column(column).and_then(|column| {
                data.shown_value(row, column).map(|value| {
                    (
                        delegate.binary_format(column).editor_text(&value),
                        "cell".to_owned(),
                    )
                })
            })
        } else if let Some(row) = grid.selected_row() {
            delegate
                .row_as_tsv(row)
                .map(|text| (text, "row".to_owned()))
        } else {
            grid.selected_col()
                .and_then(|column| delegate.column_as_tsv(delegate.result_column(column)?))
                .map(|text| (text, "column".to_owned()))
        };
        let Some((text, label)) = copied else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast(ToastKind::Success, format!("Copied {label}"), cx);
    }

    /// Stage clipboard TSV from the selected cell: existing cells become
    /// edits and rows past the end become new rows. With a whole row selected
    /// (or nothing), pasted rows are added as new rows. Every value is
    /// validated first, so a bad value stages nothing.
    pub(super) fn paste_rows_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        if self
            .data_tab(session_id, tab_id)
            .is_some_and(|data| data.recovered_changeset.is_some())
        {
            self.show_toast(
                ToastKind::Info,
                "Review and commit or discard recovered changes before pasting",
                cx,
            );
            return;
        }
        if self.editable_table_for(session_id, tab_id).is_none() {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let records = parse_tsv(&text);
        if records.is_empty() {
            return;
        }
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        if data.row_draft.is_some() || data.cell_editor.is_some() {
            return;
        }
        let Some(result) = data.result.clone() else {
            return;
        };
        let grid = data.data_grid.read(cx);
        // Pasted fields fill columns in display order from the selected cell.
        let order = grid.delegate().display_order().to_vec();
        let (start_row, start_column) = match grid.selected_cell() {
            Some((row, column)) => (row, column.saturating_sub(1)),
            None => (data.loaded_rows_with_inserts(), 0),
        };
        let total_rows = data.loaded_rows_with_inserts();
        let mut staged = Vec::new();
        for (offset, record) in records.iter().enumerate() {
            let row = start_row + offset;
            for (index, field) in record.iter().enumerate() {
                let Some(&column) = order.get(start_column + index) else {
                    break;
                };
                let info = &result.columns[column];
                let metadata = data
                    .table_columns
                    .iter()
                    .find(|candidate| candidate.name == info.name)
                    .unwrap_or(info);
                let value = match field {
                    None => CellValue::Null,
                    Some(text) => match if crate::row_drafts::field_value_kind(metadata)
                        == crate::row_drafts::FieldValueKind::Bytes
                    {
                        grid.delegate().binary_format(column).parse(text)
                    } else {
                        parse_field_value(metadata, text).map_err(|error| error.to_string())
                    } {
                        Ok(value) => value,
                        Err(error) => {
                            self.show_toast(
                                ToastKind::Error,
                                format!("Row {}, {}: {error}", offset + 1, info.name),
                                cx,
                            );
                            return;
                        }
                    },
                };
                staged.push((row, column, value));
            }
        }
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        let new_rows = (start_row + records.len()).saturating_sub(total_rows);
        data.pending_inserts
            .extend(std::iter::repeat_with(Default::default).take(new_rows));
        for (row, column, value) in staged {
            if data.pending_deletes.contains(&row) {
                continue;
            }
            data.stage_cell(row, column, Some(MutationValue::Parameter(value)));
        }
        data.sync_cell_edits(cx);
        self.show_toast(
            ToastKind::Success,
            format!(
                "Staged {}",
                counted(records.len(), "pasted row", "pasted rows")
            ),
            cx,
        );
        cx.notify();
    }
}

/// Parse tab-separated clipboard text. Quoted fields may contain tabs,
/// newlines and doubled quotes; a bare `NULL` is SQL NULL, matching DBX's
/// own TSV copy.
fn parse_tsv(text: &str) -> Vec<Vec<Option<String>>> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    let mut chars = text.chars().peekable();
    let finish = |field: &mut String, was_quoted: &mut bool| {
        let value = if !*was_quoted && field == "NULL" {
            None
        } else {
            Some(std::mem::take(field))
        };
        field.clear();
        *was_quoted = false;
        value
    };
    while let Some(character) = chars.next() {
        match character {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() && !was_quoted => {
                quoted = true;
                was_quoted = true;
            }
            '\t' if !quoted => record.push(finish(&mut field, &mut was_quoted)),
            '\r' if !quoted && chars.peek() == Some(&'\n') => {}
            '\n' if !quoted => {
                record.push(finish(&mut field, &mut was_quoted));
                records.push(std::mem::take(&mut record));
            }
            character => field.push(character),
        }
    }
    record.push(finish(&mut field, &mut was_quoted));
    records.push(record);
    records
}

#[cfg(test)]
mod tests {
    use super::parse_tsv;

    #[test]
    fn tsv_paste_keeps_nulls_quotes_and_embedded_newlines() {
        assert_eq!(
            parse_tsv("1\tNULL\t\"\"\n2\t\"a\tb\nc\"\t\"say \"\"hi\"\"\"\n"),
            vec![
                vec![Some("1".into()), None, Some(String::new())],
                vec![
                    Some("2".into()),
                    Some("a\tb\nc".into()),
                    Some("say \"hi\"".into())
                ],
            ]
        );
        assert_eq!(parse_tsv("\"NULL\""), vec![vec![Some("NULL".into())]]);
        assert!(parse_tsv("").is_empty());
    }
}
