use super::*;

impl DbxApp {
    pub(super) fn stop_query_watch(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(query) = self.active_query_tab_mut(session_id) {
            let watching = query.watch_interval.take().is_some();
            query.watch_generation = query.watch_generation.wrapping_add(1);
            query.watch_read_only_pending = false;
            if watching
                && query.busy
                && let Some(cancellation) = &query.cancellation
            {
                cancellation.cancel();
            }
        }
        cx.notify();
    }

    pub(super) fn start_query_watch(
        &mut self,
        session_id: SessionId,
        interval: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy {
            self.show_toast(
                ToastKind::Info,
                "Finish the current connection task before starting a watch",
                cx,
            );
            return;
        }
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let database = session.current_database.clone();
        let Some(query) = self
            .active_query_tab(session_id)
            .filter(|query| !query.busy && !query.in_transaction)
        else {
            self.show_toast(
                ToastKind::Info,
                "Finish the current query or transaction before starting a watch",
                cx,
            );
            return;
        };
        let editor = query.query_editor.read(cx);
        let text = editor.text(cx);
        let range = editor.execution_range(editor::QueryExecutionScope::SelectionOrStatement, cx);
        let sql = text[range].to_string();
        if !QuerySession::supports_isolated_reads(&engine) {
            self.show_toast(
                ToastKind::Info,
                "This connector does not provide an isolated read-only session",
                cx,
            );
            return;
        }
        if let Err(error) = dbx_core::execution_plan_query(session.kind, &sql) {
            self.show_toast(ToastKind::Error, error.to_string(), cx);
            return;
        }
        let revision = query.query_revision;
        let interval = interval.clamp(5, 3600);
        let query = self.active_query_tab_mut(session_id).unwrap();
        query.watch_generation = query.watch_generation.wrapping_add(1);
        let generation = query.watch_generation;
        query.watch_interval = Some(interval);
        query.watch_read_only_pending = true;
        query.plan_pending = false;
        query.execution_override = Some(query_actions::ExecutionSource::detached(&sql));
        self.run_query_for_execution(session_id, true, cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(interval))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        let same_connection = this.session(session_id).is_some_and(|session| {
                            session.current_database == database
                                && session
                                    .engine
                                    .as_ref()
                                    .is_some_and(|current| Arc::ptr_eq(current, &engine))
                        });
                        let connection_busy =
                            this.session(session_id).is_none_or(|session| session.busy);
                        let visible = this.active_session_id() == Some(session_id)
                            && this.session(session_id).is_some_and(|session| {
                                session.active_secondary_tab == Some(tab_id)
                            });
                        let Some(query) = this.query_tab_mut(session_id, tab_id) else {
                            return false;
                        };
                        if query.watch_generation != generation
                            || query.watch_interval != Some(interval)
                        {
                            return false;
                        }
                        if !same_connection
                            || query.query_revision != revision
                            || query.in_transaction
                        {
                            query.watch_interval = None;
                            cx.notify();
                            return false;
                        }
                        if !visible || connection_busy || query.busy {
                            return true;
                        }
                        query.watch_read_only_pending = true;
                        query.plan_pending = false;
                        query.execution_override =
                            Some(query_actions::ExecutionSource::detached(&sql));
                        this.run_query_for_execution(session_id, true, cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn show_loaded_column_statistics(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = self
            .active_query_tab(session_id)
            .and_then(|query| query.result.clone())
        else {
            return;
        };
        self.show_result_column_statistics(session_id, result, window, cx);
    }

    pub(super) fn show_loaded_table_column_statistics(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = self
            .session(session_id)
            .and_then(|session| {
                session
                    .active_data_tab_id()
                    .and_then(|id| session.data_tab(id))
            })
            .and_then(|data| data.result.clone())
        else {
            return;
        };
        self.show_result_column_statistics(session_id, result, window, cx);
    }

    /// An explicit user action: aggregate the whole table, independently of
    /// the loaded page/filter, through the normal cancelable query task.
    pub(super) fn show_full_table_column_statistics(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if !session
            .engine
            .as_ref()
            .is_some_and(|engine| QuerySession::supports_isolated_reads(engine))
        {
            return;
        }
        if session.busy {
            self.show_toast(
                ToastKind::Info,
                "Finish the current connection task before scanning table statistics",
                cx,
            );
            return;
        }
        let Some(data) = session.active_data_tab() else {
            return;
        };
        let columns = data.table_columns.clone();
        let sql = match dbx_core::table_column_statistics_query(session.kind, &data.table, &columns)
        {
            Ok(sql) => sql,
            Err(error) => {
                self.show_toast(ToastKind::Error, error.to_string(), cx);
                return;
            }
        };
        self.open_saved_query_for(session_id, crate::workspace::SavedQuery {
            name: "Column statistics · full table".into(),
            sql: format!("-- Full-table server statistics: scans all visible rows, ignoring the loaded page and filters.\n-- Distinct and extrema use source types and database collation; unsupported types are labeled.\n{sql}"),
        }, window, cx);
        if let Some(query) = self.active_query_tab_mut(session_id) {
            query.full_table_statistics_pending = Some(columns);
            query.watch_read_only_pending = true;
        }
        self.run_query_for_execution(session_id, true, cx);
    }

    fn show_result_column_statistics(
        &mut self,
        session_id: SessionId,
        result: Arc<QueryResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = result.rows.len();
        let statistics = dbx_core::loaded_column_statistics(&result);
        self.open_saved_query_for(session_id, crate::workspace::SavedQuery {
            name: "Column statistics · loaded rows".into(),
            sql: format!("-- Column statistics for {count} loaded result rows only.\n-- No full-table scan was run. Distinct counts use client values, not database collation.\n-- Numeric summaries (~) use approximate floating-point arithmetic.\n"),
        }, window, cx);
        if let Some(query) = self.active_query_tab_mut(session_id) {
            query.set_result(Some(statistics), cx);
            query.status = format!(
                "Statistics for {count} loaded rows only{}",
                if result.truncated {
                    " · source result was limited"
                } else {
                    ""
                }
            );
            query.results_stale = false;
        }
        cx.notify();
    }

    pub(super) fn compare_connection_schemas(
        &mut self,
        target_id: SessionId,
        source_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, source)) = self.session(target_id).zip(self.session(source_id)) else {
            return;
        };
        if target_id == source_id || target.kind.dialect() != source.kind.dialect() {
            return;
        }
        if [target, source].iter().any(|session| session.busy || session.secondary_tabs.iter().any(|tab| matches!(&tab.kind, SecondaryTabKind::Query(query) if query.in_transaction || query.busy))) {
            self.show_toast(ToastKind::Info, "Finish running queries and open transactions on both connections before comparing schemas", cx);
            return;
        }
        let Some((target_engine, source_engine)) = target.engine.clone().zip(source.engine.clone())
        else {
            return;
        };
        let kind = target.kind;
        let (target_database, source_database) = (
            target.current_database.clone(),
            source.current_database.clone(),
        );
        let (target_generation, source_generation) =
            (target.request_generation, source.request_generation);
        let target_name = target.name.replace(['\r', '\n'], " ");
        let source_name = source.name.replace(['\r', '\n'], " ");
        let expected_target = target_engine.clone();
        let expected_source = source_engine.clone();
        let task = self.runtime.spawn(async move {
            tokio::time::timeout(std::time::Duration::from_secs(60), async move {
                let (current, desired) = tokio::try_join!(
                    target_engine.relational_schema(),
                    source_engine.relational_schema()
                )?;
                dbx_core::schema_migration(kind, &current, &desired)
            })
            .await
            .map_err(|_| dbx_core::DbxError::Query("Schema comparison timed out".into()))?
        });
        if let Some(session) = self.session_mut(target_id) {
            session.track_background_task(&task);
        }
        if let Some(session) = self.session_mut(source_id) {
            session.track_background_task(&task);
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                let target_current = this.session(target_id).is_some_and(|session| session.request_generation == target_generation && session.current_database == target_database && session.engine.as_ref().is_some_and(|engine| Arc::ptr_eq(engine, &expected_target)));
                let source_current = this.session(source_id).is_some_and(|session| session.request_generation == source_generation && session.current_database == source_database && session.engine.as_ref().is_some_and(|engine| Arc::ptr_eq(engine, &expected_source)));
                if !target_current || !source_current { return; }
                match result {
                    Ok(Ok(draft)) => {
                        let comments = draft.changes.iter().map(|change| format!("-- {}\n", change.replace(['\r', '\n'], " "))).chain(draft.warnings.iter().map(|warning| format!("-- REVIEW: {}\n", warning.replace(['\r', '\n'], " ")))).collect::<String>();
                        this.active_session_id = Some(target_id);
                        this.open_saved_query_for(target_id, crate::workspace::SavedQuery {
                            name: format!("Schema comparison: {source_name}"),
                            sql: format!("-- TARGET: {target_name}\n-- DESIRED SCHEMA FROM: {source_name}\n-- Review this migration draft before running. No changes have been applied.\n-- {} changes; {} require review.\n{comments}\n{}", draft.changes.len(), draft.warnings.len(), draft.sql),
                        }, window, cx);
                    }
                    Ok(Err(error)) => this.show_toast(ToastKind::Error, error.to_string(), cx),
                    Err(_) => this.show_toast(ToastKind::Error, "Schema comparison was cancelled", cx),
                }
            })?;
            Ok::<(), anyhow::Error>(())
        }).detach();
    }

    pub(super) fn request_analyze_query_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        let Some(tab_id) = session.active_secondary_tab else {
            return;
        };
        let Some(query) = self
            .active_query_tab(session_id)
            .filter(|query| !query.busy)
        else {
            return;
        };
        let editor = query.query_editor.read(cx);
        let text = editor.text(cx);
        let range = editor.execution_range(editor::QueryExecutionScope::SelectionOrStatement, cx);
        let sql = text[range].to_string();
        if let Err(error) = dbx_core::execution_plan_analyze_query(session.kind, &sql) {
            self.show_toast(ToastKind::Error, error.to_string(), cx);
            return;
        }
        let return_focus = Some(editor.focus_handle());
        let focus = cx.focus_handle();
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: "Execute EXPLAIN ANALYZE?".into(),
            detail: "ANALYZE executes this read query to measure its plan. DBX uses a separate native connection in a server-enforced read-only transaction and rolls it back; your query tab’s transaction stays open. External effects of database functions cannot be undone by rollback.".into(),
            confirm_label: "Execute ANALYZE",
            tone: ConfirmationTone::Warning,
            action: ConfirmationAction::AnalyzeQuery { session_id, tab_id, sql: sql.clone() },
            focus: focus.clone(), return_focus, sql: Some(sql),
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn analyze_query_for(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        sql: String,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.active_secondary_tab != Some(tab_id) {
            return;
        }
        let Some(query) = self
            .active_query_tab_mut(session_id)
            .filter(|query| !query.busy)
        else {
            return;
        };
        query.plan_pending = true;
        query.plan_analyze_pending = true;
        query.execution_override = Some(query_actions::ExecutionSource::detached(&sql));
        self.run_query_for_execution(session_id, true, cx);
    }

    pub(super) fn explain_query_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some((kind, editor)) = self.session(session_id).and_then(|session| {
            let query = session
                .secondary_tabs
                .iter()
                .find(|tab| Some(tab.id) == session.active_secondary_tab)?;
            match &query.kind {
                SecondaryTabKind::Query(query) if !query.busy => {
                    Some((session.kind, query.query_editor.clone()))
                }
                _ => None,
            }
        }) else {
            return;
        };
        let sql = editor.read(cx).text(cx);
        let range = editor
            .read(cx)
            .execution_range(editor::QueryExecutionScope::SelectionOrStatement, cx);
        match dbx_core::execution_plan_query(kind, &sql[range]) {
            Ok(command) => {
                if let Some(query) = self.active_query_mut_for(session_id) {
                    query.plan_pending = true;
                }
                self.run_console_command_for(session_id, &command, cx);
            }
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
    }
    fn active_query_mut_for(&mut self, session_id: SessionId) -> Option<&mut QueryTab> {
        let session = self.session_mut(session_id)?;
        let tab = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| Some(tab.id) == session.active_secondary_tab)?;
        match &mut tab.kind {
            SecondaryTabKind::Query(query) => Some(query),
            _ => None,
        }
    }
    pub(super) fn set_query_timeout_for(
        &mut self,
        session_id: SessionId,
        seconds: u64,
        cx: &mut Context<Self>,
    ) {
        if let Some(query) = self.active_query_mut_for(session_id) {
            query.timeout_secs = seconds.clamp(1, 3600);
        }
        cx.notify();
    }
    pub(super) fn pin_plan_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(query) = self.active_query_mut_for(session_id)
            && let Some(plan) = query.plan.clone()
        {
            query.plan_baseline = Some(plan);
            self.show_toast(
                ToastKind::Success,
                "Plan captured; explain another query to compare",
                cx,
            );
        } else {
            self.show_toast(ToastKind::Info, "Explain a query first", cx);
        }
    }
    pub(super) fn compare_plan_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(query) = self.active_query_mut_for(session_id)
            && let (Some(before), Some(after)) = (&query.plan_baseline, &query.plan)
        {
            let result = dbx_core::compare_execution_plans(before, after);
            query.set_result(Some(result), cx);
            query.plan_tree = false;
            query.status = "Plan comparison by step · estimated costs; actual measurements only where ANALYZE supplied them".into();
            query.results_stale = false;
            cx.notify();
        } else {
            self.show_toast(
                ToastKind::Info,
                "Capture a plan, then explain a second query",
                cx,
            );
        }
    }
    pub(super) fn open_monitor_for(
        &mut self,
        session_id: SessionId,
        monitor: dbx_core::Monitor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.session(session_id).map(|session| session.kind) else {
            return;
        };
        match dbx_core::monitor_query(kind, monitor) {
            Ok(sql) => {
                let name = match monitor {
                    dbx_core::Monitor::Sessions => "Server sessions",
                    dbx_core::Monitor::Locks => "Lock waits",
                };
                self.open_saved_query_for(
                    session_id,
                    crate::workspace::SavedQuery {
                        name: name.into(),
                        sql,
                    },
                    window,
                    cx,
                );
                self.run_query_for_execution(session_id, true, cx);
            }
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
    }
    pub(super) fn inspect_schema_for(
        &mut self,
        session_id: SessionId,
        capture: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy
            || session.secondary_tabs.iter().any(
                |tab| matches!(&tab.kind, SecondaryTabKind::Query(query) if query.in_transaction),
            )
        {
            self.show_toast(
                ToastKind::Info,
                "Finish open transactions before inspecting the schema",
                cx,
            );
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let baseline = session.schema_baseline.clone();
        let generation = session.request_generation;
        let expected_database = session.current_database.clone();
        if !capture && baseline.is_none() {
            self.show_toast(ToastKind::Info, "Capture a schema baseline first", cx);
            return;
        }
        let kind = session.kind;
        let runtime = self.runtime.clone();
        let task = runtime.spawn(async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(60),
                engine.relational_schema(),
            )
            .await
            .map_err(|_| dbx_core::DbxError::Query("Schema inspection timed out".into()))?
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await?;
            this.update_in(cx, |this, window, cx| {
                if !this.session(session_id).is_some_and(|session|
                    session.request_generation == generation && session.current_database == expected_database
                ) { return; }
                match result {
                    Ok(schema) if capture => {
                        if let Some(session) = this.session_mut(session_id) { session.schema_baseline = Some(schema); }
                        this.persist_query_workspace_for(session_id, cx);
                        this.show_toast(ToastKind::Success, "Schema baseline captured", cx);
                    }
                    Ok(schema) if baseline.as_ref().is_some_and(|baseline| baseline.database != schema.database) => {
                        this.show_toast(ToastKind::Error, "Schema baseline belongs to another database; capture a baseline for this database first", cx);
                    }
                    Ok(schema) => match dbx_core::schema_migration(kind, baseline.as_ref().unwrap(), &schema) {
                        Ok(draft) => {
                            let changes = draft.changes.iter().map(|change| format!("-- {}\n", change.replace(['\r', '\n'], " "))).collect::<String>();
                            let warnings = draft.warnings.iter().map(|warning| format!("-- REVIEW: {}\n", warning.replace(['\r', '\n'], " "))).collect::<String>();
                            this.open_saved_query_for(session_id, crate::workspace::SavedQuery { name: "Schema migration draft".into(), sql: format!("-- {} changes; {} require review\n{changes}{warnings}\n{}", draft.changes.len(), draft.warnings.len(), draft.sql) }, window, cx);
                        }
                        Err(error) => this.show_toast(ToastKind::Error, error.to_string(), cx),
                    },
                    Err(error) => this.show_toast(ToastKind::Error, error.to_string(), cx),
                }
            })?;
            Ok::<(), anyhow::Error>(())
        }).detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn connection_schema_comparison_drafts_source_changes_on_target_without_applying(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let target_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.runtime = Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            );
            app.vault_state = Some(VaultState::Unlocked);
            app.profile_store = None;
            app.workspace_store = None;
            app.sessions.clear();
            for (id, name, sql) in [
                (target_id, "target", "CREATE TABLE items(id INTEGER)"),
                (
                    source_id,
                    "source",
                    "CREATE TABLE items(id INTEGER, desired_label TEXT)",
                ),
            ] {
                let engine = Arc::new(
                    app.runtime
                        .block_on(DatabaseEngine::connect(ConnectionConfig::new(
                            DatabaseKind::SQLite,
                            "sqlite::memory:",
                        )))
                        .unwrap(),
                );
                app.runtime.block_on(engine.execute_sql(sql)).unwrap();
                let mut session = ConnectionSession::new(
                    id,
                    None,
                    name.into(),
                    DatabaseKind::SQLite,
                    None,
                    window,
                    cx,
                );
                session.engine = Some(engine);
                app.sessions.push(session);
            }
            app.active_session_id = Some(target_id);
            app.compare_connection_schemas(target_id, source_id, window, cx);
            app
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let runtime = app.read_with(cx, |app, _| app.runtime.clone());
            runtime.block_on(async {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            });
            cx.run_until_parked();
            if app.read_with(cx, |app, _| app.active_query_tab(target_id).is_some()) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "schema comparison completion timed out"
            );
        }
        app.read_with(cx, |app, cx| {
            assert_eq!(app.active_session_id(), Some(target_id));
            let sql = app
                .active_query_tab(target_id)
                .unwrap()
                .query_editor
                .read(cx)
                .text(cx);
            assert!(sql.contains("-- TARGET: target"));
            assert!(sql.contains("-- DESIRED SCHEMA FROM: source"));
            assert!(
                sql.contains("ADD COLUMN") && sql.contains("desired_label"),
                "{sql}"
            );
            let engine = app.session(target_id).unwrap().engine.as_ref().unwrap();
            let columns = app
                .runtime
                .block_on(engine.describe_table(&TableRef::new("items")))
                .unwrap();
            assert_eq!(columns.len(), 1, "the draft must not execute DDL");
        });
    }
}
