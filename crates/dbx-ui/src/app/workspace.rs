use super::*;
use crate::workspace::{SavedQuery, SavedTab, connection_key};

impl DbxApp {
    pub(super) fn persist_startup_workspace(&mut self, cx: &mut Context<Self>) {
        if self.vault_state != Some(VaultState::Unlocked) {
            return;
        }
        let Some(store) = &self.workspace_store else {
            return;
        };
        let mut active = None;
        let connections = self
            .sessions
            .iter()
            .filter_map(|session| {
                let profile_id = session.profile_id?;
                if Some(session.id) == self.active_session_id {
                    active = Some(session.id);
                }
                Some((
                    session.id,
                    session.startup_layout.clone().unwrap_or_else(|| {
                        crate::workspace::StartupConnection {
                            profile_id,
                            database: session.current_database.clone(),
                            tabs: Some(
                                session
                                    .secondary_tabs
                                    .iter()
                                    .map(|tab| match &tab.kind {
                                        SecondaryTabKind::Query(query) => {
                                            SavedTab::Query(SavedQuery {
                                                name: query.name.read(cx).clone(),
                                                sql: query.query_text.read(cx).clone(),
                                            })
                                        }
                                        SecondaryTabKind::Data(data) => {
                                            SavedTab::Data(data.table.clone())
                                        }
                                        SecondaryTabKind::Structure(structure) => {
                                            SavedTab::Structure(structure.table.clone())
                                        }
                                        SecondaryTabKind::Diagram(_) => SavedTab::Diagram,
                                    })
                                    .collect(),
                            ),
                            active_tab: session
                                .secondary_tabs
                                .iter()
                                .position(|tab| Some(tab.id) == session.active_secondary_tab),
                        }
                    }),
                ))
            })
            .collect::<Vec<_>>();
        let active = connections.iter().position(|(id, _)| Some(*id) == active);
        let connections = connections
            .into_iter()
            .map(|(_, connection)| connection)
            .collect();
        // The startup index is small; saving through the vault atomically keeps
        // it in step with document writes and prevents stale shutdown saves.
        if let Err(error) = store.save_startup(&crate::workspace::StartupWorkspace {
            connections,
            active,
        }) {
            self.show_toast(ToastKind::Error, error, cx);
        }
    }

