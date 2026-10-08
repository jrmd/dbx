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
        let (host, port, user, key, jump) = ssh
            .map(|ssh| {
                (
                    ssh.host,
                    ssh.port.to_string(),
                    ssh.username,
                    ssh.identity_file
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    ssh.jump_host.unwrap_or_default(),
                )
            })
            .unwrap_or_else(|| {
                (
                    String::new(),
                    "22".into(),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            });
        for (entity, text) in [
            (&self.draft.transport.socket, socket),
            (&self.draft.transport.ssh_host, host),
            (&self.draft.transport.ssh_port, port),
            (&self.draft.transport.ssh_user, user),
            (&self.draft.transport.ssh_key, key),
            (&self.draft.transport.ssh_jump, jump),
            (&self.draft.transport.ssh_password, String::new()),
        ] {
            entity.update(cx, |value, cx| {
                *value = text;
                cx.notify();
            });
        }
    }

    fn default_url(kind: DatabaseKind) -> &'static str {
        kind.default_url()
    }

    fn hydrate_connection_fields(
        &mut self,
        kind: DatabaseKind,
        url: String,
        cx: &mut Context<Self>,
    ) {
        let mut fields =
            ConnectionFields::from_url(url.clone()).unwrap_or_else(|_| ConnectionFields::new(kind));
        fields.kind = kind;
        let mut normalized_url = fields.url().unwrap_or(url);
        if matches!(
            kind,
            DatabaseKind::BigQuery | DatabaseKind::Turso | DatabaseKind::CloudflareD1
        ) && let Ok(mut parsed) = url::Url::parse(&normalized_url)
        {
            let _ = parsed.set_password(None);
            normalized_url = parsed.into();
        }
        self.draft.kind = kind;
        self.hydrate_transport(None, None, cx);
        let simple_address = url::Url::parse(&normalized_url)
            .is_ok_and(|url| url.query().is_none() && url.scheme() == kind.scheme());
        self.draft.mode = if kind.supports_details() && simple_address {
            ConnectionFormMode::Details
        } else {
            ConnectionFormMode::ConnectionString
        };
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
        fields.read_only = self.draft.read_only;
        fields.host = self.draft.host.read(cx).clone();
        fields.port = self.draft.port.read(cx).clone();
        fields.username = self.draft.username.read(cx).clone();
        fields.password = self.draft.password.read(cx).clone();
        fields.database = self.draft.database.read(cx).clone();
        let transport = &self.draft.transport;
        fields.socket_enabled = transport.socket_enabled;
        fields.socket = transport.socket.read(cx).clone();
        fields.ssh_password = (transport.ssh_enabled
            && !transport.ssh_password.read(cx).is_empty())
        .then(|| transport.ssh_password.read(cx).clone());
        fields.ssh = transport.ssh_enabled.then(|| dbx_core::SshConfig {
            host: transport.ssh_host.read(cx).trim().to_owned(),
            port: transport.ssh_port.read(cx).trim().parse().unwrap_or(0),
            username: transport.ssh_user.read(cx).trim().to_owned(),
            identity_file: (!transport.ssh_key.read(cx).trim().is_empty())
                .then(|| transport.ssh_key.read(cx).trim().into()),
            jump_host: (!transport.ssh_jump.read(cx).trim().is_empty())
                .then(|| transport.ssh_jump.read(cx).trim().to_owned()),
        });
        if self.draft.mode == ConnectionFormMode::ConnectionString
            || !self.draft.kind.supports_details()
        {
            fields.connection_string = self.draft.connection_url.read(cx).clone();
        } else {
            fields.use_structured_fields();
            // Keep TLS, replica-set and driver options while editing ordinary
            // address fields. Rebuilding only the authority must not weaken TLS.
            if let (Ok(mut rebuilt), Ok(original)) = (
                fields.url().and_then(|u| {
                    url::Url::parse(&u)
                        .map_err(|_| crate::connection_fields::ConnectionFieldsError::InvalidUrl)
                }),
                url::Url::parse(self.draft.connection_url.read(cx)),
            ) && self.draft.kind.accepts_scheme(original.scheme())
            {
                rebuilt.set_query(original.query());
                if matches!(
                    self.draft.kind,
                    DatabaseKind::Elasticsearch | DatabaseKind::ClickHouse
                ) {
                    if self.draft.kind == DatabaseKind::ClickHouse
                        && original.scheme() != "clickhouse"
                    {
                        rebuilt = url::Url::parse(&format!(
                            "{}://{}",
                            original.scheme(),
                            rebuilt.as_str().split_once("://").unwrap().1
                        ))
                        .expect("validated HTTP connection URL");
                    }
                    let _ = rebuilt.set_scheme(original.scheme());
                }
                fields.connection_string = rebuilt.into();
            }
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
        if self.vault_state != Some(VaultState::Unlocked) {
            return Err("Unlock the vault before connecting".into());
        }
        let (kind, visible_url) = self.draft_connection(cx)?;
        let config = self.connection_fields(cx).config()?;
        Ok((kind, visible_url, config))
    }

    pub(super) fn fetch_cloud_token(
        &mut self,
        provider: dbx_core::CloudAuthentication,
        cx: &mut Context<Self>,
    ) {
        let config = match self.connection_fields(cx).config() {
            Ok(config) => config,
            Err(error) => {
                self.show_toast(ToastKind::Error, error, cx);
                return;
            }
        };
        let requested = self.connection_fields(cx);
        let runtime = self.runtime.clone();
        let task = runtime
            .spawn(async move { dbx_core::cloud_database_password(&config, provider).await });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.vault_state != Some(VaultState::Unlocked)
                    || this.connection_fields(cx) != requested
                {
                    return;
                }
                match result {
                    Ok(Ok(token)) => {
                        this.draft.password.update(cx, |password, cx| {
                            password.zeroize();
                            *password = token;
                            cx.notify();
                        });
                        this.show_toast(
                            ToastKind::Success,
                            "Cloud token ready. Connect now; fetch a new token after it expires.",
                            cx,
                        );
                    }
                    Ok(Err(error)) => this.show_toast(ToastKind::Error, error.to_string(), cx),
                    Err(_) => this.show_toast(ToastKind::Error, "Cloud token request failed", cx),
                }
            });
        })
        .detach();
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

    pub(super) fn has_pending_lock_work(&self) -> bool {
        self.testing_connection
            || self.credential_hydrating
            || self.sessions.iter().any(|session| {
                session.busy
                    || session.background_tasks.has_pending()
                    || session.secondary_tabs.iter().any(|tab| match &tab.kind {
                        SecondaryTabKind::Query(query) => {
                            query.busy || query.in_transaction || query.agent.is_busy()
                        }
                        SecondaryTabKind::Data(data) => data.busy || data.has_unsaved_cell_work(),
                        SecondaryTabKind::Diagram(diagram) => diagram.busy,
                        SecondaryTabKind::Structure(structure) => structure.busy,
                    })
            })
    }

    pub(super) fn request_lock_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vault_state != Some(VaultState::Unlocked)
            || self.vault_busy
            || self.saving_connection
        {
            return;
        }
        if self.has_pending_lock_work() {
            let focus = cx.focus_handle();
            self.confirmation_dialog = Some(ConfirmationDialog {
                title: "Cancel work and lock?".into(),
                detail: "Locking will cancel running work, discard unsaved cell edits, and close all connections.".into(),
                confirm_label: "Cancel work and lock",
                tone: ConfirmationTone::Warning,
                action: ConfirmationAction::LockVault,
                focus: focus.clone(),
                return_focus: window.focused(cx),
                sql: None,
            });
            focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.lock_vault(cx);
        self.vault_editors
            .passphrase_editor
            .read(cx)
            .focus_handle()
            .focus(window, cx);
    }

    pub(super) fn lock_vault(&mut self, cx: &mut Context<Self>) {
        if self.saving_connection {
            return;
        }
        let Some(vault) = self.profile_store.as_ref().and_then(ProfileStore::vault) else {
            return;
        };
        let recovery_error = self.flush_query_workspaces(cx);
        if vault.lock().is_err() {
            self.set_error("Couldn’t lock the vault".into());
            cx.notify();
            return;
        }
        self.vault_state = Some(VaultState::Locked);
        self.workspace_documents.clear();
        // Use the normal teardown so queries, connection attempts, and tab
        // tasks are cancelled and their database engines are released.
        while let Some(session) = self.sessions.last() {
            self.close_session(session.id, cx);
        }
        self.table_context_menu = None;
        self.database_export_dialog = None;
        self.confirmation_dialog = None;
        self.mutation_error_dialog = None;
        self.settings_open = false;
        self.toasts.clear();
        self.test_generation = self.test_generation.saturating_add(1);
        self.connection_test_abort.cancel();
        self.testing_connection = false;
        self.cancel_credential_hydration();
        self.draft.password.update(cx, |value, cx| {
            value.zeroize();
            cx.notify();
        });
        if !self.draft.kind.is_file() {
            self.draft.connection_url.update(cx, |value, cx| {
                value.zeroize();
                cx.notify();
            });
        }
        self.draft.import_url.update(cx, |value, cx| {
            value.zeroize();
            cx.notify();
        });
        self.clear_vault_inputs(cx);
        self.error = recovery_error;
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
            ConnectionFormMode::Details if self.draft.kind.supports_details() => {
                let connection_string = match self.connection_fields(cx).url() {
                    Ok(url) => url,
                    Err(error) => {
                        self.set_error(error.to_string());
                        cx.notify();
                        return;
                    }
                };
                if connection_string.starts_with("mongodb+srv:") {
                    self.set_error("SRV connections use DNS rather than a fixed host/port; edit this URI in Connection string mode".into());
                    cx.notify();
                    return;
                }
                let mut fields = match ConnectionFields::from_url(connection_string) {
                    Ok(mut fields)
                        if fields.kind == self.draft.kind
                            || self.draft.kind.accepts_scheme(fields.kind.scheme()) =>
                    {
                        fields.kind = self.draft.kind;
                        fields
                    }
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
        self.draft.read_only = false;
        self.draft.choosing_kind = false;
        self.hydrate_connection_fields(kind, Self::default_url(kind).to_owned(), cx);
        self.error = None;
        cx.notify();
    }

    /// The type a pasted connection URL implies, if DBX recognises it.
    pub(super) fn import_url_kind(&self, cx: &App) -> Option<DatabaseKind> {
        let url = self.draft.import_url.read(cx);
        ConnectionFields::from_url(url.trim())
            .ok()
            .map(|fields| fields.kind)
    }

    pub(super) fn continue_with_import_url(&mut self, cx: &mut Context<Self>) {
        let Some(kind) = self.import_url_kind(cx) else {
            return;
        };
        let url = self.draft.import_url.update(cx, |value, cx| {
            cx.notify();
            std::mem::take(value)
        });
        self.cancel_credential_hydration();
        self.draft.selected_profile = None;
        self.draft.read_only = false;
        self.draft.choosing_kind = false;
        self.hydrate_connection_fields(kind, url.trim().to_owned(), cx);
        self.error = None;
        cx.notify();
    }

    pub(super) fn change_connection_kind(&mut self, cx: &mut Context<Self>) {
        self.draft.choosing_kind = true;
        self.error = None;
        cx.notify();
    }

    /// A connection carries at most one tag; choosing the current tag again
    /// clears it.
    pub(super) fn select_tag(&mut self, tag: ConnectionTag, cx: &mut Context<Self>) {
        if self
            .draft
            .tag
            .as_ref()
            .is_some_and(|item| item.id == tag.id)
        {
            self.draft.tag = None;
        } else {
            self.draft.tag = Some(tag);
        }
        cx.notify();
    }

    pub(super) fn edit_tag(&mut self, tag: Option<ConnectionTag>, cx: &mut Context<Self>) {
        self.tag_editor.editing = tag.as_ref().map(|tag| tag.id);
        self.tag_editor.creating = false;
        self.tag_editor.deleting = None;
        self.tag_editor.name.update(cx, |value, cx| {
            *value = tag.as_ref().map(|tag| tag.name.clone()).unwrap_or_default();
            cx.notify();
        });
        self.tag_editor.color.update(cx, |value, cx| {
            *value = format!(
                "{:06X}",
                tag.as_ref().map(|tag| tag.color).unwrap_or(0x82aaff)
            );
            cx.notify();
        });
        cx.notify();
    }

    pub(super) fn new_tag(&mut self, cx: &mut Context<Self>) {
        self.edit_tag(None, cx);
        self.tag_editor.creating = true;
    }

    /// Deleting a tag also untags every connection that used it.
    pub(super) fn delete_connection_tag(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(store) = self.profile_store.as_ref() else {
            self.show_toast(
                ToastKind::Error,
                "Connection profile storage is unavailable",
                cx,
            );
            return;
        };
        match store.delete_tag(id) {
            Ok(tags) => {
                self.connection_tags = tags;
                let current = std::iter::once(&mut self.draft.tag)
                    .chain(self.sessions.iter_mut().map(|session| &mut session.tag));
                for existing in current {
                    if existing.as_ref().is_some_and(|tag| tag.id == id) {
                        *existing = None;
                    }
                }
                if let Ok(profiles) = store.list() {
                    self.saved_connections = profiles;
                }
                self.edit_tag(None, cx);
            }
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
        cx.notify();
    }

    pub(super) fn save_connection_tag(&mut self, cx: &mut Context<Self>) {
        let raw = self
            .tag_editor
            .color
            .read(cx)
            .trim()
            .trim_start_matches('#');
        let color = if raw.len() == 6 {
            u32::from_str_radix(raw, 16).ok()
        } else {
            None
        };
        let Some(color) = color else {
            self.show_toast(
                ToastKind::Error,
                "Enter a six-digit hex colour, such as #82AAFF",
                cx,
            );
            return;
        };
        let tag = ConnectionTag {
            id: self.tag_editor.editing.unwrap_or_else(Uuid::new_v4),
            name: self.tag_editor.name.read(cx).trim().to_owned(),
            color,
        };
        let Some(store) = self.profile_store.as_ref() else {
            self.show_toast(
                ToastKind::Error,
                "Connection profile storage is unavailable",
                cx,
            );
            return;
        };
        match store.save_tag(tag.clone()) {
            Ok(tags) => {
                self.connection_tags = tags;
                let current = std::iter::once(&mut self.draft.tag)
                    .chain(self.sessions.iter_mut().map(|session| &mut session.tag));
                for existing in current.flatten() {
                    if existing.id == tag.id {
                        *existing = tag.clone();
                    }
                }
                if let Ok(profiles) = store.list() {
                    self.saved_connections = profiles;
                }
                self.edit_tag(None, cx);
            }
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
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
        self.draft.choosing_kind = false;
        self.draft.tag = profile.tag;
        self.draft.connection_name.update(cx, |name, cx| {
            *name = profile.name;
            cx.notify();
        });
        self.hydrate_connection_fields(profile.kind, profile.url.clone(), cx);
        self.draft.read_only = profile.read_only;
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
                        this.draft.transport.ssh_password.update(cx, |value, cx| {
                            value.zeroize();
                            *value = loaded.config.ssh_password.clone().unwrap_or_default();
                            cx.notify();
                        });
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

    fn connection_profile_draft(
        &self,
        fields: &ConnectionFields,
        cx: &App,
    ) -> Result<ConnectionProfileDraft, String> {
        // Resolve the effective credential before the store extracts it. A
        // hidden Details field must not override a newly pasted URL password.
        let mut draft = ConnectionProfileDraft::from_config(
            self.draft.connection_name.read(cx).trim().to_owned(),
            fields.config()?,
        )
        .with_tag(self.draft.tag.clone());
        if let Some(id) = self.draft.selected_profile {
            draft = draft.with_id(id);
        }
        Ok(draft)
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
        let fields = self.connection_fields(cx);
        let mut requested_fields = fields.clone();
        let draft = match self.connection_profile_draft(&fields, cx) {
            Ok(draft) => draft,
            Err(error) => {
                self.set_error(error.to_string());
                cx.notify();
                return;
            }
        };
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
                        for session in &mut this.sessions {
                            if session.profile_id == Some(profile.id) {
                                session.tag = profile.tag.clone();
                            }
                        }
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
        let requested_kind = self.draft.kind;
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
                            if this.draft.kind != requested_kind || !requested_kind.is_file() {
                                return;
                            }
                            this.draft.selected_profile = None;
                            let url = if requested_kind == DatabaseKind::DuckDB {
                                sqlite_url(&path).replacen("sqlite:", "duckdb:", 1)
                            } else {
                                sqlite_url(&path)
                            };
                            this.hydrate_connection_fields(requested_kind, url, cx);
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
        let tag = self.draft.tag.clone();
        let mut session =
            ConnectionSession::new(session_id, profile_id, name, kind, tag, window, cx);
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
                let _ = initial_table;
                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
                if connected {
                    this.load_schema_objects_for(session_id, cx);
                    this.prefetch_redis_command_catalog_for(session_id, cx);
                    this.restore_query_workspace_for(session_id, window, cx);
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
        let task = runtime.spawn(async move {
            let engine = DatabaseEngine::connect(config).await?;
            let tables = engine.list_tables().await?;
            Ok::<usize, dbx_core::DbxError>(tables.len())
        });
        self.connection_test_abort.replace(task.abort_handle());
        cx.spawn(async move |this, cx| {
            let result = task.await?;
            this.update(cx, |this, cx| {
                if this.test_generation != generation {
                    return;
                }
                this.connection_test_abort.clear();
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
        self.draft.read_only = false;
        self.draft.tag = Some(default_tags().remove(3));
        self.draft.choosing_kind = true;
        self.settings_open = false;
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
        self.draft.read_only = false;
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
    fn locking_with_open_connections_removes_workspace_and_cancels_requests(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        let vault = store.vault().unwrap();
        vault.create("test vault passphrase").unwrap();
        let session_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(DbxApp::new);
        let (runtime, request, connection_request, test_request, engine) =
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.profile_store = Some(store);
                    app.vault_state = Some(VaultState::Unlocked);
                    app.hydrate_connection_fields(
                        DatabaseKind::SQLite,
                        "sqlite::memory:".into(),
                        cx,
                    );
                    app.draft.import_url.update(cx, |value, _| {
                        *value = "postgres://user:secret@localhost/db".into()
                    });
                    let test_request = app.runtime.spawn(std::future::pending::<()>());
                    app.connection_test_abort
                        .replace(test_request.abort_handle());
                    app.testing_connection = true;
                    let engine = Arc::new(
                        app.runtime
                            .block_on(DatabaseEngine::connect(ConnectionConfig::new(
                                DatabaseKind::SQLite,
                                "sqlite::memory:",
                            )))
                            .unwrap(),
                    );
                    let mut session = ConnectionSession::new(
                        session_id,
                        None,
                        "Lock test".into(),
                        DatabaseKind::SQLite,
                        None,
                        window,
                        cx,
                    );
                    session.engine = Some(engine.clone());
                    let tab_id = Uuid::new_v4();
                    let mut query =
                        QueryTab::new(DatabaseKind::SQLite, session_id, tab_id, window, cx);
                    let request = app.runtime.spawn(std::future::pending::<()>());
                    query.abort_handle.replace(request.abort_handle());
                    query.busy = true;
                    session.secondary_tabs.push(SecondaryTab {
                        id: tab_id,
                        kind: SecondaryTabKind::Query(Box::new(query)),
                    });
                    session.active_secondary_tab = Some(tab_id);
                    session.pane = Pane::Query;
                    let mut connecting = ConnectionSession::new(
                        Uuid::new_v4(),
                        None,
                        "Connecting".into(),
                        DatabaseKind::SQLite,
                        None,
                        window,
                        cx,
                    );
                    connecting.busy = true;
                    let connection_request = app.runtime.spawn(std::future::pending::<()>());
                    connecting.track_background_task(&connection_request);
                    app.sessions = vec![session, connecting];
                    app.active_session_id = Some(session_id);
                    app.connection_picker_open = false;
                    (
                        app.runtime.clone(),
                        request,
                        connection_request,
                        test_request,
                        Arc::downgrade(&engine),
                    )
                })
            });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("connection-tab").is_some());

        for (settings_open, connection_picker_open) in
            [(false, false), (false, true), (true, false)]
        {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.settings_open = settings_open;
                    app.connection_picker_open = connection_picker_open;
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            let lock = cx.debug_bounds("lock-vault").expect("Lock is visible");
            cx.simulate_click(lock.center(), gpui::Modifiers::none());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(vault.state(), VaultState::Unlocked);
            assert!(cx.debug_bounds("confirmation-dialog").is_some());
            assert!(!request.is_finished());
            assert!(!connection_request.is_finished());
            assert!(!test_request.is_finished());
            if connection_picker_open {
                cx.simulate_keystrokes("escape");
            } else {
                let cancel = cx.debug_bounds("cancel-confirmation").unwrap();
                cx.simulate_click(cancel.center(), gpui::Modifiers::none());
            }
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("confirmation-dialog").is_none());
            assert_eq!(vault.state(), VaultState::Unlocked);
            assert!(!request.is_finished());
            assert!(!connection_request.is_finished());
            assert!(!test_request.is_finished());
            app.read_with(cx, |app, _| {
                assert_eq!(app.sessions.len(), 2);
                assert_eq!(app.active_session_id, Some(session_id));
                assert!(app.confirmation_dialog.is_none());
            });
        }
        cx.simulate_resize(gpui::size(px(720.), px(640.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let lock = cx.debug_bounds("lock-vault").expect("Lock is visible");
        cx.simulate_click(lock.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let confirm = cx.debug_bounds("confirm-action").unwrap();
        cx.simulate_click(confirm.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(vault.state(), VaultState::Locked);
        assert!(
            cx.debug_bounds("vault-gate").is_some(),
            "Lock must show the unlock screen even with an open connection"
        );
        assert!(cx.debug_bounds("connection-tab").is_none());
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(
                    app.sessions.is_empty(),
                    "Locked connections must be inaccessible"
                );
                assert!(app.active_session_id.is_none());
                assert!(app.draft.import_url.read(cx).is_empty());
                assert!(!app.testing_connection);
                app.connect(window, cx);
                app.test_connection(cx);
                app.open_settings(cx);
                assert!(
                    app.sessions.is_empty(),
                    "Connecting while locked must be rejected"
                );
                assert!(!app.testing_connection);
                assert!(!app.settings_open);
                assert_eq!(
                    app.error.as_deref(),
                    Some("Unlock the vault before connecting")
                );
                assert!(
                    app.vault_editors
                        .passphrase_editor
                        .read(cx)
                        .focus_handle()
                        .is_focused(window)
                );
            });
        });
        assert!(
            engine.upgrade().is_none(),
            "Lock must release the database engine"
        );
        assert!(runtime.block_on(request).unwrap_err().is_cancelled());
        assert!(
            runtime
                .block_on(connection_request)
                .unwrap_err()
                .is_cancelled()
        );
        assert!(runtime.block_on(test_request).unwrap_err().is_cancelled());
    }

    #[gpui::test]
    fn inactive_query_work_warns_and_idle_connections_lock_without_warning(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        let vault = store.vault().unwrap();
        vault.create("test vault passphrase").unwrap();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.profile_store = Some(store);
            app.vault_state = Some(VaultState::Unlocked);
            let session_id = Uuid::new_v4();
            app.sessions.push(ConnectionSession::new(
                session_id,
                None,
                "Idle connection".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            ));
            app.active_session_id = Some(session_id);
            assert!(!app.has_pending_lock_work());
            let tab_id = Uuid::new_v4();
            let mut query = QueryTab::new(DatabaseKind::SQLite, session_id, tab_id, window, cx);
            query.busy = true;
            app.sessions[0].secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Query(Box::new(query)),
            });
            assert!(
                app.has_pending_lock_work(),
                "Work in an inactive tab must be detected"
            );
            app
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let lock = cx.debug_bounds("lock-vault").unwrap();
        cx.simulate_click(lock.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(vault.state(), VaultState::Unlocked);
        assert!(cx.debug_bounds("confirmation-dialog").is_some());
        let cancel = cx.debug_bounds("cancel-confirmation").unwrap();
        cx.simulate_click(cancel.center(), gpui::Modifiers::none());
        app.update(cx, |app, _| {
            let SecondaryTabKind::Query(query) = &mut app.sessions[0].secondary_tabs[0].kind else {
                panic!("query tab");
            };
            query.busy = false;
            let completed = app.runtime.spawn(async {});
            app.sessions[0].track_background_task(&completed);
            app.runtime.block_on(completed).unwrap();
            assert!(
                !app.has_pending_lock_work(),
                "Completed background tasks must not warn"
            );
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let lock = cx.debug_bounds("lock-vault").unwrap();
        cx.simulate_click(lock.center(), gpui::Modifiers::none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(vault.state(), VaultState::Locked);
        assert!(cx.debug_bounds("confirmation-dialog").is_none());
        assert!(cx.debug_bounds("vault-gate").is_some());
        assert!(app.read_with(cx, |app, _| app.sessions.is_empty()));
    }

    #[gpui::test]
    fn locked_vault_takes_precedence_over_workspace_picker_and_settings(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Locked);
            let session_id = Uuid::new_v4();
            app.sessions.push(ConnectionSession::new(
                session_id,
                None,
                "Existing connection".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            ));
            app.active_session_id = Some(session_id);
            app
        });
        for (settings_open, connection_picker_open) in
            [(false, false), (false, true), (true, false)]
        {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.settings_open = settings_open;
                    app.connection_picker_open = connection_picker_open;
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("vault-gate").is_some());
            assert!(cx.debug_bounds("connection-tab").is_none());
        }
    }

    #[gpui::test]
    fn saved_postgres_connection_string_uses_restored_vault_password(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        store
            .vault()
            .unwrap()
            .create("test vault passphrase")
            .unwrap();
        let original = "postgresql://alice:fixture%25password@localhost:5432/app?sslmode=require";
        let profile = store
            .save(ConnectionProfileDraft::new(
                "Postgres",
                DatabaseKind::PostgreSQL,
                original,
            ))
            .unwrap();
        let loaded = store.load(profile.id).unwrap();
        let (app, cx) = cx.add_window_view(DbxApp::new);
        app.update(cx, |app, cx| {
            app.vault_state = Some(VaultState::Unlocked);
            app.hydrate_connection_fields(profile.kind, profile.url.clone(), cx);
            assert_eq!(app.draft.mode, ConnectionFormMode::ConnectionString);
            let mut restored = ConnectionFields::from_url(loaded.config.url).unwrap();
            app.draft.password.update(cx, |value, cx| {
                *value = std::mem::take(&mut restored.password);
                cx.notify();
            });
            let config = app.resolve_draft(cx).unwrap().2;
            let resolved = ConnectionFields::from_url(config.url).unwrap();
            assert_eq!(resolved.password, "fixture%password");
            assert!(resolved.connection_string.contains("sslmode=require"));
            app.set_connection_form_mode(ConnectionFormMode::Details, cx);
            assert_eq!(app.draft.password.read(cx), "fixture%password");
            app.set_connection_form_mode(ConnectionFormMode::ConnectionString, cx);
            assert_eq!(
                ConnectionFields::from_url(app.resolve_draft(cx).unwrap().2.url)
                    .unwrap()
                    .password,
                "fixture%password"
            );
        });
    }

    #[gpui::test]
    fn saving_pasted_postgres_url_preserves_its_password_over_old_details(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::at(directory.path().join("connections.json"));
        store
            .vault()
            .unwrap()
            .create("test vault passphrase")
            .unwrap();
        let (app, cx) = cx.add_window_view(DbxApp::new);
        app.update(cx, |app, cx| {
            app.draft.connection_name.update(cx, |value, cx| {
                *value = "Postgres".into();
                cx.notify();
            });
            app.hydrate_connection_fields(
                DatabaseKind::PostgreSQL,
                "postgres://alice:old-password@localhost/app".into(),
                cx,
            );
            app.set_connection_form_mode(ConnectionFormMode::ConnectionString, cx);
            app.draft.connection_url.update(cx, |value, cx| {
                *value = "postgresql://alice:new%25password@localhost/app?sslmode=require".into();
                cx.notify();
            });
            let draft = app
                .connection_profile_draft(&app.connection_fields(cx), cx)
                .unwrap();
            let profile = store.save(draft).unwrap();
            let loaded = store.load(profile.id).unwrap();
            assert_eq!(
                ConnectionFields::from_url(loaded.config.url)
                    .unwrap()
                    .password,
                "new%password"
            );
            assert!(!profile.url.contains("password"));
        });
    }

    #[gpui::test]
    fn provider_forms_preserve_tls_and_keep_api_tokens_in_masked_fields(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.vault_state = Some(VaultState::Unlocked);
                for kind in DatabaseKind::ALL {
                    app.hydrate_connection_fields(kind, kind.default_url().into(), cx);
                    assert_eq!(app.connection_fields(cx).config().unwrap().kind, kind);
                    let tab = QueryTab::new(kind, Uuid::new_v4(), Uuid::new_v4(), window, cx);
                    assert_eq!(tab.query_text.read(cx), kind.default_query());
                }
                let url = "postgres://root@localhost:26257/app?sslmode=require";
                app.hydrate_connection_fields(DatabaseKind::CockroachDB, url.into(), cx);
                app.set_connection_form_mode(ConnectionFormMode::Details, cx);
                assert_eq!(app.connection_fields(cx).url().unwrap(), url);
                for (kind, url) in [
                    (
                        DatabaseKind::BigQuery,
                        "bigquery://:fixture-token@project/dataset?location=EU",
                    ),
                    (
                        DatabaseKind::Turso,
                        "libsql://:fixture-token@database.turso.io",
                    ),
                    (
                        DatabaseKind::CloudflareD1,
                        "d1://:fixture-token@account/database",
                    ),
                ] {
                    app.hydrate_connection_fields(kind, url.into(), cx);
                    assert!(!app.draft.connection_url.read(cx).contains("fixture-token"));
                    assert_eq!(app.draft.password.read(cx), "fixture-token");
                    assert!(
                        app.connection_fields(cx)
                            .url()
                            .unwrap()
                            .contains("fixture-token")
                    );
                }
            });
        });
    }

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
            jump_host: None,
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
