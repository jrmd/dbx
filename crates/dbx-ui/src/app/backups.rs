use super::*;
pub(super) fn native_target_identity(config: &ConnectionConfig) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut config = config.clone();
    config.max_connections = 1;
    config.connect_timeout_ms = 1;
    if let Ok(mut url) = url::Url::parse(&config.url) {
        let _ = url.set_password(None);
        config.url = url.into();
    }
    let encoded = zeroize::Zeroizing::new(serde_json::to_vec(&config).unwrap_or_default());
    Sha256::digest(&*encoded).into()
}
impl DbxApp {
    pub(super) fn choose_native_backup(
        &mut self,
        session_id: SessionId,
        restore: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy {
            return;
        }
        if session.profile_id.is_none() {
            self.show_toast(
                ToastKind::Info,
                "Save this connection before using native backup and restore",
                cx,
            );
            return;
        }
        if !matches!(session.kind, DatabaseKind::PostgreSQL | DatabaseKind::MySQL) {
            self.show_toast(
                ToastKind::Info,
                "Native backup uses installed PostgreSQL or MySQL client tools",
                cx,
            );
            return;
        }
        if restore
            && session
                .engine
                .as_ref()
                .is_some_and(|engine| engine.is_read_only())
        {
            self.show_toast(
                ToastKind::Info,
                "Protected connection: restore is disabled",
                cx,
            );
            return;
        }
        if session.secondary_tabs.iter().any(|tab| match &tab.kind {
            SecondaryTabKind::Query(query) => query.in_transaction || query.busy,
            SecondaryTabKind::Data(data) => data.has_unsaved_cell_work(),
            _ => false,
        }) {
            self.show_toast(
                ToastKind::Info,
                "Finish open transactions and staged changes before native backup or restore",
                cx,
            );
            return;
        }
        let kind = session.kind;
        if restore {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose trusted native backup to restore".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = receiver.await && let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| {
                        let Some(session) = this.session(session_id) else { return; };
                        let target = format!("{} / {}", session.name, session.current_database.as_deref().unwrap_or("unknown database"));
                        this.confirmation_dialog = Some(ConfirmationDialog { title: format!("Restore into {target}?"),
                            detail: format!("Source: {}. The backup can execute database code and replace objects. PostgreSQL uses one transaction without clean/create. MySQL DDL can commit before an error or cancellation. Use a new empty database for recovery; confirm this exact target before proceeding.", path.display()),
                            confirm_label: "Restore database", tone: ConfirmationTone::Danger,
                            action: ConfirmationAction::NativeRestore { session_id, path, database: session.current_database.clone() }, focus: cx.focus_handle(), return_focus: None, sql: None });
                        cx.notify();
                    })?;
                }
                Ok::<(), anyhow::Error>(())
            }).detach();
        } else {
            let directory = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
            let receiver = cx.prompt_for_new_path(
                &directory,
                Some(if kind == DatabaseKind::PostgreSQL {
                    "database.backup"
                } else {
                    "database.mysql.sql"
                }),
            );
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(path))) = receiver.await {
                    this.update(cx, |this, cx| {
                        this.run_native_backup(session_id, path, false, cx)
                    })?;
                }
                Ok::<(), anyhow::Error>(())
            })
            .detach();
        }
    }
    pub(super) fn run_native_backup(
        &mut self,
        session_id: SessionId,
        path: PathBuf,
        restore: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy || self.vault_state != Some(VaultState::Unlocked) {
            return;
        }
        let Some(profile_id) = session.profile_id else {
            return;
        };
        let Some(store) = self.profile_store.clone() else {
            return;
        };
        if restore
            && session
                .engine
                .as_ref()
                .is_some_and(|engine| engine.is_read_only())
        {
            return;
        }
        if session.secondary_tabs.iter().any(|tab| match &tab.kind {
            SecondaryTabKind::Query(query) => query.in_transaction || query.busy,
            SecondaryTabKind::Data(data) => data.has_unsaved_cell_work() || data.busy,
            _ => false,
        }) {
            self.show_toast(
                ToastKind::Info,
                "Finish pending work before native backup or restore",
                cx,
            );
            return;
        }
        let identity = session.connection_identity;
        let database = session.current_database.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.request_generation += 1;
        let generation = session.request_generation;
        session.status = if restore {
            "Restoring native backup…"
        } else {
            "Creating native backup…"
        }
        .into();
        let control = self.start_transfer_progress(session_id, generation, cx);
        let log = control.clone();
        let runtime = self.runtime.clone();
        let task = runtime.spawn(async move {
            let mut loaded = tokio::task::spawn_blocking(move || store.load(profile_id))
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
            if identity != Some(native_target_identity(&loaded.config)) { return Err("Saved connection settings differ from the open session. Reconnect and review the target before using native backup or restore.".into()); }
            if let Some(database) = database {
                let mut url =
                    url::Url::parse(&loaded.config.url).map_err(|_| "Invalid connection URL")?;
                url.set_path(&database);
                loaded.config.url = url.into();
            }
            dbx_core::native_backup(loaded.config, &path, restore, control)
                .await
                .map_err(|error| error.to_string())
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }
        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                session.transfer_control = None;
                session.transfer_log = log.log();
                match result {
                    Ok(version) => this.show_toast(
                        ToastKind::Success,
                        format!(
                            "Native {} completed · {version}",
                            if restore { "restore" } else { "backup" }
                        ),
                        cx,
                    ),
                    Err(error) => this.show_toast(ToastKind::Error, error, cx),
                }
                if restore {
                    this.refresh_tables_for(session_id, cx);
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
    pub(super) fn show_transfer_log(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        let log = session
            .transfer_control
            .as_ref()
            .map(|control| control.log())
            .unwrap_or_else(|| session.transfer_log.clone());
        self.open_saved_query_for(
            session_id,
            crate::workspace::SavedQuery {
                name: "Native backup / restore log".into(),
                sql: log
                    .lines()
                    .map(|line| format!("-- {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            },
            window,
            cx,
        );
    }
}