    pub(super) fn restore_startup_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.sessions.is_empty() {
            return;
        }
        let (Some(store), Some(profiles)) =
            (self.workspace_store.clone(), self.profile_store.clone())
        else {
            return;
        };
        let task = self.runtime.spawn_blocking(move || store.load_startup());
        cx.spawn_in(window, async move |this, cx| {
            let document = task.await?;
            this.update_in(cx, |this, window, cx| {
                if this.vault_state != Some(VaultState::Unlocked) || !this.sessions.is_empty() {
                    return;
                }
                let document = match document {
                    Ok(document) => document,
                    Err(error) => {
                        this.show_toast(ToastKind::Error, error, cx);
                        return;
                    }
                };
                let mut sessions = Vec::new();
                for connection in document.connections {
                    let Some(profile) = this
                        .saved_connections
                        .iter()
                        .find(|profile| profile.id == connection.profile_id)
                        .cloned()
                    else {
                        continue;
                    };
                    let id = Uuid::new_v4();
                    this.sessions.push(ConnectionSession::new(
                        id,
                        Some(profile.id),
                        profile.name,
                        profile.kind,
                        profile.tag,
                        window,
                        cx,
                    ));
                    if let Some(session) = this.session_mut(id) {
                        session.busy = true;
                        session.startup_layout = Some(connection.clone());
                    }
                    sessions.push((id, connection));
                }
                this.active_session_id = sessions
                    .get(document.active.unwrap_or(0))
                    .or_else(|| sessions.first())
                    .map(|(id, _)| *id);
                for (session_id, connection) in sessions {
                    let profiles = profiles.clone();
                    let task = this.runtime.spawn(async move {
                        let loaded = tokio::task::spawn_blocking(move || {
                            profiles.load(connection.profile_id)
                        })
                        .await
                        .map_err(|_| "Saved connection load failed".to_owned())?
                        .map_err(|error| error.to_string())?;
                        let engine = Arc::new(
                            DatabaseEngine::connect(loaded.config)
                                .await
                                .map_err(|error| error.to_string())?,
                        );
                        if let Some(database) = connection.database
                            && engine.current_database().await.ok().as_deref() != Some(&database)
                        {
                            engine
                                .use_database(&database)
                                .await
                                .map_err(|error| error.to_string())?;
                        }
                        let tables = engine
                            .list_tables()
                            .await
                            .map_err(|error| error.to_string())?;
                        let databases = engine.list_databases().await.unwrap_or_default();
                        let database = engine.current_database().await.ok();
                        Ok::<_, String>((engine, tables, databases, database))
                    });
                    if let Some(session) = this.session_mut(session_id) {
                        session.track_background_task(&task);
                    }
                    cx.spawn_in(window, async move |this, cx| {
                        let result = task.await?;
                        this.update_in(cx, |this, window, cx| {
                            let Some(session) = this.session_mut(session_id) else {
                                return;
                            };
                            session.busy = false;
                            match result {
                                Ok((engine, tables, databases, database)) => {
                                    session.engine = Some(engine);
                                    session.schema_filter =
                                        default_schema_filter(session.kind, &tables);
                                    session.set_tables(tables);
                                    session.databases = databases;
                                    session.current_database = database;
                                    this.load_schema_objects_for(session_id, cx);
                                    this.prefetch_completion_columns_for(session_id, cx);
                                    this.prefetch_redis_command_catalog_for(session_id, cx);
                                    this.restore_query_workspace_for(session_id, window, cx);
                                }
                                Err(error) => {
                                    session.error =
                                        Some(format!("Could not restore connection: {error}"))
                                }
                            }
                            cx.notify();
                        })?;
                        Ok::<(), anyhow::Error>(())
                    })
                    .detach();
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn persist_query_workspace_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        if self.vault_state != Some(VaultState::Unlocked) {
            return;
        }
        if self
            .session(session_id)
            .is_some_and(|session| session.restoring_workspace)
        {
            return;
        }
        let Some(key) = self
            .session(session_id)
            .and_then(query_history_connection)
            .map(|identity| connection_key(&identity))
        else {
            return;
        };
        let Some(store) = self.workspace_store.clone() else {
            return;
        };
        let Some(mut document) = self.workspace_documents.get(&key).cloned() else {
            return;
        };
        document.drafts = self
            .sessions
            .iter()
            .filter(|session| {
                query_history_connection(session)
                    .is_some_and(|identity| connection_key(&identity) == key)
            })
            .flat_map(|session| {
                session
                    .secondary_tabs
                    .iter()
                    .filter_map(|tab| match &tab.kind {
                        SecondaryTabKind::Query(query) => Some(SavedQuery {
                            name: query.name.read(cx).clone(),
                            sql: query.query_text.read(cx).clone(),
                        }),
                        _ => None,
                    })
            })
            .collect();
        document.closed = self
            .session(session_id)
            .map(|session| session.closed_queries.clone())
            .unwrap_or_default();
        document.open_tables = self
            .session(session_id)
            .map(|session| {
                session
                    .secondary_tabs
                    .iter()
                    .filter_map(|tab| match &tab.kind {
                        SecondaryTabKind::Data(data) => Some(data.table.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        document.schema_baseline = self
            .session(session_id)
            .and_then(|session| session.schema_baseline.clone());
        if let Some(session) = self.session(session_id) {
            document.current_database = session.current_database.clone();
            document.tabs = session
                .secondary_tabs
                .iter()
                .map(|tab| match &tab.kind {
                    SecondaryTabKind::Query(query) => SavedTab::Query(SavedQuery {
                        name: query.name.read(cx).clone(),
                        sql: query.query_text.read(cx).clone(),
                    }),
                    SecondaryTabKind::Data(data) => SavedTab::Data(data.table.clone()),
                    SecondaryTabKind::Structure(structure) => {
                        SavedTab::Structure(structure.table.clone())
                    }
                    SecondaryTabKind::Diagram(_) => SavedTab::Diagram,
                })
                .collect();
            document.active_tab = session
                .secondary_tabs
                .iter()
                .position(|tab| Some(tab.id) == session.active_secondary_tab);
        }
        self.workspace_documents
            .insert(key.clone(), document.clone());
        self.persist_startup_workspace(cx);
        let revision = store.register(&key);
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(350))
                .await;
            let result = runtime
                .spawn_blocking(move || store.save(&key, revision, &document))
                .await;
            if let Ok(Err(error)) = result {
                this.update(cx, |this, cx| {
                    if this.vault_state == Some(VaultState::Unlocked) {
                        this.show_toast(ToastKind::Error, format!("Draft recovery: {error}"), cx);
                    }
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn restore_query_workspace_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self
            .session(session_id)
            .and_then(query_history_connection)
            .map(|identity| connection_key(&identity))
        else {
            return;
        };
        let Some(vault) = self.profile_store.as_ref().and_then(ProfileStore::vault) else {
            return;
        };
        // Rebind to the current profile repository, including isolated test stores.
        let store = self
            .workspace_store
            .as_ref()
            .filter(|store| store.matches_vault(&vault))
            .cloned()
            .unwrap_or_else(|| Arc::new(crate::workspace::WorkspaceStore::new(vault)));
        self.workspace_store = Some(store.clone());
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = runtime
                .spawn_blocking(move || store.load(&key).map(|document| (key, document)))
                .await?;
            this.update_in(cx, |this, window, cx| {
                if this.vault_state != Some(VaultState::Unlocked)
                    || this.session(session_id).is_none()
                {
                    return;
                }
                match loaded {
                    Ok((key, document)) => {
                        if !this
                            .session(session_id)
                            .and_then(query_history_connection)
                            .is_some_and(|identity| connection_key(&identity) == key)
                        {
                            return;
                        }
                        // Never replace a document the user started while recovery loaded.
                        let has_queries = this
                            .session(session_id)
                            .is_some_and(|session| !session.secondary_tabs.is_empty());
                        let startup = this
                            .session_mut(session_id)
                            .and_then(|session| session.startup_layout.take());
                        let startup_tabs = startup
                            .as_ref()
                            .and_then(|connection| connection.tabs.clone());
                        let has_startup_layout = startup_tabs.is_some();
                        let tabs = if let Some(tabs) = startup_tabs {
                            tabs
                        } else if document.tabs.is_empty() {
                            document
                                .drafts
                                .iter()
                                .cloned()
                                .map(SavedTab::Query)
                                .chain(document.open_tables.iter().cloned().map(SavedTab::Data))
                                .collect()
                        } else {
                            document.tabs.clone()
                        };
                        let active_tab = startup
                            .as_ref()
                            .and_then(|connection| connection.active_tab)
                            .or(document.active_tab);
                        if let Some(session) = this.session_mut(session_id) {
                            session.closed_queries = document.closed.clone();
                            session.schema_baseline = document.schema_baseline.clone();
                        }
                        this.workspace_documents.insert(key, document);
                        if !has_queries && tabs.is_empty() && !has_startup_layout {
                            if let Some(table) = this
                                .session(session_id)
                                .and_then(|session| session.tables.first())
                                .cloned()
                            {
                                this.select_table_for(session_id, table, window, cx);
                            }
                        } else if !has_queries {
                            this.restore_workspace_tabs_for(
                                session_id, tabs, active_tab, window, cx,
                            );
                        } else {
                            this.persist_query_workspace_for(session_id, cx);
                        }
                    }
                    Err(error) => {
                        this.show_toast(ToastKind::Error, format!("Draft recovery: {error}"), cx)
                    }
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn saved_queries_for(&self, session_id: SessionId) -> Vec<SavedQuery> {
        self.session(session_id)
            .and_then(query_history_connection)
            .map(|identity| connection_key(&identity))
            .and_then(|key| self.workspace_documents.get(&key))
            .map(|document| document.saved.clone())
            .unwrap_or_default()
    }
    pub(super) fn save_named_query_for(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        let Some(key) = query_history_connection(session).map(|identity| connection_key(&identity))
        else {
            return;
        };
        let Some(query) = session
            .secondary_tabs
            .iter()
            .find(|tab| Some(tab.id) == session.active_secondary_tab)
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Query(query) => Some(query),
                _ => None,
            })
        else {
            return;
        };
        let saved = SavedQuery {
            name: query.name.read(cx).trim().to_owned(),
            sql: query.query_text.read(cx).clone(),
        };
        if saved.name.is_empty() || saved.name.len() > 128 {
            self.show_toast(
                ToastKind::Error,
                "Give the query a name of 1–128 characters",
                cx,
            );
            return;
        }
        let Some(document) = self.workspace_documents.get_mut(&key) else {
            return;
        };
        if let Some(existing) = document
            .saved
            .iter_mut()
            .find(|existing| existing.name == saved.name)
        {
            *existing = saved;
        } else {
            document.saved.push(saved);
        }
        self.persist_query_workspace_for(session_id, cx);
        // Explicit saves wait for persistence before reporting success.
        if let (Some(store), Some(document)) = (
            self.workspace_store.clone(),
            self.workspace_documents.get(&key).cloned(),
        ) {
            let revision = store.register(&key);
            match store.save(&key, revision, &document) {
                Ok(()) => {
                    self.show_toast(ToastKind::Success, "Query saved in the encrypted vault", cx)
                }
                Err(error) => self.show_toast(ToastKind::Error, error, cx),
            }
        }
    }
    pub(super) fn delete_saved_query_for(
        &mut self,
        session_id: SessionId,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self
            .session(session_id)
            .and_then(query_history_connection)
            .map(|identity| connection_key(&identity))
            && let Some(document) = self.workspace_documents.get_mut(&key)
        {
            document.saved.retain(|saved| saved.name != name);
            self.persist_query_workspace_for(session_id, cx);
        }
    }
    pub(super) fn open_saved_query_for(
        &mut self,
        session_id: SessionId,
        saved: SavedQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_query_tab_for(session_id, window, cx);
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| Some(tab.id) == session.active_secondary_tab)
            && let SecondaryTabKind::Query(query) = &mut tab.kind
        {
            query.name.update(cx, |name, cx| {
                *name = saved.name;
                cx.notify();
            });
            query.query_text.update(cx, |text, cx| {
                *text = saved.sql;
                cx.notify();
            });
        }
    }
    pub(super) fn flush_query_workspaces(&mut self, cx: &mut Context<Self>) -> Option<String> {
        let mut failure = None;
        let ids = self
            .sessions
            .iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();
        for id in ids {
            self.persist_query_workspace_for(id, cx);
        }
        if let Some(store) = &self.workspace_store {
            for (key, document) in &self.workspace_documents {
                let revision = store.register(key);
                if let Err(error) = store.save(key, revision, document) {
                    failure = Some(format!("Draft recovery: {error}"));
                }
            }
        }
        self.error = failure.clone();
        failure
    }

    /// Restore ordered documents after discovery, without replacing work the
    /// user opened while recovery was loading.
    fn restore_workspace_tabs_for(
        &mut self,
        session_id: SessionId,
        tabs: Vec<SavedTab>,
        active_tab: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tabs.is_empty() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            // Table discovery runs alongside workspace recovery; wait for it.
            for _ in 0..300 {
                if tabs.iter().all(|tab| matches!(tab, SavedTab::Query(_))) {
                    break;
                }
                let loaded = this.update(cx, |this, _| {
                    this.session(session_id)
                        .map(|session| session.tables_revision > 0)
                })?;
                match loaded {
                    None => return Ok(()),
                    Some(true) => break,
                    Some(false) => {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(100))
                            .await
                    }
                }
            }
            this.update_in(cx, |this, window, cx| {
                let Some(session) = this.session(session_id) else {
                    return;
                };
                if !session.secondary_tabs.is_empty() {
                    return;
                }
                this.restore_workspace_tab_layout_for(session_id, tabs, active_tab, window, cx);
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn restore_workspace_tab_layout_for(
        &mut self,
        session_id: SessionId,
        tabs: Vec<SavedTab>,
        active_tab: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.restoring_workspace = true;
        let tables = session.tables.clone();
        let mut active = None;
        let mut first = None;
        for (index, tab) in tabs.into_iter().enumerate() {
            let previous_count = self
                .session(session_id)
                .map_or(0, |session| session.secondary_tabs.len());
            match tab {
                SavedTab::Query(query) => self.open_saved_query_for(session_id, query, window, cx),
                SavedTab::Data(ref reference) | SavedTab::Structure(ref reference) => {
                    if let Some(table) = tables
                        .iter()
                        .find(|table| table_ref(table) == *reference)
                        .cloned()
                    {
                        if matches!(tab, SavedTab::Data(_)) {
                            self.select_table_with_filters_for(
                                session_id,
                                table,
                                Vec::new(),
                                window,
                                cx,
                            );
                        } else {
                            self.open_structure_tab_for(session_id, table, cx);
                        }
                    }
                }
                SavedTab::Diagram => self.open_diagram_for(session_id, window, cx),
            }
            if let Some(session) = self.session(session_id)
                && session.secondary_tabs.len() > previous_count
            {
                first = first.or(session.active_secondary_tab);
                if Some(index) == active_tab {
                    active = session.active_secondary_tab;
                }
            }
        }
        if let Some(session) = self.session_mut(session_id) {
            session.restoring_workspace = false;
        }
        if let Some(active) = active.or(first) {
            self.activate_secondary_tab_for(session_id, active, window, cx);
        }
        self.persist_query_workspace_for(session_id, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn workspace_restores_mixed_tab_order_and_selection_and_skips_missing_tables(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let profile_id = Uuid::new_v4();
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        let vault = store.vault().unwrap();
        vault.create("workspace test passphrase").unwrap();
        let workspace = Arc::new(crate::workspace::WorkspaceStore::new(vault));
        let key = connection_key(&QueryHistoryConnection::profile(profile_id));
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.profile_store = Some(store);
            app.workspace_store = Some(workspace.clone());
            app.workspace_documents
                .insert(key.clone(), Default::default());
            app.vault_state = Some(VaultState::Unlocked);
            let engine = app
                .runtime
                .block_on(DatabaseEngine::connect(ConnectionConfig::new(
                    DatabaseKind::SQLite,
                    "sqlite::memory:",
                )))
                .unwrap();
            app.runtime
                .block_on(engine.execute_sql("CREATE TABLE items (id INTEGER PRIMARY KEY)"))
                .unwrap();
            let mut session = ConnectionSession::new(
                session_id,
                Some(profile_id),
                "Layout test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            session.engine = Some(Arc::new(engine));
            session.tables = vec![TableInfo::table("items", None)];
            session.tables_revision = 1;
            app.sessions.push(session);
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let query = SavedQuery {
                    name: "Investigation".into(),
                    sql: "SELECT 42".into(),
                };
                let tabs = vec![
                    SavedTab::Data(TableRef::new("missing")),
                    SavedTab::Structure(TableRef::new("items")),
                    SavedTab::Query(query),
                    SavedTab::Data(TableRef::new("items")),
                    SavedTab::Diagram,
                ];
                // Exercise the vault's actual serialization before restoring.
                let document = crate::workspace::WorkspaceDocument {
                    tabs,
                    active_tab: Some(2),
                    ..Default::default()
                };
                let revision = workspace.register(&key);
                workspace.save(&key, revision, &document).unwrap();
                let loaded = workspace.load(&key).unwrap();
                app.restore_workspace_tab_layout_for(
                    session_id,
                    loaded.tabs,
                    loaded.active_tab,
                    window,
                    cx,
                );
                let session = app.session(session_id).unwrap();
                assert_eq!(session.secondary_tabs.len(), 4);
                assert!(matches!(
                    session.secondary_tabs[0].kind,
                    SecondaryTabKind::Structure(_)
                ));
                assert!(matches!(
                    session.secondary_tabs[1].kind,
                    SecondaryTabKind::Query(_)
                ));
                assert!(matches!(
                    session.secondary_tabs[2].kind,
                    SecondaryTabKind::Data(_)
                ));
                assert!(matches!(
                    session.secondary_tabs[3].kind,
                    SecondaryTabKind::Diagram(_)
                ));
                assert_eq!(
                    session.active_secondary_tab,
                    Some(session.secondary_tabs[1].id)
                );
                app.flush_query_workspaces(cx);
                let saved = workspace.load(&key).unwrap();
                assert_eq!(saved.active_tab, Some(1));
                assert_eq!(saved.tabs.len(), 4);
                let SavedTab::Query(query) = &saved.tabs[1] else {
                    panic!("query document")
                };
                assert_eq!(query.sql, "SELECT 42");
            })
        });
    }
}
