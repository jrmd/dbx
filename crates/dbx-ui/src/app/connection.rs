use super::*;

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
        let fields =
            ConnectionFields::from_url(url.clone()).unwrap_or_else(|_| ConnectionFields::new(kind));
        let normalized_url = fields.url().unwrap_or(url);
        self.draft.kind = kind;
        self.draft.mode = ConnectionFormMode::Details;
        self.draft.connection_url.update(cx, |value, cx| {
            *value = normalized_url;
            cx.notify();
        });
        self.draft.host.update(cx, |value, cx| {
            *value = fields.host;
            cx.notify();
        });
        self.draft.port.update(cx, |value, cx| {
            *value = fields.port;
            cx.notify();
        });
        self.draft.username.update(cx, |value, cx| {
            *value = fields.username;
            cx.notify();
        });
        self.draft.password.update(cx, |value, cx| {
            *value = fields.password;
            cx.notify();
        });
        self.draft.database.update(cx, |value, cx| {
            *value = fields.database;
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

    /// Resolve the visible form through the profile store when it still names
    /// the selected profile. This preserves the password-free form URL while
    /// using the stored credential for connection operations.
    fn resolve_draft(&self, cx: &App) -> Result<(DatabaseKind, String, ConnectionConfig), String> {
        let (mut kind, visible_url) = self.draft_connection(cx)?;
        let config = if let (Some(store), Some(profile_id)) =
            (&self.profile_store, self.draft.selected_profile)
        {
            match store.get(profile_id).map_err(|error| error.to_string())? {
                Some(profile) if profile.kind == kind && profile.url == visible_url => {
                    let loaded = store.load(profile_id).map_err(|error| error.to_string())?;
                    kind = loaded.config.kind;
                    loaded.config
                }
                _ => ConnectionConfig::new(kind, visible_url.clone()),
            }
        } else {
            ConnectionConfig::new(kind, visible_url.clone())
        };
        Ok((kind, visible_url, config))
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
                let fields = match ConnectionFields::from_url(connection_string) {
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
                    *value = fields.host;
                    cx.notify();
                });
                self.draft.port.update(cx, |value, cx| {
                    *value = fields.port;
                    cx.notify();
                });
                self.draft.username.update(cx, |value, cx| {
                    *value = fields.username;
                    cx.notify();
                });
                self.draft.password.update(cx, |value, cx| {
                    *value = fields.password;
                    cx.notify();
                });
                self.draft.database.update(cx, |value, cx| {
                    *value = fields.database;
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
        let profile_id = profile.id;
        let has_secret = profile.has_secret();
        self.draft.selected_profile = Some(profile.id);
        self.draft.environment = profile.environment;
        self.draft.connection_name.update(cx, |name, cx| {
            *name = profile.name;
            cx.notify();
        });
        self.hydrate_connection_fields(profile.kind, profile.url.clone(), cx);
        self.error = None;
        self.status = if has_secret {
            "Saved connection selected · loading password…".into()
        } else {
            "Saved connection selected".into()
        };
        if has_secret && let Some(store) = self.profile_store.clone() {
            let runtime = self.runtime.clone();
            cx.spawn(async move |this, cx| {
                let loaded = runtime
                    .spawn_blocking(move || store.load(profile_id))
                    .await?;
                this.update(cx, |this, cx| {
                    if this.draft.selected_profile != Some(profile_id) {
                        return;
                    }
                    let password_is_empty = this.draft.password.read(cx).is_empty();
                    match loaded {
                        Ok(loaded) if password_is_empty => {
                            this.hydrate_connection_fields(
                                loaded.config.kind,
                                loaded.config.url,
                                cx,
                            );
                            this.error = None;
                            this.status = "Saved connection selected".into();
                        }
                        Ok(_) => {
                            // The user started editing before the keyring
                            // returned. Never overwrite their new password.
                            this.error = None;
                            this.status = "Saved connection selected".into();
                        }
                        Err(error) if password_is_empty => {
                            this.status = "Saved password unavailable · enter it again".into();
                            this.error = Some(error.to_string());
                        }
                        Err(_) => {}
                    }
                    cx.notify();
                })?;
                Ok::<(), anyhow::Error>(())
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn save_connection(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.profile_store.clone() else {
            self.set_error("Connection profile storage is unavailable".into());
            cx.notify();
            return;
        };
        let name = self.draft.connection_name.read(cx).trim().to_owned();
        let fields = self.connection_fields(cx);
        let (kind, url) = match fields.url() {
            Ok(url) => (fields.kind, url),
            Err(error) => {
                self.set_error(error.to_string());
                cx.notify();
                return;
            }
        };
        let mut draft = ConnectionProfileDraft::new(name, kind, url.clone())
            .with_environment(self.draft.environment);
        if !fields.password.is_empty() {
            draft = draft.with_secret(fields.password.clone());
        }
        if let Some(id) = self.draft.selected_profile {
            draft = draft.with_id(id);
        }
        match store.save(draft) {
            Ok(profile) => {
                self.draft.selected_profile = Some(profile.id);
                // Rehydrate from the entered URL rather than the scrubbed
                // profile so a newly entered password remains in the draft.
                self.hydrate_connection_fields(profile.kind, url, cx);
                match store.list() {
                    Ok(profiles) => self.saved_connections = profiles,
                    Err(error) => {
                        self.set_error(error.to_string());
                        cx.notify();
                        return;
                    }
                }
                self.error = None;
                self.status = format!("Saved connection ‘{}’", profile.name);
            }
            Err(error) => self.set_error(error.to_string()),
        }
        cx.notify();
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

    pub(super) fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move {
                    let engine = Arc::new(DatabaseEngine::connect(config).await?);
                    let tables = engine.list_tables().await?;
                    let databases = engine.list_databases().await.unwrap_or_default();
                    let current_database = engine.current_database().await.ok();
                    let schema_filter = default_schema_filter(kind, &tables);
                    let initial_table =
                        schema_filtered_tables(kind, &tables, schema_filter.as_deref())
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
                })
                .await?;

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
