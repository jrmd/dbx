//! Inline grid editing. Double-clicking a cell stages a typed value; staged
//! rows are saved together through checked, primary-key guarded updates.

use std::collections::BTreeMap;

use dbx_core::MutationValue;

use super::*;
use crate::editor::EditorLanguage;
use crate::row_drafts::{field_editor_text, parse_field_value};

/// The combined context lets the cell's Enter/Tab/Escape bindings sit at the
/// editor's own dispatch depth.
pub(super) const CELL_EDITOR_CONTEXT: &str = "DbxTextEditor DbxCellEditor";

/// Staged values keyed by (row, data column) in the loaded page.
pub(super) type PendingEdits = BTreeMap<(usize, usize), CellValue>;

pub(super) struct CellEditor {
    pub(super) row: usize,
    pub(super) column: usize,
    pub(super) editor: Entity<TextEditor>,
    pub(super) structured: bool,
    _blur: Subscription,
}

impl DataTab {
    pub(super) fn has_pending_edits(&self) -> bool {
        !self.pending_edits.is_empty()
    }

    pub(super) fn has_unsaved_cell_work(&self) -> bool {
        self.has_pending_edits() || self.cell_editor.is_some()
    }

    /// Mirror staged values and the open editor into the grid delegate.
    pub(super) fn sync_cell_edits(&self, cx: &mut Context<DbxApp>) {
        let pending = self
            .pending_edits
            .iter()
            .map(|(key, value)| (*key, value.clone()))
            .collect();
        let editing = self
            .cell_editor
            .as_ref()
            .filter(|editor| !editor.structured)
            .map(|editor| (editor.row, editor.column, editor.editor.clone()));
        self.data_grid.update(cx, |table, cx| {
            table.delegate_mut().set_cell_edits(pending, editing);
            cx.notify();
        });
    }
}

impl DbxApp {
    /// Describe the staged edits in a data tab, if any, for a blocking toast.
    pub(super) fn pending_edits_block(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) -> bool {
        if self
            .data_tab(session_id, tab_id)
            .is_some_and(|data| data.cell_editor.is_some())
        {
            self.show_toast(
                ToastKind::Info,
                "Finish or cancel the open cell edit first",
                cx,
            );
            return true;
        }
        let Some(count) = self
            .data_tab(session_id, tab_id)
            .map(|data| data.pending_edits.len())
            .filter(|count| *count > 0)
        else {
            return false;
        };
        self.show_toast(
            ToastKind::Info,
            format!(
                "Save or discard {} first",
                counted(count, "staged cell change", "staged cell changes")
            ),
            cx,
        );
        true
    }

