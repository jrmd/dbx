//! Workbench query actions.
use super::*;

impl DbxApp {
    pub(super) fn request_run_query_for(
        &mut self,
        session_id: SessionId,
        run_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((kind, query_editor)) = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some((session.kind, query.query_editor.clone()))
        }) else {
            return;
        };
        if !kind.is_sql() {
            // Redis is intentionally line-oriented: a multiline document is
            // not a Redis pipeline, and flattening it into one command would
            // silently change its meaning. The only execution unit is the
            // selection or current line.
            self.run_query_for_execution(session_id, false, cx);
            return;
        }
        let text = query_editor.read(cx).text(cx);
        let scope = if run_all {
            editor::QueryExecutionScope::Document
        } else {
            editor::QueryExecutionScope::SelectionOrStatement
        };
        let range = if run_all {
            0..text.len()
        } else {
            query_editor.read(cx).execution_range(scope, cx)
        };
        let query = text[range].trim();
        if query.is_empty() {
            return;
        }
        if self.prompt_query_parameters_for(session_id, query, run_all, window, cx) {
            return;
        }
        let query = match self.bind_query_parameters_for(session_id, query) {
            Ok(query) => query,
            Err(error) => {
                self.show_toast(ToastKind::Error, error, cx);
                return;
            }
        };
        let Some(tab_id) = self
            .session(session_id)
            .and_then(|session| session.active_secondary_tab)
        else {
            return;
        };
        let (title, detail, confirm_label, tone) = match editor::sql_execution_kind(&query) {
            editor::SqlExecutionKind::Destructive => (
                "Run destructive query?",
                "This statement can permanently change or delete data.",
                "Run query",
                ConfirmationTone::Danger,
            ),
            _ if editor::sql_statement_count(&query) > 1 => (
                "Run multiple statements?",
                "Statements run in order without a transaction. If one fails, earlier changes stay.",
                "Run statements",
                ConfirmationTone::Warning,
            ),
            _ => {
                if let Some(tab) = self.active_query_tab_mut(session_id) {
                    tab.execution_override = Some(query);
                }
                self.run_query_for_execution(session_id, run_all, cx);
                return;
            }
        };
        let return_focus = Some(query_editor.read(cx).focus_handle());
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: title.into(),
            detail: detail.into(),
            confirm_label,
            tone,
            action: ConfirmationAction::RunQuery {
                session_id,
                tab_id,
                run_all,
                query,
            },
            focus: focus.clone(),
            return_focus,
            sql: None,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    /// Execute the selected text or current statement. `run_all` is reserved
    /// for the explicit whole-document action; it intentionally ignores an
    /// editor selection.
    pub(super) fn run_query_for_execution(
        &mut self,
        session_id: SessionId,
        run_all: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, tab_id, kind, database, history_connection, query_editor, busy)) =
            self.session(session_id).and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Query(query_tab) = &tab.kind else {
                    return None;
                };
                Some((
                    session.engine.clone(),
                    tab_id,
                    session.kind,
                    session.current_database.clone(),
                    query_history_connection(session),
                    query_tab.query_editor.clone(),
                    query_tab.busy,
                ))
            })
        else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if busy {
            return;
        }
        let override_query = self
            .session_mut(session_id)
            .and_then(|session| {
                session
                    .secondary_tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
            })
            .and_then(|tab| match &mut tab.kind {
                SecondaryTabKind::Query(query) => query.execution_override.take(),
                _ => None,
            });
        let full_query = override_query
            .clone()
            .unwrap_or_else(|| query_editor.read(cx).text(cx));
        let scope = if run_all {
            editor::QueryExecutionScope::Document
        } else if kind.is_sql() {
            editor::QueryExecutionScope::SelectionOrStatement
        } else if kind == DatabaseKind::Redis {
            editor::QueryExecutionScope::SelectionOrCurrentLine
        } else {
            editor::QueryExecutionScope::Document
        };
        let range = if run_all || override_query.is_some() {
            0..full_query.len()
        } else {
            query_editor.read(cx).execution_range(scope, cx)
        };
        let selected_query = &full_query[range.clone()];
        let executed_leading_whitespace = selected_query.len() - selected_query.trim_start().len();
        let query = selected_query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        let query = if kind.is_sql() && override_query.is_none() {
            match self.bind_query_parameters_for(session_id, &query) {
                Ok(query) => query,
                Err(error) => {
                    self.show_toast(ToastKind::Error, error, cx);
                    return;
                }
            }
        } else {
            query
        };
        let prepared_parameters = self
            .active_query_tab_mut(session_id)
            .and_then(|tab| tab.prepared_parameters.take());
        let may_change_schema = kind.is_sql() && editor::sql_may_change_schema(&query);
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return;
        };
        query_tab.busy = true;
        query_tab.results_stale = query_tab.result.is_some();
        query_tab.error = None;
        query_tab.status = "Running query…".into();
        query_tab.request_generation = query_tab.request_generation.saturating_add(1);
        let generation = query_tab.request_generation;
        let query_revision = query_tab.query_revision;
        query_tab.executed_database = database.clone();
        cx.notify();
        // The executed statement moves into the blocking task; the original
        // stays behind so failures can locate the offending token in it.
        let executed_query = query.clone();
        let console = query_tab
            .console
            .get_or_insert_with(|| Arc::new(QuerySession::new(engine)))
            .clone();
        let cancellation = QueryCancellation::default();
        query_tab.cancellation = Some(cancellation.clone());
        let timeout_secs = query_tab.timeout_secs;
        let task = runtime.spawn(async move {
            if let Some(statements) = prepared_parameters {
                console
                    .run_prepared(
                        statements,
                        QueryOptions::default(),
                        std::time::Duration::from_secs(timeout_secs),
                        cancellation,
                    )
                    .await
            } else {
                console
                    .run(
                        &executed_query,
                        QueryOptions::default(),
                        std::time::Duration::from_secs(timeout_secs),
                        cancellation,
                    )
                    .await
            }
        });
        query_tab.abort_handle.replace(task.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                let (history_outcome, refresh_schema) = {
                    let Some(session) = this.session_mut(session_id) else {
                        return;
                    };
                    let Some(tab) = session
                        .secondary_tabs
                        .iter_mut()
                        .find(|tab| tab.id == tab_id)
                    else {
                        return;
                    };
                    let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
                        return;
                    };
                    if generation != query_tab.request_generation {
                        return;
                    }
                    query_tab.busy = false;
                    query_tab.abort_handle.clear();
                    query_tab.cancellation = None;
                    match result {
                        Ok(Ok(script)) => {
                            query_tab.in_transaction = script.in_transaction;
                            query_tab.statement_results = script.statements;
                            query_tab.active_result =
                                query_tab.statement_results.len().saturating_sub(1);
                            let mut result = query_tab
                                .statement_results
                                .last()
                                .map(|item| item.result.clone());
                            let error = query_tab
                                .statement_results
                                .last()
                                .and_then(|item| item.error.clone());
                            if std::mem::take(&mut query_tab.plan_pending)
                                && error.is_none()
                                && let Some(raw) = &result
                            {
                                let plan = dbx_core::format_execution_plan(kind, raw);
                                query_tab.plan = Some(plan.clone());
                                result = Some(plan);
                            }
                            query_tab.status =
                                result.as_ref().map(query_result_status).unwrap_or_default();
                            let outcome = if let Some(error) = &error {
                                QueryHistoryOutcome::failure(error.clone())
                            } else {
                                QueryHistoryOutcome::success(query_tab.status.clone())
                            };
                            query_tab.set_result(result, cx);
                            query_tab.results_stale = false;
                            query_tab.error = error;
                            query_tab.error_highlight = None;
                            (outcome, may_change_schema && !query_tab.in_transaction)
                        }
                        Ok(Err(error)) => {
                            query_tab.plan_pending = false;
                            query_tab.in_transaction = false;
                            let message = error.to_string();
                            // Positions reported against the trimmed statement
                            // shift by the trimmed leading whitespace.
                            let lead = range.start + executed_leading_whitespace;
                            query_tab.error_highlight =
                                if query_tab.query_revision == query_revision {
                                    editor::sql_error_range(&message, &query)
                                        .map(|range| range.start + lead..range.end + lead)
                                } else {
                                    None
                                };
                            query_tab.error = Some(message.clone());
                            query_tab.results_stale = query_tab.result.is_some();
                            (QueryHistoryOutcome::failure(message), false)
                        }
                        Err(error) => {
                            let message = format!("Query task stopped unexpectedly: {error}");
                            query_tab.error = Some(message.clone());
                            query_tab.results_stale = query_tab.result.is_some();
                            (QueryHistoryOutcome::failure(message), false)
                        }
                    }
                };
                this.record_query_history(
                    history_connection.clone(),
                    query.clone(),
                    history_outcome,
                    cx,
                );
                if refresh_schema {
                    if let Some(session) = this.session_mut(session_id) {
                        for tab in &mut session.secondary_tabs {
                            if let SecondaryTabKind::Diagram(diagram) = &mut tab.kind {
                                diagram.stale = diagram.document.is_some();
                            }
                        }
                    }
                    // Refresh Explorer first; its guarded completion then
                    // rebuilds any open diagram from the same catalogue.
                    this.refresh_tables_for(session_id, cx);
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn run_console_command_for(
        &mut self,
        session_id: SessionId,
        command: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab_id) = session.active_secondary_tab
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
            && let SecondaryTabKind::Query(query) = &mut tab.kind
            && !query.busy
        {
            query.execution_override = Some(command.to_owned());
            self.run_query_for_execution(session_id, true, cx);
        }
    }

    pub(super) fn select_statement_result_for(
        &mut self,
        session_id: SessionId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab_id) = session.active_secondary_tab
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
            && let SecondaryTabKind::Query(query) = &mut tab.kind
            && let Some(item) = query.statement_results.get(index).cloned()
        {
            query.active_result = index;
            query.status = query_result_status(&item.result);
            query.error = item.error;
            query.set_result(Some(item.result), cx);
            cx.notify();
        }
    }

    pub(super) fn copy_query_selection_action(
        &mut self,
        _: &CopyQuerySelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session_id) = self.active_session_id() {
            self.copy_query_selection_for(session_id, cx);
        }
    }

    pub(super) fn cancel_query_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        // Escape belongs to an open completion menu first; the keybinding
        // reaches here before the menu's own key handler can dismiss it.
        if let Some(menu) = self.query_completion_for(session_id, cx) {
            if let Some(session) = self.session_mut(session_id)
                && let Some(tab_id) = session.active_secondary_tab
                && let Some(tab) = session
                    .secondary_tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id)
                && let SecondaryTabKind::Query(query_tab) = &mut tab.kind
            {
                query_tab.completion_dismissed_signature = Some(menu.signature);
            }
            cx.notify();
            return;
        }
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(tab) = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return;
        };
        let SecondaryTabKind::Query(query_tab) = &mut tab.kind else {
            return;
        };
        if !query_tab.busy {
            return;
        }
        if let Some(cancellation) = &query_tab.cancellation {
            cancellation.cancel();
        }
        query_tab.status = "Stopping query; verifying server cancellation…".into();
        cx.notify();
    }

    pub(super) fn query_result_text_for(
        &self,
        session_id: SessionId,
        format: QueryResultExportFormat,
        cx: &App,
    ) -> Option<String> {
        let query = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some(query)
        })?;
        let delegate = query.result_grid.read(cx).delegate();
        match format {
            QueryResultExportFormat::Tsv => delegate.result_as_tsv(),
            QueryResultExportFormat::Csv => delegate.result_as_csv(),
            QueryResultExportFormat::Json => delegate.result_as_json(),
            QueryResultExportFormat::Insert => {
                let target = query.export_target.read(cx).text(cx);
                if target.trim().is_empty() {
                    return None;
                }
                let kind = self.session(session_id)?.kind;
                delegate.result_as_insert(kind, &TableRef::new(target.trim()))
            }
        }
    }

    /// Copy the most specific active selection: cell, then row, then column.
    /// Column zero is DBX's synthetic row-number column and has no database value.
    pub(super) fn copy_query_selection_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some((text, label)) = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            let grid = query.result_grid.read(cx);
            let delegate = grid.delegate();
            match query.result_selection {
                QueryResultSelection::Cell => {
                    let (row, column) = grid.selected_cell()?;
                    let text = if column == 0 {
                        Some((row + 1).to_string())
                    } else {
                        delegate.cell_as_plain_text(row, delegate.result_column(column)?)
                    }?;
                    Some((text, "cell"))
                }
                QueryResultSelection::Row => grid
                    .selected_row()
                    .and_then(|row| delegate.row_as_tsv(row))
                    .map(|text| (text, "row")),
                QueryResultSelection::Column => grid.selected_col().and_then(|column| {
                    (column > 0)
                        .then(|| delegate.column_as_tsv(delegate.result_column(column)?))
                        .flatten()
                        .map(|text| (text, "column"))
                }),
                QueryResultSelection::None => None,
            }
        }) else {
            self.show_toast(
                ToastKind::Info,
                "Select a result cell, row, or column to copy",
                cx,
            );
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast(ToastKind::Success, format!("Copied {label}"), cx);
    }

    pub(super) fn copy_query_result_for(
        &mut self,
        session_id: SessionId,
        format: QueryResultExportFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.query_result_text_for(session_id, format, cx) else {
            return;
        };
        let (rows, truncated) = self
            .active_query_tab(session_id)
            .and_then(|query| query.result.as_ref())
            .map(|result| (result.rows.len(), result.truncated))
            .unwrap_or_default();
        if truncated {
            self.show_toast(
                ToastKind::Info,
                format!(
                    "Only the {rows} loaded rows are included. The query result was truncated."
                ),
                cx,
            );
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.show_toast(
            ToastKind::Success,
            format!(
                "Copied result as {}",
                format.extension().to_ascii_uppercase()
            ),
            cx,
        );
    }

    pub(super) fn export_full_query_for(
        &mut self,
        session_id: SessionId,
        format: dbx_core::QueryExportFormat,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, sql, busy, in_transaction)) =
            self.session(session_id).and_then(|session| {
                let query = self.active_query_tab(session_id)?;
                Some((
                    session.engine.clone()?,
                    query.query_text.read(cx).clone(),
                    session.busy || query.busy,
                    query.in_transaction,
                ))
            })
        else {
            return;
        };
        if busy {
            return;
        }
        if in_transaction {
            self.show_toast(
                ToastKind::Info,
                "Commit or roll back the query transaction before exporting a fresh snapshot",
                cx,
            );
            return;
        }
        let extension = match format {
            dbx_core::QueryExportFormat::Csv => "csv",
            dbx_core::QueryExportFormat::Tsv => "tsv",
            dbx_core::QueryExportFormat::JsonLines => "jsonl",
        };
        let directory = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
        let receiver = cx.prompt_for_new_path(&directory, Some(&format!("full-query.{extension}")));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = receiver.await else {
                return Ok::<(), anyhow::Error>(());
            };
            let task = this.update(cx, |this, cx| {
                let session = this.session_mut(session_id)?;
                if session.busy {
                    return None;
                }
                session.busy = true;
                session.request_generation += 1;
                let generation = session.request_generation;
                session.status = "Exporting full query from a fresh read-only snapshot…".into();
                let control = this.start_transfer_progress(session_id, generation, cx);
                let task = this.runtime.spawn(async move {
                    dbx_core::with_transfer_control(
                        control,
                        dbx_core::export_query(&engine, &sql, &path, format),
                    )
                    .await
                });
                if let Some(session) = this.session_mut(session_id) {
                    session.track_background_task(&task);
                }
                Some((task, generation))
            })?;
            if let Some((task, generation)) = task {
                let result = task.await?;
                this.update(cx, |this, cx| {
                    let Some(session) = this.session_mut(session_id) else {
                        return;
                    };
                    if session.request_generation != generation {
                        return;
                    }
                    session.busy = false;
                    session.transfer_control = None;
                    match result {
                        Ok(rows) => this.show_toast(
                            ToastKind::Success,
                            format!("Exported all {rows} query rows"),
                            cx,
                        ),
                        Err(error) => this.show_toast(ToastKind::Error, error.to_string(), cx),
                    }
                    cx.notify();
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn export_query_result_for(
        &mut self,
        session_id: SessionId,
        format: QueryResultExportFormat,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self.query_result_text_for(session_id, format, cx) else {
            return;
        };
        let (rows, truncated) = self
            .active_query_tab(session_id)
            .and_then(|query| query.result.as_ref())
            .map(|result| (result.rows.len(), result.truncated))
            .unwrap_or_default();
        if truncated {
            self.show_toast(
                ToastKind::Info,
                format!(
                    "Only the {rows} loaded rows are included. The query result was truncated."
                ),
                cx,
            );
        }
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested = format!("query-result.{}", format.extension());
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(path))) => {
                    let destination = path.display().to_string();
                    let result = runtime
                        .spawn_blocking(move || {
                            dbx_core::write_atomic_export(&path, text.as_bytes())
                        })
                        .await;
                    this.update(cx, |this, cx| {
                        let (kind, message) = match result {
                            Ok(Ok(())) => (
                                ToastKind::Success,
                                format!(
                                    "Exported {rows} loaded rows to {destination}{}",
                                    if truncated { " (partial result)" } else { "" }
                                ),
                            ),
                            Ok(Err(error)) => (
                                ToastKind::Error,
                                format!("Could not export result: {error}"),
                            ),
                            Err(error) => (
                                ToastKind::Error,
                                format!("Result export task stopped: {error}"),
                            ),
                        };
                        this.show_toast(kind, message, cx);
                    })?;
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(
                            ToastKind::Error,
                            format!("Could not open the save dialog: {error}"),
                            cx,
                        );
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(
                            ToastKind::Error,
                            format!("Save dialog closed unexpectedly: {error}"),
                            cx,
                        );
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn copy_query_error_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let error = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            query.error.clone()
        });
        if let Some(error) = error {
            cx.write_to_clipboard(ClipboardItem::new_string(error));
            self.show_toast(ToastKind::Success, "Copied query error", cx);
        }
    }

    pub(super) fn focus_query_error_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            query
                .error_highlight
                .as_ref()
                .map(|range| (query.query_editor.clone(), range.start))
        });
        if let Some((editor, offset)) = editor {
            let focus = editor.read(cx).focus_handle();
            editor.update(cx, |editor, cx| editor.move_cursor_to(offset, cx));
            focus.focus(window, cx);
        }
    }

    /// Pretty-print the active query tab's SQL in place, keeping the caret
    /// anchored to the token it sat on. Redis tabs have nothing to format.
    pub(super) fn format_query_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(query_editor) = self.session(session_id).and_then(|session| {
            if !session.kind.is_sql() {
                return None;
            }
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some(query.query_editor.clone())
        }) else {
            return;
        };

        let (text, cursor, focus_handle) = query_editor.update(cx, |editor, cx| {
            (
                editor.text(cx),
                editor.cursor_offset(),
                editor.focus_handle(),
            )
        });
        let (formatted, mapped_cursor) = editor::format_sql_at_cursor(&text, cursor);
        if formatted != text {
            let length = text.len();
            query_editor.update(cx, |editor, cx| {
                editor.replace_range(0..length, formatted.as_str(), cx);
                editor.move_cursor_to(mapped_cursor, cx);
            });
        }
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab_id) = session.active_secondary_tab
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id)
            && let SecondaryTabKind::Query(query_tab) = &mut tab.kind
        {
            query_tab.completion_signature = None;
            query_tab.completion_dismissed_signature = None;
            query_tab.completion_index = 0;
        }
        focus_handle.focus(window, cx);
        cx.notify();
    }
}
