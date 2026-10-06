use super::*;

impl DbxApp {
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
            query.status = "Plan comparison by step · estimates, not measured runtime".into();
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
