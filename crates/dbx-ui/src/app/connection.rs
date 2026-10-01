use super::*;
use crate::device_unlock::DeviceUnlock;

fn hydration_matches_current_draft(
    selected_profile: Option<Uuid>,
    expected_profile: Option<Uuid>,
    current_fields: &ConnectionFields,
    requested_fields: &ConnectionFields,
) -> bool {
    selected_profile == expected_profile && current_fields == requested_fields
}

impl DbxApp {
    fn hydrate_transport(
        &mut self,
        socket: Option<std::path::PathBuf>,
        ssh: Option<dbx_core::SshConfig>,
        cx: &mut Context<Self>,
    ) {
        self.draft.transport.socket_enabled = socket.is_some();
        self.draft.transport.ssh_enabled = ssh.is_some();
        let socket = socket
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (host, port, user, key) = ssh
            .map(|ssh| {
                (
                    ssh.host,
                    ssh.port.to_string(),
                    ssh.username,
                    ssh.identity_file
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| (String::new(), "22".into(), String::new(), String::new()));
        for (entity, text) in [
            (&self.draft.transport.socket, socket),
            (&self.draft.transport.ssh_host, host),
            (&self.draft.transport.ssh_port, port),
            (&self.draft.transport.ssh_user, user),
            (&self.draft.transport.ssh_key, key),
        ] {
            entity.update(cx, |value, cx| {
                *value = text;
                cx.notify();
            });
        }
    }

    fn default_url(kind: DatabaseKind) -> &'static str {
        match kind {
            DatabaseKind::PostgreSQL => "postgres://postgres@localhost:5432/postgres",
            DatabaseKind::MySQL => "mysql://root@localhost:3306/mysql",
            DatabaseKind::SQLite => "sqlite://dbx.db?mode=rwc",
            DatabaseKind::Redis => "redis://127.0.0.1:6379/0",
        }
    }

    fn hydrate_connection_fields(
        &mut self,
        kind: DatabaseKind,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let mut fields =
            ConnectionFields::from_url(url.clone()).unwrap_or_else(|_| ConnectionFields::new(kind));
        let normalized_url = fields.url().unwrap_or(url);
        self.draft.kind = kind;
        self.hydrate_transport(None, None, cx);
        self.draft.mode = ConnectionFormMode::Details;
        self.draft.connection_url.update(cx, |value, cx| {
            *value = normalized_url;
            cx.notify();
        });
        self.draft.host.update(cx, |value, cx| {
            *value = std::mem::take(&mut fields.host);
            cx.notify();
        });
        self.draft.port.update(cx, |value, cx| {
            *value = std::mem::take(&mut fields.port);
            cx.notify();
        });
        self.draft.username.update(cx, |value, cx| {
            *value = std::mem::take(&mut fields.username);
            cx.notify();
        });
        self.draft.password.update(cx, |value, cx| {
            value.zeroize();
            *value = std::mem::take(&mut fields.password);
            cx.notify();
        });
        self.draft.database.update(cx, |value, cx| {
            *value = std::mem::take(&mut fields.database);
            cx.notify();
        });
    }

    fn connection_fields(&self, cx: &App) -> ConnectionFields {
        let mut fields = ConnectionFields::new(self.draft.kind);
        fields.host = self.draft.host.read(cx).clone();
        fields.port = self.draft.port.read(cx).clone();
        fields.username = self.draft.username.read(cx).clone();
        fields.password = self.draft.password.read(cx).clone();
        fields.database = self.draft.database.read(cx).clone();
        let transport = &self.draft.transport;
        fields.socket_enabled = transport.socket_enabled;
        fields.socket = transport.socket.read(cx).clone();
        fields.ssh = transport.ssh_enabled.then(|| dbx_core::SshConfig {
            host: transport.ssh_host.read(cx).trim().to_owned(),
            port: transport.ssh_port.read(cx).trim().parse().unwrap_or(0),
            username: transport.ssh_user.read(cx).trim().to_owned(),
            identity_file: (!transport.ssh_key.read(cx).trim().is_empty())
                .then(|| transport.ssh_key.read(cx).trim().into()),
        });
        if self.draft.mode == ConnectionFormMode::ConnectionString
            || self.draft.kind == DatabaseKind::SQLite
        {
            fields.connection_string = self.draft.connection_url.read(cx).clone();
        } else {
            fields.use_structured_fields();
        }
        fields
    }

    fn draft_connection(&self, cx: &App) -> Result<(DatabaseKind, String), String> {
        let fields = self.connection_fields(cx);
        let url = fields.url().map_err(|error| error.to_string())?;
        Ok((fields.kind, url))
    }

    /// Resolve the already-hydrated visible form for Test and Connect.
    /// Saved credentials are restored eagerly when the profile is selected.
    fn resolve_draft(&self, cx: &App) -> Result<(DatabaseKind, String, ConnectionConfig), String> {
        let (kind, visible_url) = self.draft_connection(cx)?;
        let config = self.connection_fields(cx).config()?;
        Ok((kind, visible_url, config))
    }

    fn clear_vault_inputs(&mut self, cx: &mut Context<Self>) {
        self.vault_editors.passphrase.update(cx, |value, cx| {
            value.zeroize();
            cx.notify();
        });
        self.vault_editors.confirmation.update(cx, |value, cx| {
            value.zeroize();
            cx.notify();
        });
    }

    /// The keychain entry for this vault. Tests never touch the real
    /// system keychain.
    fn device_unlock(&self) -> Option<DeviceUnlock> {
        if cfg!(test) {
            return None;
        }
        let vault = self.profile_store.as_ref().and_then(ProfileStore::vault)?;
        Some(DeviceUnlock::for_vault(vault.path()))
    }

    /// Unlock with the key this device remembered, if any. The passphrase
    /// gate stays up (showing "Unlocking…") until the keychain answers, and
    /// remains the fallback when it has nothing or refuses.
    pub(super) fn try_device_unlock(&mut self, cx: &mut Context<Self>) {
        if self.vault_state != Some(VaultState::Locked) || !self.remember_device {
            return;
        }
        let (Some(vault), Some(device)) = (
            self.profile_store.as_ref().and_then(ProfileStore::vault),
            self.device_unlock(),
        ) else {
            return;
        };
        self.vault_busy = true;
        self.vault_generation += 1;
        let generation = self.vault_generation;
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let unlocked = runtime
                .spawn_blocking(move || {
                    let Ok(Some(key)) = device.load() else {
                        return false;
                    };
                    match vault.unlock_with_key(key) {
                        Ok(()) => true,
                        // The vault was recreated since this device was
                        // trusted; the stale key can never work again.
                        Err(VaultError::Authentication) => {
                            let _ = device.forget();
                            false
                        }
                        Err(_) => false,
                    }
                })
                .await
                .unwrap_or(false);
            this.update(cx, |this, cx| {
                if this.vault_generation != generation {
                    return;
                }
                this.vault_busy = false;
                if unlocked {
                    this.vault_state = Some(VaultState::Unlocked);
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Takes effect at the next passphrase unlock, which stores or forgets
    /// the key; the choice itself is saved now so the gate remembers it.
    pub(super) fn toggle_remember_device(&mut self, cx: &mut Context<Self>) {
        self.remember_device = !self.remember_device;
        self.persist_settings(cx);
        cx.notify();
    }

    pub(super) fn submit_vault_passphrase(&mut self, creating: bool, cx: &mut Context<Self>) {
        let Some(vault) = self.profile_store.as_ref().and_then(ProfileStore::vault) else {
            self.set_error("The vault is unavailable".into());
            cx.notify();
            return;
        };
        let mut passphrase = self.vault_editors.passphrase.read(cx).clone();
        let mut confirmation = self.vault_editors.confirmation.read(cx).clone();
        if passphrase.chars().count() < 12 {
            passphrase.zeroize();
            confirmation.zeroize();
            self.set_error("Passphrase must contain at least 12 characters".into());
            cx.notify();
            return;
        }
        if creating && passphrase != confirmation {
            passphrase.zeroize();
            confirmation.zeroize();
            self.set_error("Passphrase confirmation does not match".into());
            cx.notify();
            return;
        }
        confirmation.zeroize();
        let passphrase = SecretString::from(std::mem::take(&mut passphrase));
        self.clear_vault_inputs(cx);
        self.vault_busy = true;
        self.vault_generation += 1;
        let generation = self.vault_generation;
        let runtime = self.runtime.clone();
        let device = self.device_unlock();
        let remember_device = self.remember_device;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (result, device_error) = runtime
                .spawn_blocking(move || {
                    let result = if creating {
                        vault.create(passphrase)
                    } else {
                        vault.unlock(passphrase)
                    };
                    // Trusting (or untrusting) the device never blocks the
                    // unlock itself; a keychain failure is only reported.
                    let device_error = match (&result, device) {
                        (Ok(()), Some(device)) if remember_device => vault
                            .key()
                            .map_err(|error| error.to_string())
                            .and_then(|key| device.store(&key))
                            .err(),
                        (Ok(()), Some(device)) => device.forget().err(),
                        _ => None,
                    };
                    (result, device_error)
                })
                .await?;
            this.update(cx, |this, cx| {
                if this.vault_generation != generation {
                    return;
                }
                this.vault_busy = false;
                match result {
                    Ok(()) => {
                        this.vault_state = Some(VaultState::Unlocked);
                        this.error = None;
                        let selected_with_secret = this.draft.selected_profile.filter(|id| {
                            this.saved_connections
                                .iter()
                                .find(|profile| profile.id == *id)
                                .is_some_and(SavedConnection::has_secret)
                        });
                        if let Some(profile_id) = selected_with_secret {
                            this.hydrate_saved_credential(profile_id, cx);
                        }
                        if let Some(error) = device_error {
                            this.show_toast(ToastKind::Error, error, cx);
                        }
                    }
                    Err(_) => this.set_error(if creating {
                        "Couldn’t create the vault".into()
                    } else {
                        "Wrong passphrase".into()
                    }),
                }
                this.clear_vault_inputs(cx);
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn lock_vault(&mut self, cx: &mut Context<Self>) {
        if self.saving_connection {
            return;
        }
        let Some(vault) = self.profile_store.as_ref().and_then(ProfileStore::vault) else {
            return;
        };
        if vault.lock().is_err() {
            self.set_error("Couldn’t lock the vault".into());
            cx.notify();
            return;
        }
        self.vault_state = Some(VaultState::Locked);
        self.cancel_credential_hydration();
        self.draft.password.update(cx, |value, cx| {
            value.zeroize();
            cx.notify();
        });
        if self.draft.kind != DatabaseKind::SQLite {
            self.draft.connection_url.update(cx, |value, cx| {
                value.zeroize();
                cx.notify();
            });
        }
        self.clear_vault_inputs(cx);
        self.error = None;
        cx.notify();
    }

    fn draft_test_fingerprint(
        &self,
        cx: &App,
    ) -> Result<(ConnectionFormMode, ConnectionConfig), String> {
        let fields = self.connection_fields(cx);
        Ok((self.draft.mode, fields.config()?))
    }

    pub(super) fn set_connection_form_mode(
        &mut self,
        mode: ConnectionFormMode,
        cx: &mut Context<Self>,
    ) {
        if mode == self.draft.mode {
            return;
        }

        match mode {
            ConnectionFormMode::Details if self.draft.kind != DatabaseKind::SQLite => {
                let connection_string = self.draft.connection_url.read(cx).trim().to_owned();
                let mut fields = match ConnectionFields::from_url(connection_string) {
                    Ok(fields) if fields.kind == self.draft.kind => fields,
                    Ok(_) => {
                        self.set_error(format!(
                            "Connection string must be for {}",
                            self.draft.kind
                        ));
                        cx.notify();
                        return;
                    }
                    Err(error) => {
                        self.set_error(error.to_string());
                        cx.notify();
                        return;
                    }
                };
                self.draft.host.update(cx, |value, cx| {
                    *value = std::mem::take(&mut fields.host);
                    cx.notify();
                });
                self.draft.port.update(cx, |value, cx| {
                    *value = std::mem::take(&mut fields.port);
                    cx.notify();
                });
                self.draft.username.update(cx, |value, cx| {
                    *value = std::mem::take(&mut fields.username);
                    cx.notify();
                });
                self.draft.password.update(cx, |value, cx| {
                    value.zeroize();
                    *value = std::mem::take(&mut fields.password);
                    cx.notify();
                });
                self.draft.database.update(cx, |value, cx| {
                    *value = std::mem::take(&mut fields.database);
                    cx.notify();
                });
                self.draft.mode = ConnectionFormMode::Details;
            }
            ConnectionFormMode::ConnectionString => {
                let url = match self.connection_fields(cx).url() {
                    Ok(url) => url,
                    Err(error) => {
                        self.set_error(error.to_string());
                        cx.notify();
                        return;
                    }
                };
                self.draft.connection_url.update(cx, |value, cx| {
                    *value = url;
                    cx.notify();
                });
                self.draft.mode = ConnectionFormMode::ConnectionString;
            }
            ConnectionFormMode::Details => return,
        }
        self.error = None;
        cx.notify();
    }

    pub(super) fn select_kind(&mut self, kind: DatabaseKind, cx: &mut Context<Self>) {
        self.cancel_credential_hydration();
        self.draft.selected_profile = None;
        self.hydrate_connection_fields(kind, Self::default_url(kind).to_owned(), cx);
        self.error = None;
        cx.notify();
    }

    pub(super) fn select_environment(
        &mut self,
        environment: ConnectionEnvironment,
        cx: &mut Context<Self>,
    ) {
        self.draft.environment = environment;
        cx.notify();
    }

    pub(super) fn select_saved_connection(
        &mut self,
        profile: SavedConnection,
        cx: &mut Context<Self>,
    ) {
        self.cancel_credential_hydration();
        self.compact_connection_form_open = true;
        let has_saved_password = profile.has_secret();
        let profile_id = profile.id;
        self.draft.selected_profile = Some(profile.id);
        self.draft.environment = profile.environment;
        self.draft.connection_name.update(cx, |name, cx| {
            *name = profile.name;
            cx.notify();
        });
        self.hydrate_connection_fields(profile.kind, profile.url.clone(), cx);
        self.hydrate_transport(profile.socket, profile.ssh, cx);
        self.error = None;
        if has_saved_password {
            self.hydrate_saved_credential(profile_id, cx);
        }
        cx.notify();
    }

    pub(super) fn select_saved_connection_in_compact_picker(
        &mut self,
        profile: SavedConnection,
        cx: &mut Context<Self>,
    ) {
        self.select_saved_connection(profile, cx);
        // Keep the row under the pointer so a second click can complete the
        // native double-click gesture. Editing is an explicit compact action.
        self.compact_connection_form_open = false;
        cx.notify();
    }

    pub(super) fn show_selected_connection_form(&mut self, cx: &mut Context<Self>) {
        if self.draft.selected_profile.is_some() {
            self.compact_connection_form_open = true;
            cx.notify();
        }
    }

    pub(super) fn open_saved_connection(
        &mut self,
        profile: SavedConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile_id = profile.id;
        if self.draft.selected_profile != Some(profile_id) {
            self.select_saved_connection(profile.clone(), cx);
        } else if profile.has_secret()
            && !self.credential_hydrating
            && self.draft.password.read(cx).is_empty()
        {
            self.hydrate_saved_credential(profile_id, cx);
        }

        if self.credential_hydrating {
            let Some(window_handle) = window.window_handle().downcast::<DbxApp>() else {
                self.set_error("Couldn’t open the saved connection".into());
                cx.notify();
                return;
            };
            self.credential_connect_window = Some(window_handle);
            self.error = None;
            cx.notify();
            return;
        }

        self.connect(window, cx);
    }

    fn hydrate_saved_credential(&mut self, profile_id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.profile_store.clone() else {
            return;
        };
        self.credential_hydrating = true;
        self.credential_hydration_generation += 1;
        let hydration_generation = self.credential_hydration_generation;
        let requested_fields = self.connection_fields(cx);
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || store.load(profile_id))
                .await?;
            let connect_window = this.update(cx, |this, cx| {
                if this.credential_hydration_generation != hydration_generation {
                    return None;
                }
                this.credential_hydrating = false;
                if !hydration_matches_current_draft(
                    this.draft.selected_profile,
                    Some(profile_id),
                    &this.connection_fields(cx),
                    &requested_fields,
                ) {
                    this.credential_connect_window = None;
                    this.show_toast(
                        ToastKind::Info,
                        "Connection details changed, so the saved password wasn’t restored",
                        cx,
                    );
                    this.error = None;
                    cx.notify();
                    return None;
                }
                match result {
                    Ok(loaded) => {
                        let mut fields = ConnectionFields::from_url(loaded.config.url)
                            .unwrap_or_else(|_| ConnectionFields::new(loaded.config.kind));
                        this.draft.password.update(cx, |value, cx| {
                            value.zeroize();
                            *value = std::mem::take(&mut fields.password);
                            cx.notify();
                        });
                        this.error = None;
                        let connect_window = this.credential_connect_window.take();
                        cx.notify();
                        connect_window
                    }
                    Err(_) => {
                        this.credential_connect_window = None;
                        this.set_error(
                            "No saved password for this connection. Enter it and save.".into(),
                        );
                        cx.notify();
                        None
                    }
                }
            })?;
            if let Some(window_handle) = connect_window {
                window_handle.update(cx, |this, window, cx| this.connect(window, cx))?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn save_connection(&mut self, cx: &mut Context<Self>) {
        if self.saving_connection || self.vault_busy {
            return;
        }
        let Some(store) = self.profile_store.clone() else {
            self.set_error("Connection profile storage is unavailable".into());
            cx.notify();
            return;
        };
        let name = self.draft.connection_name.read(cx).trim().to_owned();
        let mut fields = self.connection_fields(cx);
        let mut requested_fields = fields.clone();
        let config = match fields.config() {
            Ok(config) => config,
            Err(error) => {
                self.set_error(error.to_string());
                cx.notify();
                return;
            }
        };
        let mut draft = ConnectionProfileDraft::from_config(name, config)
            .with_environment(self.draft.environment);
        if !fields.password.is_empty() {
            draft = draft.with_secret(std::mem::take(&mut fields.password));
        }
        if let Some(id) = self.draft.selected_profile {
            draft = draft.with_id(id);
        }
        let selected_profile = self.draft.selected_profile;
        let runtime = self.runtime.clone();
        self.saving_connection = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (save_result, list_result) = runtime
                .spawn_blocking(move || {
                    let save_result = store.save(draft);
                    let list_result = store.list();
                    (save_result, list_result)
                })
                .await?;
            this.update(cx, |this, cx| {
                this.saving_connection = false;
                if let Ok(profiles) = list_result {
                    this.saved_connections = profiles;
                }
                let unchanged = hydration_matches_current_draft(
                    this.draft.selected_profile,
                    selected_profile,
                    &this.connection_fields(cx),
                    &requested_fields,
                );
                match save_result {
                    Ok(profile) if unchanged => {
                        this.draft.selected_profile = Some(profile.id);
                        this.show_toast(
                            ToastKind::Success,
                            format!("Saved “{}”", profile.name),
                            cx,
                        );
                        this.error = None;
                    }
                    Ok(_) => {}
                    Err(error) if unchanged => this.set_error(error.to_string()),
                    Err(_) => {}
                }
                requested_fields.password.zeroize();
                requested_fields.connection_string.zeroize();
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn choose_ssh_key(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from("Choose SSH private key")),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    this.draft.transport.ssh_key.update(cx, |value, cx| {
                        *value = path.to_string_lossy().into_owned();
                        cx.notify();
                    });
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn choose_sqlite_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from("Choose database")),
        });
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.update(cx, |this, cx| {
                            if this.draft.kind != DatabaseKind::SQLite {
                                return;
                            }
                            this.draft.selected_profile = None;
                            this.hydrate_connection_fields(
                                DatabaseKind::SQLite,
                                sqlite_url(&path),
                                cx,
                            );
                            this.error = None;
                            cx.notify();
                        })?;
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open file picker: {error}"));
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("File picker closed unexpectedly: {error}"));
                        cx.notify();
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn cancel_credential_hydration(&mut self) {
        self.credential_hydration_generation =
            self.credential_hydration_generation.saturating_add(1);
        self.credential_hydrating = false;
        self.credential_connect_window = None;
    }

    pub(super) fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.credential_hydrating {
            self.set_error("Saved connection password is still loading".into());
            cx.notify();
            return;
        }
        let (kind, _, config) = match self.resolve_draft(cx) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.set_error(error);
                cx.notify();
                return;
            }
        };
        let profile_id = self.draft.selected_profile;
        let session_id = Uuid::new_v4();
        let name = self.draft.connection_name.read(cx).trim().to_owned();
        let environment = profile_id
            .and_then(|id| {
                self.saved_connections
                    .iter()
                    .find(|profile| profile.id == id)
            })
            .map(|profile| profile.environment)
            .unwrap_or(self.draft.environment);
        let mut session =
            ConnectionSession::new(session_id, profile_id, name, kind, environment, window, cx);
        session.busy = true;
        session.request_generation = 1;
        let generation = session.request_generation;
        self.sessions.push(session);
        self.active_session_id = Some(session_id);
        self.connection_picker_open = false;
        self.error = None;
        let runtime = self.runtime.clone();
        cx.notify();

        let task = runtime.spawn(async move {
            let engine = Arc::new(DatabaseEngine::connect(config).await?);
            let tables = engine.list_tables().await?;
            let databases = engine.list_databases().await.unwrap_or_default();
            let current_database = engine.current_database().await.ok();
            let schema_filter = default_schema_filter(kind, &tables);
            let initial_table = schema_filtered_tables(kind, &tables, schema_filter.as_deref())
                .into_iter()
                .next();
            Ok::<_, dbx_core::DbxError>((
                engine,
                tables,
                databases,
                current_database,
                schema_filter,
                initial_table,
            ))
        });
        if let Some(session) = self.session_mut(session_id) {
            session.track_background_task(&task);
        }

        cx.spawn_in(window, async move |this, cx| {
            let result = task.await?;
            this.update_in(cx, |this, window, cx| {
                let connected = result.is_ok();
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                let mut initial_table = None;
                match result {
                    Ok((engine, tables, databases, current_database, schema_filter, initial)) => {
                        session.engine = Some(engine);
                        session.set_tables(tables);
                        session.databases = databases;
                        session.current_database = current_database;
                        session.schema_filter = schema_filter;
                        session.completion_columns.clear();
                        session.error = None;
                        session.pane = Pane::Data;
                        initial_table = initial;
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                    }
                }
                // Open the first table the same way a navigator click would,
                // so it arrives in its own data tab.
                if let Some(table) = initial_table {
                    this.select_table_for(session_id, table, window, cx);
                }
                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
                if connected {
                    this.prefetch_redis_command_catalog_for(session_id, cx);
                }
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn test_connection(&mut self, cx: &mut Context<Self>) {
        if self.credential_hydrating {
            self.set_error("Saved connection password is still loading".into());
            cx.notify();
            return;
        }
        let (_, _, config) = match self.resolve_draft(cx) {
            Ok(resolved) => resolved,
            Err(error) => {
                self.set_error(error);
                cx.notify();
                return;
            }
        };
        self.test_generation += 1;
        let generation = self.test_generation;
        let fingerprint = match self.draft_test_fingerprint(cx) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                self.set_error(error);
                cx.notify();
                return;
            }
        };
        self.testing_connection = true;
        self.error = None;
        let runtime = self.runtime.clone();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let engine = DatabaseEngine::connect(config).await?;
                    let tables = engine.list_tables().await?;
                    Ok::<usize, dbx_core::DbxError>(tables.len())
                })
                .await?;
            this.update(cx, |this, cx| {
                if this.test_generation != generation {
                    return;
                }
                if this.draft_test_fingerprint(cx).ok().as_ref() != Some(&fingerprint) {
                    this.testing_connection = false;
                    this.show_toast(
                        ToastKind::Info,
                        "Connection details changed. Test again.",
                        cx,
                    );
                    this.error = None;
                    cx.notify();
                    return;
                }
                this.testing_connection = false;
                match result {
                    Ok(table_count) => {
                        this.show_toast(
                            ToastKind::Success,
                            format!(
                                "Connection succeeded · {table_count} {}",
                                if table_count == 1 { "table" } else { "tables" }
                            ),
                            cx,
                        );
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn begin_new_connection(&mut self, cx: &mut Context<Self>) {
        self.cancel_credential_hydration();
        self.compact_connection_form_open = true;
        self.draft.selected_profile = None;
        self.draft.environment = ConnectionEnvironment::default();
        self.draft.connection_name.update(cx, |name, cx| {
            name.clear();
            cx.notify();
        });
        self.hydrate_connection_fields(
            DatabaseKind::SQLite,
            Self::default_url(DatabaseKind::SQLite).to_owned(),
            cx,
        );
        self.connection_picker_open = true;
        self.error = None;
        cx.notify();
    }

    pub(super) fn show_saved_connections(&mut self, cx: &mut Context<Self>) {
        self.cancel_credential_hydration();
        self.compact_connection_form_open = false;
        self.draft.selected_profile = None;
        self.draft.password.update(cx, |value, cx| {
            value.zeroize();
            cx.notify();
        });
        self.error = None;
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn saved_transports_hydrate_and_changes_invalidate_connection_tests(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        let mut config = ConnectionConfig::new(
            DatabaseKind::PostgreSQL,
            "postgres://developer@localhost:5432/app",
        );
        config.socket = Some("/var/run/postgresql".into());
        config.ssh = Some(dbx_core::SshConfig {
            host: "bastion.example".into(),
            port: 2222,
            username: "developer".into(),
            identity_file: None,
        });
        let profile = store
            .save(ConnectionProfileDraft::from_config(
                "Socket via SSH",
                config.clone(),
            ))
            .unwrap();
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.profile_store = Some(store);
                app.vault_state = Some(VaultState::Unlocked);
                app.connection_picker_open = true;
                app.select_saved_connection(profile, cx);
                assert_eq!(app.resolve_draft(cx).unwrap().2, config);
                let original = app.draft_test_fingerprint(cx).unwrap();
                app.draft.transport.ssh_port.update(cx, |value, cx| {
                    *value = "22".into();
                    cx.notify();
                });
                assert_ne!(original, app.draft_test_fingerprint(cx).unwrap());
                app.set_connection_form_mode(ConnectionFormMode::ConnectionString, cx);
                assert!(app.resolve_draft(cx).unwrap().2.ssh.is_some());
            });
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("connection-transport-form").is_some());
        cx.simulate_resize(gpui::Size {
            width: px(640.),
            height: px(600.),
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.compact_layout = true;
                app.compact_connection_form_open = true;
                cx.notify();
            })
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let compact_bounds = cx
            .debug_bounds("connection-transport-form")
            .expect("transport fields remain visible in the compact connection form");
        assert!(compact_bounds.size.width >= px(250.));
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.begin_new_connection(cx);
                assert!(!app.draft.transport.ssh_enabled);
                assert!(!app.draft.transport.socket_enabled);
            });
        });
    }

    #[test]
    fn hydration_only_applies_to_the_selected_unchanged_draft() {
        let profile = Uuid::new_v4();
        let fields = ConnectionFields::new(DatabaseKind::PostgreSQL);
        assert!(hydration_matches_current_draft(
            Some(profile),
            Some(profile),
            &fields,
            &fields,
        ));
        assert!(!hydration_matches_current_draft(
            Some(Uuid::new_v4()),
            Some(profile),
            &fields,
            &fields,
        ));
        let mut edited_fields = fields.clone();
        edited_fields.host = "edited-host".into();
        assert!(!hydration_matches_current_draft(
            Some(profile),
            Some(profile),
            &edited_fields,
            &fields,
        ));
    }

    #[test]
    fn saved_connection_double_click_opens_while_single_click_selects() {
        assert_eq!(
            saved_connection_click_action(1),
            SavedConnectionClickAction::Select
        );
        assert_eq!(
            saved_connection_click_action(2),
            SavedConnectionClickAction::Open
        );
    }

    #[test]
    fn compact_layout_prioritizes_saved_connections_until_the_form_is_requested() {
        assert!(compact_connection_picker_visible(true, false, 2));
        assert!(!compact_connection_picker_visible(true, true, 2));
        assert!(!compact_connection_picker_visible(true, false, 0));
        assert!(!compact_connection_picker_visible(false, false, 2));
    }
}
