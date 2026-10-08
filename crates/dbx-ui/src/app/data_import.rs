use super::*;
use dbx_core::data_import::{ImportData, import_data};

fn preview_cell(value: &CellValue) -> String {
    let text = match value {
        CellValue::Text(text) => return text.chars().take(100).collect(),
        CellValue::Json(_) => "[JSON value]".into(),
        CellValue::Bytes(bytes) => format!("[{} bytes]", bytes.len()),
        _ => value.to_string(),
    };
    text.chars().take(100).collect()
}

pub(super) struct DataImportDialog {
    session_id: SessionId,
    table: TableInfo,
    database: String,
    columns: Vec<dbx_core::ColumnInfo>,
    data: Arc<ImportData>,
    mapping: Vec<Entity<TextEditor>>,
    pub(super) busy: bool,
    error: Option<String>,
}
impl DbxApp {
    pub(super) fn preview_data_import(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self.session(session_id).and_then(|s| s.engine.clone()) else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let target = table_ref(&table);
            let result = runtime
                .spawn(async move {
                    let data = tokio::task::spawn_blocking(move || ImportData::read(&path))
                        .await
                        .map_err(|e| dbx_core::DbxError::Io(e.to_string()))??;
                    let columns = engine.describe_table(&target).await?;
                    let database = engine.current_database().await?;
                    Ok::<_, dbx_core::DbxError>((data, columns, database))
                })
                .await?;
            this.update_in(cx, |this, window, cx| {
                if this.vault_state != Some(VaultState::Unlocked)
                    || this.session(session_id).is_none()
                {
                    return;
                }
                match result {
                    Ok((data, columns, database)) => {
                        let mapping = data
                            .default_mapping(&columns)
                            .into_iter()
                            .map(|name| {
                                let value = cx.new(|_| name.unwrap_or_default());
                                cx.new(|cx| TextEditor::new(value, false, window, cx))
                            })
                            .collect::<Vec<_>>();
                        if let Some(first) = mapping.first() {
                            first.read(cx).focus_handle().focus(window, cx);
                        }
                        this.data_import_dialog = Some(DataImportDialog {
                            session_id,
                            table,
                            database,
                            columns,
                            data: Arc::new(data),
                            mapping,
                            busy: false,
                            error: None,
                        });
                    }
                    Err(error) => this.set_error(error.to_string()),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
    pub(super) fn copy_or_compare_table(
        &mut self,
        id: SessionId,
        table: TableInfo,
        action: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(engine) = self
            .session(id)
            .filter(|session| !session.busy)
            .and_then(|session| session.engine.clone())
        else {
            return;
        };
        if action != 0 && self.copied_table_data.is_none() {
            self.show_toast(ToastKind::Info, "Capture the source table first", cx);
            return;
        }
        let source = self.copied_table_data.clone();
        let task = self.runtime.spawn(async move {
            let reference = table_ref(&table);
            let columns = engine.describe_table(&reference).await?;
            let database = engine.current_database().await?;
            let data = if action == 1 {
                source.clone().unwrap()
            } else {
                Arc::new(dbx_core::data_import::snapshot_data(&engine, &reference).await?)
            };
            let diff = if action == 2 {
                Some(dbx_core::data_import::diff_data(
                    source.as_ref().unwrap(),
                    &data,
                    &columns
                        .iter()
                        .filter(|c| c.primary_key)
                        .map(|c| c.name.clone())
                        .collect::<Vec<_>>(),
                )?)
            } else {
                None
            };
            Ok::<_, dbx_core::DbxError>((table, data, columns, database, diff))
        });
        self.session_mut(id).unwrap().track_background_task(&task);
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await?;
            this.update_in(cx, |this, window, cx| {
                if this.vault_state != Some(VaultState::Unlocked) || this.session(id).is_none() { return; }
                match result {
                    Ok((table, data, columns, database, diff)) => {
                        if action == 0 { this.copied_table_data = Some(data); this.show_toast(ToastKind::Info, "Captured a consistent table snapshot. Choose another table’s ‘Append captured data’ or ‘Compare with captured data’. Limit: 100,000 rows / 64 MiB.", cx); }
                        else if action == 1 {
                            let mapping = data.default_mapping(&columns).into_iter().map(|name| { let value = cx.new(|_| name.unwrap_or_default()); cx.new(|cx| TextEditor::new(value, false, window, cx)) }).collect();
                            this.data_import_dialog = Some(DataImportDialog { session_id: id, table, database, columns, data, mapping, busy: false, error: Some("Cross-connection copy can convert types. Review destination types and sample values; unsupported values fail and roll back the copy. Existing keys are not replaced.".into()) });
                        } else {
                            let diff = diff.unwrap();
                            this.open_saved_query_for(id, crate::workspace::SavedQuery { name: "Data comparison".into(), sql: format!("-- Complete bounded snapshots compared by destination primary key. Types compare exactly; snapshots were captured at different times.\n-- Source only: {}\n-- Destination only: {}\n-- Changed: {}\n-- Equal: {}\n-- No synchronization writes were generated or executed.", diff.only_source, diff.only_target, diff.changed, diff.equal) }, window, cx);
                        }
                    }
                    Err(error) => this.set_error(error.to_string()),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        }).detach();
    }
    fn execute_previewed_import(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.data_import_dialog else {
            return;
        };
        if dialog.busy {
            return;
        }
        let id = dialog.session_id;
        let Some(session) = self.session(id) else {
            self.data_import_dialog = None;
            return;
        };
        if session.busy
            || session.secondary_tabs.iter().any(|tab| matches!(&tab.kind, SecondaryTabKind::Query(q) if q.busy || q.in_transaction))
            || session.secondary_tabs.iter().any(|tab| matches!(&tab.kind, SecondaryTabKind::Data(data) if data.has_unsaved_cell_work()))
        {
            self.show_toast(ToastKind::Info, "Finish pending work before importing", cx);
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let data = dialog.data.clone();
        let table = table_ref(&dialog.table);
        let database = dialog.database.clone();
        let columns = dialog.columns.clone();
        let mapping = dialog
            .mapping
            .iter()
            .map(|editor| {
                let value = editor.read(cx).text(cx);
                (!value.is_empty()).then_some(value)
            })
            .collect::<Vec<_>>();
        self.data_import_dialog.as_mut().unwrap().busy = true;
        let session = self.session_mut(id).unwrap();
        session.busy = true;
        session.request_generation += 1;
        session.status = "Importing previewed data…".into();
        let generation = session.request_generation;
        let control = self.start_transfer_progress(id, generation, cx);
        let task = self.runtime.spawn(async move {
            dbx_core::with_transfer_control(
                control,
                import_data(&engine, &table, &data, &mapping, &database, &columns),
            )
            .await
        });
        self.session_mut(id).unwrap().track_background_task(&task);
        cx.spawn(async move |this, cx| {
            let result = task
                .await
                .unwrap_or_else(|e| Err(dbx_core::DbxError::Io(e.to_string())));
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(id) else {
                    return;
                };
                if session.request_generation != generation {
                    return;
                }
                session.busy = false;
                session.transfer_control = None;
                match result {
                    Ok(rows) => {
                        this.data_import_dialog = None;
                        this.show_toast(ToastKind::Success, format!("Imported {rows} rows"), cx);
                        this.refresh_tables_for(id, cx);
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut this.data_import_dialog {
                            dialog.busy = false;
                            dialog.error = Some(error.to_string());
                        }
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_data_import(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = &self.data_import_dialog else {
            return div().into_any_element();
        };
        div().absolute().inset_0().bg(theme().overlay).flex().items_center().justify_center().on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(glass_raised(div(), RADIUS_GLASS).w(px(760.)).max_w(relative(0.95)).max_h(relative(0.9)).id("data-import-dialog").overflow_y_scroll().p(px(20.)).flex().flex_col().gap(px(12.))
                .child(div().text_lg().child(format!("Review {} rows → {} / {}", dialog.data.rows.len(), dialog.database, table_sidebar_label(&dialog.table, None))))
                .child(div().text_sm().child("Map each source field to a destination column. Leave blank to omit it and use the database default. CSV unquoted empty = NULL; quoted empty = empty text. JSON null = NULL; dates stay text; binary uses the destination’s supported binary format. Any failed row rolls back the entire import."))
                .child(div().text_sm().text_color(theme().text_muted).child(format!("Destination columns: {}", dialog.columns.iter().map(|c| format!("{} ({})", c.name, c.data_type)).collect::<Vec<_>>().join(", "))))
                .child(div().id("data-import-fields").max_h(px(300.)).overflow_y_scroll().flex().flex_col().gap(px(8.)).children(dialog.data.headers.iter().enumerate().map(|(index, name)| {
                    let samples = dialog.data.rows.iter().take(3).map(|row| preview_cell(&row[index])).collect::<Vec<_>>().join(" · ");
                    div().flex().gap(px(12.)).items_center().child(div().w(px(300.)).text_sm().child(format!("{name}: {samples}"))).child(div().flex_1().child(editor::input(dialog.mapping[index].clone(), dialog.mapping[index].read(cx).focus_handle(), false)))
                })))
                .when_some(dialog.error.clone(), |view, error| view.child(div().text_sm().text_color(theme().danger).child(error)))
                .child(div().flex().justify_end().gap(px(8.))
                    .child(button("data-import-cancel", if dialog.busy { "Cancel import" } else { "Cancel" }, ButtonKind::Quiet).on_click(cx.listener(|this, _, window, cx| { if let Some(dialog) = &this.data_import_dialog && dialog.busy { this.cancel_transfer_for(dialog.session_id, cx); return; } this.data_import_dialog = None; this.focus_handle.focus(window, cx); cx.notify(); })))
                    .child(button("data-import-run", "Append reviewed rows", ButtonKind::Primary).disabled(dialog.busy).on_click(cx.listener(|this, _, _, cx| this.execute_previewed_import(cx))))))
            .into_any_element()
    }
}
