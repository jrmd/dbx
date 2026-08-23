use super::*;

impl DbxApp {
    pub(super) fn begin_database_export(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((kind, busy, engine, database, session_name, tables)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.busy,
                    session.engine.clone(),
                    session.current_database.clone(),
                    session.name.clone(),
                    session.tables.clone(),
                )
            })
        else {
            return;
        };
        if busy || engine.is_none() || !kind.is_sql() {
            return;
        }
        let tables: Vec<_> = tables
            .into_iter()
            .filter(|table| table.kind == EntityKind::Table)
            .collect();
        if tables.is_empty() {
            if let Some(session) = self.session_mut(session_id) {
                session.status = "No tables are available to export".into();
                session.error = None;
            }
            cx.notify();
            return;
        }
        let selected_tables = tables.iter().map(table_selection_key).collect();
        let base_name = database
            .or_else(|| (!session_name.trim().is_empty()).then_some(session_name))
            .unwrap_or_else(|| "database".into());
        let output_name = cx.new(|_| format!("{}_export", transfer_name_stem(&base_name)));
        let output_name_editor =
            cx.new(|cx| TextEditor::new(output_name.clone(), false, window, cx));
        let output_name_subscription = cx.observe(&output_name, |_, _, cx| cx.notify());
        let focus = output_name_editor.read(cx).focus_handle();
        let output_directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));

        self.table_context_menu = None;
        self.database_export_dialog = Some(DatabaseExportDialog {
            session_id,
            tables,
            selected_tables,
            format: DumpFormat::Sql,
            schema_only: false,
            gzipped: false,
            output_directory,
            output_name,
            output_name_editor,
            _output_name_subscription: output_name_subscription,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn set_database_export_format(
        &mut self,
        format: DumpFormat,
        cx: &mut Context<Self>,
    ) {
        if let Some(dialog) = self.database_export_dialog.as_mut() {
            dialog.format = format;
            if format != DumpFormat::Sql {
                dialog.schema_only = false;
            }
            cx.notify();
        }
    }

    pub(super) fn toggle_database_export_table(&mut self, key: String, cx: &mut Context<Self>) {
        if let Some(dialog) = self.database_export_dialog.as_mut() {
            if !dialog.selected_tables.remove(&key) {
                dialog.selected_tables.insert(key);
            }
            cx.notify();
        }
    }

    pub(super) fn toggle_all_database_export_tables(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.database_export_dialog.as_mut() {
            if dialog.selected_tables.len() == dialog.tables.len() {
                dialog.selected_tables.clear();
            } else {
                dialog.selected_tables = dialog.tables.iter().map(table_selection_key).collect();
            }
            cx.notify();
        }
    }

    pub(super) fn toggle_database_export_schema_only(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.database_export_dialog.as_mut()
            && dialog.format == DumpFormat::Sql
        {
            dialog.schema_only = !dialog.schema_only;
            cx.notify();
        }
    }

    pub(super) fn toggle_database_export_gzip(&mut self, cx: &mut Context<Self>) {
        if let Some(dialog) = self.database_export_dialog.as_mut() {
            dialog.gzipped = !dialog.gzipped;
            cx.notify();
        }
    }

    pub(super) fn choose_database_export_directory(&mut self, cx: &mut Context<Self>) {
        if self.database_export_dialog.is_none() {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(SharedString::from("Choose export folder")),
        });
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.update(cx, |this, cx| {
                            if let Some(dialog) = this.database_export_dialog.as_mut() {
                                dialog.output_directory = path;
                                cx.notify();
                            }
                        })?;
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open the folder picker: {error}"));
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Folder picker closed unexpectedly: {error}"));
                        cx.notify();
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn cancel_database_export(&mut self, cx: &mut Context<Self>) {
        if self.database_export_dialog.take().is_some() {
            self.error = None;
            cx.notify();
        }
    }

    pub(super) fn execute_database_export(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.database_export_dialog.as_ref() else {
            return;
        };
        let selected_tables: Vec<TableRef> = dialog
            .tables
            .iter()
            .filter(|table| dialog.selected_tables.contains(&table_selection_key(table)))
            .map(table_ref)
            .collect();
        if selected_tables.is_empty() {
            self.set_error("Select at least one table to export".into());
            cx.notify();
            return;
        }
        let output_name = dialog.output_name.read(cx).trim().to_owned();
        if output_name.is_empty() {
            self.set_error("Enter an output name".into());
            cx.notify();
            return;
        }
        let session_id = dialog.session_id;
        let request = DatabaseExportRequest {
            tables: selected_tables,
            output_directory: dialog.output_directory.clone(),
            output_name,
            format: dialog.format,
            schema_only: dialog.schema_only,
            gzipped: dialog.gzipped,
        };
        let Some((engine, kind, busy)) = self
            .session(session_id)
            .map(|session| (session.engine.clone(), session.kind, session.busy))
        else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if busy || !kind.is_sql() {
            return;
        }

        self.database_export_dialog = None;
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = "Exporting database…".into();
        session.request_generation += 1;
        let generation = session.request_generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { export_database(&engine, &request).await })
                .await
                .unwrap_or_else(|error| Err(dbx_core::DbxError::Io(error.to_string())));
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                match result {
                    Ok(summary) => {
                        session.error = None;
                        let mode = if summary.schema_only {
                            "schema"
                        } else {
                            "data"
                        };
                        session.status = format!(
                            "Exported {} table(s) · {} row(s) · {} {} file{}",
                            summary.tables_exported,
                            summary.rows_exported,
                            mode,
                            summary.files_written,
                            if summary.files_written == 1 { "" } else { "s" }
                        );
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Database export failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn begin_database_import(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let available = self.session(session_id).is_some_and(|session| {
            session.kind.is_sql() && !session.busy && session.engine.is_some()
        });
        if !available {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from("Choose database SQL dump")),
        });
        cx.spawn_in(window, async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.update_in(cx, |this, window, cx| {
                            this.confirm_database_import(session_id, path, window, cx);
                        })?;
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open the file picker: {error}"));
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

    fn confirm_database_import(
        &mut self,
        session_id: SessionId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file_format = match detect_file_format(&path) {
            Ok(file_format) => file_format,
            Err(error) => {
                self.set_error(error.to_string());
                cx.notify();
                return;
            }
        };
        if file_format.format != DumpFormat::Sql {
            self.set_error(
                "Database imports require an SQL dump. CSV and TSV files import into one table from its context menu.".into(),
            );
            cx.notify();
            return;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "database dump".into());
        let connection_name = self
            .session(session_id)
            .and_then(|session| session.current_database.clone())
            .unwrap_or_else(|| "the active database".into());
        let receiver = window.prompt(
            PromptLevel::Warning,
            "Run this database SQL dump?",
            Some(format!(
                "Every statement in ‘{file_name}’ will run against {connection_name}. Review the file first if you did not create it."
            ).as_str()),
            &[
                PromptButton::cancel("Cancel"),
                PromptButton::ok("Run dump"),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if matches!(receiver.await, Ok(1)) {
                this.update(cx, |this, cx| {
                    this.execute_database_import(session_id, path, cx)
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn execute_database_import(
        &mut self,
        session_id: SessionId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some((engine, kind, busy)) = self
            .session(session_id)
            .map(|session| (session.engine.clone(), session.kind, session.busy))
        else {
            return;
        };
        let Some(engine) = engine else {
            return;
        };
        if busy || !kind.is_sql() {
            return;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "database dump".into());
        let runtime = self.runtime.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = format!("Importing {file_name}…");
        session.request_generation += 1;
        let generation = session.request_generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { import_database(&engine, &path).await })
                .await
                .unwrap_or_else(|error| Err(dbx_core::DbxError::Io(error.to_string())));
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                match result {
                    Ok(report) => {
                        session.error = None;
                        session.status = format!(
                            "Imported {} statement(s) from {file_name}",
                            report.statements_executed
                        );
                        this.refresh_tables_for(session_id, cx);
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Database import failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn transfer_available_for(&self, session_id: SessionId, table: &TableInfo) -> bool {
        self.session(session_id).is_some_and(|session| {
            session.kind.is_sql()
                && !session.busy
                && session.engine.is_some()
                && table.kind == EntityKind::Table
        })
    }

    /// Ask where to save an export. The chosen extension decides the format,
    /// so suggesting `.sql` produces a SQL dump while the user may freely
    /// type `.csv`, `.tsv`, or a `.gz` variant instead.
    pub(super) fn begin_table_export(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        cx: &mut Context<Self>,
    ) {
        if !self.transfer_available_for(session_id, &table) {
            return;
        }
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let suggested = format!("{}.sql", export_file_stem(&table));
        let receiver = cx.prompt_for_new_path(&directory, Some(suggested.as_str()));
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(path))) => {
                    this.update(cx, |this, cx| {
                        this.execute_table_export(session_id, table, path, cx)
                    })?;
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open the save dialog: {error}"));
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Save dialog closed unexpectedly: {error}"));
                        cx.notify();
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn execute_table_export(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy || !session.kind.is_sql() || table.kind != EntityKind::Table {
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let target = table_ref(&table);
        let runtime = self.runtime.clone();
        let destination = path.display().to_string();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = format!("Exporting {}…", table.name);
        session.request_generation += 1;
        let generation = session.request_generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { export_table(&engine, &target, &path).await })
                .await
                .unwrap_or_else(|error| Err(dbx_core::DbxError::Io(error.to_string())));
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                match result {
                    Ok(summary) => {
                        session.error = None;
                        session.status = format!(
                            "Exported {} row(s) to {}",
                            summary.rows_exported, destination
                        );
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Export failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Pick a file to import into `table`. SQL dumps run against the whole
    /// connection; CSV/TSV files append rows to the right-clicked table.
    pub(super) fn begin_table_import(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.transfer_available_for(session_id, &table) {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(SharedString::from("Choose import file")),
        });
        cx.spawn_in(window, async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.update_in(cx, |this, window, cx| {
                            this.confirm_table_import(session_id, table, path, window, cx);
                        })?;
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.set_error(format!("Could not open the file picker: {error}"));
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

    fn confirm_table_import(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file_format = match detect_file_format(&path) {
            Ok(file_format) => file_format,
            Err(error) => {
                self.set_error(error.to_string());
                cx.notify();
                return;
            }
        };
        let connection_name = self
            .session(session_id)
            .map(|session| session.name.clone())
            .unwrap_or_default();
        let qualified_name = table_sidebar_label(&table, None);
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (message, detail, confirmation) = match file_format.format {
            dbx_core::DumpFormat::Sql => (
                "Run this SQL dump?".to_owned(),
                format!(
                    "Every statement in ‘{file_name}’ will be executed against connection \
                     ‘{connection_name}’. Review the file first if you did not create it."
                ),
                "Run dump",
            ),
            dbx_core::DumpFormat::Csv | dbx_core::DumpFormat::Tsv => (
                format!("Append rows to {qualified_name}?"),
                format!(
                    "‘{file_name}’ ({}) will be inserted into {qualified_name}. Its first row \
                     must contain column names that match the table.",
                    file_format.format
                ),
                "Import data",
            ),
        };
        let receiver = window.prompt(
            PromptLevel::Warning,
            &message,
            Some(&detail),
            &[
                PromptButton::cancel("Cancel"),
                PromptButton::ok(confirmation),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if matches!(receiver.await, Ok(1)) {
                this.update(cx, |this, cx| {
                    this.execute_table_import(session_id, table, path, cx)
                })?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn execute_table_import(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        if session.busy || !session.kind.is_sql() || table.kind != EntityKind::Table {
            return;
        }
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let target = table_ref(&table);
        let runtime = self.runtime.clone();
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let imported_table = target.clone();
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        session.busy = true;
        session.error = None;
        session.status = format!("Importing {}…", file_name);
        session.request_generation += 1;
        let generation = session.request_generation;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn(async move { import_file(&engine, Some(&target), &path).await })
                .await
                .unwrap_or_else(|error| Err(dbx_core::DbxError::Io(error.to_string())));
            this.update(cx, |this, cx| {
                let Some(session) = this.session_mut(session_id) else {
                    return;
                };
                if generation != session.request_generation {
                    return;
                }
                session.busy = false;
                match result {
                    Ok(report) => {
                        session.error = None;
                        // Reload the grid when the imported table is open so
                        // new rows appear without a manual refresh.
                        let imported_table_open =
                            session.selected_table.as_ref() == Some(&imported_table);
                        if imported_table_open {
                            this.refresh_table_for(session_id, cx);
                        }
                        if let Some(session) = this.session_mut(session_id) {
                            session.status = if report.statements_executed > 0 {
                                format!(
                                    "Ran {} statement(s) from {}",
                                    report.statements_executed, file_name
                                )
                            } else {
                                format!(
                                    "Imported {} row(s) from {}",
                                    report.rows_inserted, file_name
                                )
                            };
                        }
                    }
                    Err(error) => {
                        session.error = Some(error.to_string());
                        session.status = "Import failed".into();
                    }
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
}