    pub(super) fn begin_cell_edit_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        row: usize,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editable_table_for(session_id, tab_id).is_none() {
            return;
        }
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        if data.row_draft.is_some() {
            self.show_toast(ToastKind::Info, "Finish the open row edit first", cx);
            return;
        }
        let Some(original) = data
            .result
            .as_ref()
            .and_then(|result| result.rows.get(row)?.values.get(column))
            .cloned()
        else {
            return;
        };
        if data.cell_editor.is_some() {
            self.commit_cell_edit_for(session_id, tab_id, None, window, cx);
        }
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        // A failed commit keeps its editor open for correction.
        if data.cell_editor.is_some() {
            return;
        }
        let value = data
            .pending_edits
            .get(&(row, column))
            .cloned()
            .unwrap_or(original);
        let structured = value_view::json_preview(&value).is_some()
            || data
                .table_columns
                .get(column)
                .is_some_and(|column| column.data_type.to_ascii_lowercase().contains("json"));
        let text = match &value {
            CellValue::Null => String::new(),
            value => value_view::json_preview(value).unwrap_or_else(|| field_editor_text(value)),
        };
        let value = cx.new(|_| text);
        let editor = cx.new(|cx| {
            TextEditor::new_with_language(
                value,
                structured,
                if structured {
                    EditorLanguage::Json
                } else {
                    EditorLanguage::PlainText
                },
                window,
                cx,
            )
        });
        let focus = editor.read(cx).focus_handle();
        let blur = cx.on_blur(&focus, window, move |this, window, cx| {
            if !structured {
                this.commit_cell_edit_for(session_id, tab_id, None, window, cx);
            }
        });
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        data.cell_editor = Some(CellEditor {
            row,
            column,
            editor: editor.clone(),
            structured,
            _blur: blur,
        });
        data.sync_cell_edits(cx);
        editor.update(cx, |editor, cx| editor.select_all_text(cx));
        focus.focus(window, cx);
        cx.notify();
    }

    /// Stage the open editor's value. `advance` moves the editor to the next
    /// (`Some(true)`) or previous (`Some(false)`) column afterwards.
    pub(super) fn commit_cell_edit_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        advance: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        let Some(cell) = data.cell_editor.as_ref() else {
            return;
        };
        let (row, column) = (cell.row, cell.column);
        let structured = cell.structured;
        let text = cell.editor.read(cx).text(cx);
        let Some((metadata, original, column_count)) = data.result.as_ref().and_then(|result| {
            let result_column = result.columns.get(column)?;
            let metadata = data
                .table_columns
                .iter()
                .find(|candidate| candidate.name == result_column.name)
                .unwrap_or(result_column)
                .clone();
            let original = result.rows.get(row)?.values.get(column)?.clone();
            Some((metadata, original, result.columns.len()))
        }) else {
            return;
        };
        // An emptied nullable cell that was NULL stays NULL instead of
        // becoming an empty string the user never typed.
        if structured
            && !(text.is_empty() && original == CellValue::Null)
            && let Err(error) = serde_json::from_str::<serde_json::Value>(&text)
        {
            self.show_toast(ToastKind::Error, format!("Invalid JSON: {error}"), cx);
            return;
        }
        let value = if text.is_empty() && original == CellValue::Null {
            Ok(CellValue::Null)
        } else {
            parse_field_value(&metadata, &text)
        };
        let value = match value {
            Ok(value) => value,
            Err(error) => {
                self.show_toast(ToastKind::Error, error.to_string(), cx);
                return;
            }
        };
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        if value == original {
            data.pending_edits.remove(&(row, column));
        } else {
            data.pending_edits.insert((row, column), value);
        }
        data.cell_editor = None;
        data.sync_cell_edits(cx);
        let next = advance.and_then(|forward| {
            if forward {
                (column + 1 < column_count).then_some(column + 1)
            } else {
                column.checked_sub(1)
            }
        });
        match next {
            Some(next) => self.begin_cell_edit_for(session_id, tab_id, row, next, window, cx),
            None => {
                let grid = data.data_grid.read(cx).focus_handle(cx);
                grid.focus(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn cancel_cell_edit_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        if data.cell_editor.take().is_none() {
            return;
        }
        data.sync_cell_edits(cx);
        let grid = data.data_grid.read(cx).focus_handle(cx);
        grid.focus(window, cx);
        cx.notify();
    }

    /// Stage NULL for the open editor's cell when its column allows it.
    pub(super) fn set_cell_null_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        let Some(cell) = data.cell_editor.as_ref() else {
            return;
        };
        let (row, column) = (cell.row, cell.column);
        let Some((nullable, original)) = data.result.as_ref().and_then(|result| {
            let name = &result.columns.get(column)?.name;
            let nullable = data
                .table_columns
                .iter()
                .find(|candidate| candidate.name == *name)
                .is_none_or(|candidate| candidate.nullable);
            Some((nullable, result.rows.get(row)?.values.get(column)?.clone()))
        }) else {
            return;
        };
        if !nullable {
            self.show_toast(ToastKind::Error, "This column does not allow NULL", cx);
            return;
        }
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        if original == CellValue::Null {
            data.pending_edits.remove(&(row, column));
        } else {
            data.pending_edits.insert((row, column), CellValue::Null);
        }
        data.cell_editor = None;
        data.sync_cell_edits(cx);
        let grid = data.data_grid.read(cx).focus_handle(cx);
        grid.focus(window, cx);
        cx.notify();
    }

    pub(super) fn discard_pending_edits_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) {
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        data.pending_edits.clear();
        data.cell_editor = None;
        data.sync_cell_edits(cx);
        cx.notify();
    }

    /// Apply every staged row as one checked batch. Each row must still hold
    /// its displayed values; a conflict keeps the staged edits for review.
    pub(super) fn save_pending_edits_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(engine), Some(table)) = (
            self.session(session_id)
                .and_then(|session| session.engine.clone()),
            self.editable_table_for(session_id, tab_id).cloned(),
        ) else {
            return;
        };
        let Some(data) = self.data_tab(session_id, tab_id) else {
            return;
        };
        if data.busy || data.pending_edits.is_empty() {
            return;
        }
        let Some(result) = data.result.clone() else {
            return;
        };
        let mut rows: BTreeMap<usize, Vec<(usize, CellValue)>> = BTreeMap::new();
        for ((row, column), value) in &data.pending_edits {
            rows.entry(*row).or_default().push((*column, value.clone()));
        }
        let mut updates = Vec::with_capacity(rows.len());
        for (row_index, cells) in rows {
            let Some(row) = result.rows.get(row_index) else {
                continue;
            };
            let filters = match self.identity_filters_for(session_id, tab_id, row) {
                Ok(filters) => filters,
                Err(error) => {
                    self.show_mutation_error_for(session_id, tab_id, error, None, window, cx);
                    return;
                }
            };
            let mut assignments = Vec::with_capacity(cells.len());
            let mut originals = Vec::with_capacity(cells.len());
            for (column, value) in cells {
                let name = result.columns[column].name.clone();
                originals.push((name.clone(), row.values[column].clone()));
                assignments.push((name, MutationValue::parameter(value)));
            }
            updates.push((
                UpdateRequest::new_with_mutation_values(table.clone(), assignments, filters),
                originals,
            ));
        }
        let row_count = updates.len();
        let runtime = self.runtime.clone();
        let Some(data) = self.data_tab_mut(session_id, tab_id) else {
            return;
        };
        data.busy = true;
        data.error = None;
        data.status = format!("Saving {}…", counted(row_count, "row", "rows"));
        data.request_generation += 1;
        let generation = data.request_generation;
        let task = runtime.spawn(async move { engine.update_checked_batch(&updates).await });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let outcome = task
                .await
                .map_err(|error| format!("Row update task failed: {error}"))
                .and_then(|outcome| outcome.map_err(|error| error.to_string()));
            this.update_in(cx, |this, window, cx| {
                let Some(data) = this.data_tab_mut(session_id, tab_id) else {
                    return;
                };
                if generation != data.request_generation {
                    return;
                }
                data.busy = false;
                match outcome {
                    Ok(saved) => {
                        data.pending_edits.clear();
                        data.cell_editor = None;
                        data.sync_cell_edits(cx);
                        this.show_toast(
                            ToastKind::Success,
                            format!("Saved {}", counted(saved, "row", "rows")),
                            cx,
                        );
                        this.refresh_table_for(session_id, tab_id, cx);
                    }
                    Err(error) => {
                        this.show_mutation_error_for(session_id, tab_id, error, None, window, cx);
                    }
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::OrderDirection;

    // Tokio database jobs use real threads; drain GPUI completions while
    // waiting for their notifications, with a bounded wall-clock deadline.
    fn wait_for_database_ui(
        cx: &mut gpui::VisualTestContext,
        app: &Entity<DbxApp>,
        predicate: impl Fn(&DbxApp) -> bool,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            // Backend tasks complete on the test thread, so their JoinHandle
            // wakes obey GPUI's deterministic scheduler contract.
            let runtime = cx.update(|_, cx| app.read(cx).runtime.clone());
            runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if cx.update(|_, cx| predicate(app.read(cx))) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "database UI completion timed out"
            );
        }
    }

    #[gpui::test]
    fn json_editor_validates_and_persists_the_complete_document(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app,cx) = cx.add_window_view(|window,cx| {
            let mut app=DbxApp::new(window,cx); app.vault_state=Some(VaultState::Unlocked);
            let engine=Arc::new(app.runtime.block_on(DatabaseEngine::connect(ConnectionConfig::new(DatabaseKind::SQLite,"sqlite::memory:"))).unwrap());
            app.runtime.block_on(engine.execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY, document JSON); INSERT INTO items VALUES(1, '{\"valid\":true}'),(2,NULL)")).unwrap();
            let result=app.runtime.block_on(engine.query("SELECT * FROM items",QueryOptions::default())).unwrap();
            let mut session=ConnectionSession::new(session_id,None,"JSON editor".into(),DatabaseKind::SQLite,None,window,cx);
            session.tables=vec![TableInfo::table("items",None)];
            let mut data=DataTab::new(session_id,tab_id,TableRef::new("items"),true,window,cx);
            data.table_columns=app.runtime.block_on(engine.describe_table(&data.table)).unwrap(); data.result_table=Some(data.table.clone());
            data.set_result(Some(result),&session.tables,cx);
            session.engine=Some(engine);session.secondary_tabs.push(SecondaryTab{id:tab_id,kind:SecondaryTabKind::Data(Box::new(data))});session.active_secondary_tab=Some(tab_id);session.pane=Pane::Data;
            app.sessions.push(session);app.active_session_id=Some(session_id);app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_cell_edit_for(session_id, tab_id, 1, 1, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .as_ref()
                        .unwrap()
                        .structured
                );
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    !app.data_tab(session_id, tab_id)
                        .unwrap()
                        .has_pending_edits(),
                    "Opening a SQL NULL JSON cell must not turn it into JSON null"
                );
            })
        });
        let document = serde_json::json!({"complete":"x".repeat(1200)}).to_string();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.begin_cell_edit_for(session_id, tab_id, 0, 1, window, cx);
                let cell = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap();
                assert!(cell.structured);
                let editor = cell.editor.clone();
                editor.update(cx, |editor, cx| editor.set_text("{invalid", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .is_some()
                );
                assert!(
                    !app.data_tab(session_id, tab_id)
                        .unwrap()
                        .has_pending_edits()
                );
                editor.update(cx, |editor, cx| editor.set_text(&document, cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .is_none()
                );
                app.save_pending_edits_for(session_id, tab_id, window, cx);
            })
        });
        wait_for_database_ui(cx, &app, |app| {
            app.data_tab(session_id, tab_id)
                .is_some_and(|data| !data.busy)
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let data = app.data_tab(session_id, tab_id).unwrap();
            assert!(data.error.is_none(), "{:?}", data.error);
            assert!(!data.has_pending_edits());
            let result = app
                .runtime
                .block_on(
                    app.session(session_id)
                        .unwrap()
                        .engine
                        .as_ref()
                        .unwrap()
                        .query("SELECT document FROM items", QueryOptions::default()),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&result.rows[0].values[0].to_string())
                    .unwrap(),
                serde_json::from_str::<serde_json::Value>(&document).unwrap()
            );
        });
    }

    #[gpui::test]
    fn saving_inline_edits_persists_and_server_sort_resets_paging(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.runtime = Arc::new(tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap());
            app.vault_state = Some(VaultState::Unlocked);
            let engine = Arc::new(app.runtime.block_on(DatabaseEngine::connect(ConnectionConfig::new(
                DatabaseKind::SQLite,
                format!("sqlite://{}?mode=rwc", directory.path().join("rows.sqlite").display()),
            ))).unwrap());
            app.runtime.block_on(engine.execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO items VALUES (1, 'original'), (2, 'second')")).unwrap();
            let result = app.runtime.block_on(engine.query("SELECT * FROM items ORDER BY id", QueryOptions::default())).unwrap();
            let columns = app.runtime.block_on(engine.describe_table(&TableRef::new("items"))).unwrap();
            let mut session = ConnectionSession::new(session_id, None, "Persistence test".into(), DatabaseKind::SQLite, None, window, cx);
            session.engine = Some(engine);
            session.tables = vec![TableInfo::table("items", None)];
            let mut data = DataTab::new(session_id, tab_id, TableRef::new("items"), true, window, cx);
            data.table_columns = columns;
            data.result_table = Some(data.table.clone());
            data.set_result(Some(result), &session.tables, cx);
            session.secondary_tabs.push(SecondaryTab { id: tab_id, kind: SecondaryTabKind::Data(Box::new(data)) });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Data;
            app.sessions.push(session);
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.on_data_grid_event(
                    session_id,
                    tab_id,
                    &TableEvent::DoubleClickedCell(0, 2),
                    window,
                    cx,
                );
                let editor = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone();
                editor.update(cx, |editor, cx| editor.set_text("saved", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                app.save_pending_edits_for(session_id, tab_id, window, cx);
            })
        });
        wait_for_database_ui(cx, &app, |app| {
            app.data_tab(session_id, tab_id)
                .is_some_and(|data| !data.busy && !data.has_pending_edits())
        });
        cx.update(|_, cx| {
            let app = app.read(cx);
            let rows = app
                .runtime
                .block_on(
                    app.session(session_id)
                        .unwrap()
                        .engine
                        .as_ref()
                        .unwrap()
                        .query("SELECT name FROM items WHERE id=1", QueryOptions::default()),
                )
                .unwrap();
            assert_eq!(rows.rows[0].values[0], CellValue::Text("saved".into()));
        });
        for direction in [
            Some(OrderDirection::Ascending),
            Some(OrderDirection::Descending),
            None,
        ] {
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.data_tab_mut(session_id, tab_id).unwrap().table_page = 3;
                    app.set_table_sort_for(
                        session_id,
                        tab_id,
                        direction.map(|direction| Order {
                            column: "id".into(),
                            direction,
                        }),
                        cx,
                    );
                })
            });
            wait_for_database_ui(cx, &app, |app| {
                app.data_tab(session_id, tab_id)
                    .is_some_and(|data| !data.busy)
            });
            cx.update(|_, cx| {
                let data = app.read(cx).data_tab(session_id, tab_id).unwrap();
                assert_eq!(data.table_page, 0);
                assert_eq!(
                    data.result.as_ref().unwrap().rows[0].values[0],
                    CellValue::Integer(if direction == Some(OrderDirection::Descending) {
                        2
                    } else {
                        1
                    })
                );
                assert!(data.error.is_none(), "{:?}", data.error);
            });
        }
    }

    #[gpui::test]
    fn inline_edits_stage_validate_block_navigation_and_discard(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.runtime = Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            );
            app.vault_state = Some(VaultState::Unlocked);
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Cell test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            session.tables = vec![TableInfo::table("items", None)];
            let mut id = ColumnInfo::result("id", 0, "INTEGER");
            id.primary_key = true;
            id.nullable = false;
            let columns = vec![id, ColumnInfo::result("name", 1, "TEXT")];
            let mut data =
                DataTab::new(session_id, tab_id, TableRef::new("items"), true, window, cx);
            data.table_columns = columns.clone();
            data.result_table = Some(data.table.clone());
            data.set_result(
                Some(QueryResult {
                    columns,
                    rows: vec![RowData::new(vec![
                        CellValue::Integer(1),
                        CellValue::Text("original".into()),
                    ])],
                    ..QueryResult::empty(None, 0)
                }),
                &session.tables,
                cx,
            );
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Data(Box::new(data)),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Data;
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.on_data_grid_event(
                    session_id,
                    tab_id,
                    &TableEvent::DoubleClickedCell(0, 2),
                    window,
                    cx,
                );
                let editor = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone();
                editor.update(cx, |editor, cx| editor.set_text("changed", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert_eq!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .pending_edits
                        .get(&(0, 1)),
                    Some(&CellValue::Text("changed".into()))
                );
                assert!(app.has_pending_lock_work());
                app.close_session(session_id, cx);
                assert!(app.session(session_id).is_some());
                app.set_table_sort_for(
                    session_id,
                    tab_id,
                    Some(Order {
                        column: "id".into(),
                        direction: OrderDirection::Descending,
                    }),
                    cx,
                );
                assert!(app.data_tab(session_id, tab_id).unwrap().sort.is_none());
                app.request_close_secondary_tab_for(session_id, tab_id, window, cx);
                assert!(app.data_tab(session_id, tab_id).is_some());
                app.begin_cell_edit_for(session_id, tab_id, 0, 1, window, cx);
                assert_eq!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .as_ref()
                        .unwrap()
                        .editor
                        .read(cx)
                        .text(cx),
                    "changed"
                );
                app.cancel_cell_edit_for(session_id, tab_id, window, cx);
                app.begin_cell_edit_for(session_id, tab_id, 0, 0, window, cx);
                let editor = app
                    .data_tab(session_id, tab_id)
                    .unwrap()
                    .cell_editor
                    .as_ref()
                    .unwrap()
                    .editor
                    .clone();
                editor.update(cx, |editor, cx| editor.set_text("invalid integer", cx));
                app.commit_cell_edit_for(session_id, tab_id, None, window, cx);
                assert!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .cell_editor
                        .is_some()
                );
                app.cancel_cell_edit_for(session_id, tab_id, window, cx);
                app.discard_pending_edits_for(session_id, tab_id, cx);
                assert!(
                    !app.data_tab(session_id, tab_id)
                        .unwrap()
                        .has_unsaved_cell_work()
                );
                assert_eq!(
                    app.data_tab(session_id, tab_id)
                        .unwrap()
                        .result
                        .as_ref()
                        .unwrap()
                        .rows[0]
                        .values[1],
                    CellValue::Text("original".into())
                );
            })
        });
    }
}
