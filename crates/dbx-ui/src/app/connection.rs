use super::*;

fn hydration_matches_current_draft(
    selected_profile: Option<Uuid>,
    expected_profile: Option<Uuid>,
    current_fields: &ConnectionFields,
    requested_fields: &ConnectionFields,
) -> bool {
    selected_profile == expected_profile && current_fields == requested_fields
}

impl DbxApp {
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
        let config = ConnectionConfig::new(kind, visible_url.clone());
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

    pub(super) fn submit_vault_passphrase(&mut self, creating: bool, cx: &mut Context<Self>) {
        let Some(vault) = self.profile_store.as_ref().and_then(ProfileStore::vault) else {
            self.set_error("Credential vault is unavailable".into());
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
        self.error = None;
        self.status = if creating {
            "Creating credential vault…"
        } else {
            "Unlocking credential vault…"
        }
        .into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    if creating {
                        vault.create(passphrase)
                    } else {
                        vault.unlock(passphrase)
                    }
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
                        this.status = if creating {
                            "Credential vault created and unlocked"
                        } else {
                            "Credential vault unlocked"
                        }
                        .into();
                        this.error = None;
                        let selected_with_secret = this.draft.selected_profile.filter(|id| {
                            this.saved_connections
                                .iter()
                                .find(|profile| profile.id == *id)
                                .is_some_and(SavedConnection::has_secret)
                        });
                        if let Some(profile_id) = selected_with_secret {
                            this.status =
                                "Credential vault unlocked · loading saved password…".into();
                            this.hydrate_saved_credential(profile_id, cx);
                        }
                    }
                    Err(_) => this.set_error(if creating {
                        "Could not create the credential vault.".into()
                    } else {
                        "Could not unlock the credential vault. Check the passphrase or vault data."
                            .into()
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
            self.set_error("Could not lock the credential vault".into());
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
        self.status = "Credential vault locked".into();
        self.error = None;
        cx.notify();
    }

    fn draft_test_fingerprint(
        &self,
        cx: &App,
    ) -> Result<(DatabaseKind, ConnectionFormMode, String, String), String> {
        let fields = self.connection_fields(cx);
        Ok((
            fields.kind,
            self.draft.mode,
            fields.url().map_err(|error| error.to_string())?,
            fields.redacted_url().map_err(|error| error.to_string())?,
        ))
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
        let has_saved_password = profile.has_secret();
        let profile_id = profile.id;
        self.draft.selected_profile = Some(profile.id);
        self.draft.environment = profile.environment;
        self.draft.connection_name.update(cx, |name, cx| {
            *name = profile.name;
            cx.notify();
        });
        self.hydrate_connection_fields(profile.kind, profile.url.clone(), cx);
        self.error = None;
        self.status = if has_saved_password {
            "Loading saved password…".into()
        } else {
            "Saved connection selected".into()
        };
        if has_saved_password {
            self.hydrate_saved_credential(profile_id, cx);
        }
        cx.notify();
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
            let result = runtime.spawn_blocking(move || store.load(profile_id)).await?;
            this.update(cx, |this, cx| {
                if this.credential_hydration_generation != hydration_generation { return; }
                this.credential_hydrating = false;
                if !hydration_matches_current_draft(
                    this.draft.selected_profile,
                    Some(profile_id),
                    &this.connection_fields(cx),
                    &requested_fields,
                ) {
                    this.status = "Connection details changed · saved password not restored".into();
                    this.error = None;
                    cx.notify();
                    return;
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
                        this.status = "Saved connection selected · password restored".into();
                        this.error = None;
                    }
                    Err(_) => this.set_error("Saved connection password is unavailable in the credential vault. Import old passwords or enter it once and Save.".into()),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        }).detach();
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
        let (kind, url) = match fields.url() {
            Ok(url) => (fields.kind, url),
            Err(error) => {
                self.set_error(error.to_string());
                cx.notify();
                return;
            }
        };
        let mut draft =
            ConnectionProfileDraft::new(name, kind, url).with_environment(self.draft.environment);
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
        self.status = "Saving connection…".into();
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
                        this.status = format!("Saved connection ‘{}’", profile.name);
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
                            this.status = format!("Selected {}", path.display());
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
        self.status = format!("Connecting to {kind}…");
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
            let initial = if let Some(table) = initial_table {
                let table_ref = table_ref(&table);
                let structure = engine.table_structure(&table_ref).await?;
                let mut result = if kind.is_sql() {
                    Some(
                        engine
                            .query_table(
                                &table_ref,
                                &[],
                                &[],
                                &[],
                                Some(table_browse_page(0)),
                                QueryOptions::default(),
                            )
                            .await?,
                    )
                } else {
                    Some(
                        engine
                            .query("SCAN 0 COUNT 100", QueryOptions::default())
                            .await?,
                    )
                };
                let has_next_page = if kind.is_sql() {
                    result
                        .as_mut()
                        .map(trim_table_browse_result)
                        .unwrap_or(false)
                } else {
                    false
                };
                Some((table_ref, structure, result, has_next_page))
            } else {
                None
            };
            Ok::<_, dbx_core::DbxError>((
                engine,
                tables,
                databases,
                current_database,
                schema_filter,
                initial,
            ))
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
                match result {
                    Ok((engine, tables, databases, current_database, schema_filter, initial)) => {
                        session.engine = Some(engine);
                        session.tables = tables;
                        session.databases = databases;
                        session.current_database = current_database;
                        session.schema_filter = schema_filter;
                        if let Some((table, structure, result, has_next_page)) = initial {
                            session.selected_table = Some(table.clone());
                            session.table_columns = structure.columns;
                            session.foreign_keys = structure.foreign_keys;
                            session.completion_columns.insert(
                                completion_table_key(&table),
                                session.table_columns.clone(),
                            );
                            session.table_page = 0;
                            session.table_has_next_page = has_next_page;
                            session.set_result(result, cx);
                            session.result_table = Some(table);
                        } else {
                            session.selected_table = None;
                            session.table_columns.clear();
                            session.completion_columns.clear();
                            session.table_page = 0;
                            session.table_has_next_page = false;
                            session.set_result(None, cx);
                            session.result_table = None;
                        }
                        session.status = format!("Connected to {kind}");
                        session.error = None;
                        session.pane = Pane::Data;
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Connection failed".into();
                    }
                }
                cx.notify();
                this.prefetch_completion_columns_for(session_id, cx);
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
        let (kind, _, config) = match self.resolve_draft(cx) {
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
        self.status = format!("Testing {kind} connection…");
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
                    this.status = "Connection details changed · test again".into();
                    this.error = None;
                    cx.notify();
                    return;
                }
                this.testing_connection = false;
                match result {
                    Ok(table_count) => {
                        this.status =
                            format!("Connection succeeded · {table_count} table(s) found");
                        this.error = None;
                    }
                    Err(error) => {
                        this.status = "Connection test failed".into();
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
