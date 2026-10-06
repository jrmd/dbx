use super::*;
use crate::workspace::{SavedQuery, connection_key};

impl DbxApp {
    pub(super) fn persist_query_workspace_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        if self.vault_state != Some(VaultState::Unlocked) {
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
        document.schema_baseline = self
            .session(session_id)
            .and_then(|session| session.schema_baseline.clone());
        self.workspace_documents
            .insert(key.clone(), document.clone());
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
                        let has_queries = this.session(session_id).is_some_and(|session| {
                            session
                                .secondary_tabs
                                .iter()
                                .any(|tab| matches!(tab.kind, SecondaryTabKind::Query(_)))
                        });
                        let drafts = document.drafts.clone();
                        if let Some(session) = this.session_mut(session_id) {
                            session.closed_queries = document.closed.clone();
                            session.schema_baseline = document.schema_baseline.clone();
                        }
                        this.workspace_documents.insert(key, document);
                        if !has_queries {
                            for draft in drafts {
                                this.open_saved_query_for(session_id, draft, window, cx);
                            }
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
}
