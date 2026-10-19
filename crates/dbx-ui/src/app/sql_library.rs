use super::*;
use crate::workspace::SavedQuery;

impl DbxApp {
    pub(super) fn ensure_sql_library_loaded(&mut self, cx: &mut Context<Self>) {
        if self.sql_library.needs_refresh() {
            self.refresh_sql_library(cx);
        }
    }
    pub(super) fn refresh_sql_library(&mut self, cx: &mut Context<Self>) {
        if self.sql_library_refreshing {
            return;
        }
        self.sql_library_refreshing = true;
        self.sql_library_generation = self.sql_library_generation.wrapping_add(1);
        let generation = self.sql_library_generation;
        let library = self.sql_library.clone();
        let root = library.root().to_owned();
        let task = cx
            .background_executor()
            .spawn(async move { library.entries() });
        cx.spawn(async move |this, cx| {
            let entries = task.await;
            this.update(cx, |this, cx| {
                this.sql_library_refreshing = false;
                if this.sql_library.root() == root && this.sql_library_generation == generation {
                    this.sql_library.cache_entries(entries);
                }
                this.ensure_sql_library_loaded(cx);
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
    pub(super) fn browse_sql_files(
        &mut self,
        favorites: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_sql_library(cx);
        self.open_quick_open_action(&OpenQuickOpen, window, cx);
        if let Some(open) = &self.quick_open {
            open.query.update(cx, |editor, cx| {
                editor.set_text(if favorites { "favorites:" } else { "files:" }, cx)
            });
        }
    }
    pub(super) fn choose_sql_library_folder(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a folder of plaintext .sql files".into()),
        });
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        this.update(cx, |this, cx| {
                            match this.sql_library.choose_directory(path) {
                                Ok(()) => {
                                    this.refresh_sql_library(cx);
                                    this.show_toast(
                                        ToastKind::Success,
                                        "SQL library folder selected",
                                        cx,
                                    );
                                }
                                Err(error) => this.show_toast(ToastKind::Error, error, cx),
                            }
                            cx.notify();
                        })?;
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(ToastKind::Error, error.to_string(), cx)
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.show_toast(ToastKind::Error, error.to_string(), cx)
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn toggle_sql_file_favorite(&mut self, relative: PathBuf, cx: &mut Context<Self>) {
        if self.sql_library_favorite_pending {
            return;
        }
        self.sql_library_favorite_pending = true;
        self.sql_library_generation = self.sql_library_generation.wrapping_add(1);
        let library = self.sql_library.clone();
        let root = library.root().to_owned();
        let task = cx.background_executor().spawn(async move {
            library.toggle_favorite(&relative)?;
            library.entries()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.sql_library_favorite_pending = false;
                if this.sql_library.root() != root {
                    cx.notify();
                    return;
                }
                this.sql_library_generation = this.sql_library_generation.wrapping_add(1);
                match result {
                    Ok(entries) => this.sql_library.cache_entries(Ok(entries)),
                    Err(error) => this.show_toast(ToastKind::Error, error, cx),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_library_sql_for(
        &mut self,
        session_id: SessionId,
        relative: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let library = self.sql_library.clone();
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = runtime
                .spawn_blocking(move || {
                    library.read(&relative).map(|sql| SavedQuery {
                        name: relative
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        sql,
                    })
                })
                .await?;
            this.update_in(cx, |this, window, cx| match loaded {
                Ok(saved)
                    if this.can_open_connection_targets() && this.session(session_id).is_some() =>
                {
                    this.open_saved_query_for(session_id, saved, window, cx)
                }
                Ok(_) => {}
                Err(error) => this.show_toast(ToastKind::Error, error, cx),
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    pub(super) fn open_sql_file_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open a UTF-8 .sql file as a query draft".into()),
        });
        let runtime = self.runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            let loaded = match receiver.await {
                Ok(Ok(Some(paths))) => {
                    let Some(path) = paths.into_iter().next() else {
                        return Ok::<(), anyhow::Error>(());
                    };
                    runtime
                        .spawn_blocking(move || {
                            crate::query_library::read_sql(&path).map(|sql| SavedQuery {
                                name: path
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                sql,
                            })
                        })
                        .await?
                }
                Ok(Ok(None)) => return Ok(()),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            this.update_in(cx, |this, window, cx| match loaded {
                Ok(saved)
                    if this.can_open_connection_targets() && this.session(session_id).is_some() =>
                {
                    this.open_saved_query_for(session_id, saved, window, cx)
                }
                Ok(_) => {}
                Err(error) => this.show_toast(ToastKind::Error, error, cx),
            })?;
            Ok(())
        })
        .detach();
    }

    pub(super) fn save_query_sql_file_for(
        &mut self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let Some(query) = self.active_query_tab(session_id) else {
            return;
        };
        let name = crate::query_library::suggested_filename(query.name.read(cx));
        let sql = query.query_text.read(cx).clone();
        let directory = self.sql_library.root().to_owned();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            self.show_toast(ToastKind::Error, format!("SQL library folder: {error}"), cx);
            return;
        }
        let receiver = cx.prompt_for_new_path(&directory, Some(&name));
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = match receiver.await {
                Ok(Ok(Some(mut path))) => {
                    if path.extension().is_none() {
                        path.set_extension("sql");
                    }
                    let destination = path.display().to_string();
                    runtime
                        .spawn_blocking(move || {
                            crate::query_library::write_sql(&path, &sql).map(|()| destination)
                        })
                        .await?
                }
                Ok(Ok(None)) => return Ok::<(), anyhow::Error>(()),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            this.update(cx, |this, cx| match result {
                Ok(destination) => {
                    this.refresh_sql_library(cx);
                    this.show_toast(
                        ToastKind::Success,
                        format!("Saved plaintext SQL to {destination}"),
                        cx,
                    );
                }
                Err(error) => this.show_toast(ToastKind::Error, error, cx),
            })?;
            Ok(())
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn drain_library(app: &Entity<DbxApp>, cx: &mut gpui::VisualTestContext) {
        cx.run_until_parked();
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert!(
                !app.sql_library_refreshing && !app.sql_library_favorite_pending,
                "SQL library worker did not settle"
            );
        });
    }
    #[gpui::test]
    fn background_snapshots_ignore_old_folders_and_favorite_writes_are_serialized(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let old = tempfile::tempdir().unwrap();
        let current = tempfile::tempdir().unwrap();
        crate::query_library::write_sql(&old.path().join("old.sql"), "SELECT 1").unwrap();
        for file in ["one.sql", "two.sql"] {
            crate::query_library::write_sql(&current.path().join(file), "SELECT 2").unwrap();
        }
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.runtime = Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            );
            app.sql_library = crate::query_library::SqlLibrary::at(old.path().to_owned());
            app
        });
        app.update(cx, |app, cx| {
            app.refresh_sql_library(cx);
            app.sql_library = crate::query_library::SqlLibrary::at(current.path().to_owned());
            app.ensure_sql_library_loaded(cx);
        });
        drain_library(&app, cx);
        app.update(cx, |app, cx| {
            let entries = app.sql_library.cached_entries().unwrap();
            assert_eq!(entries.len(), 2);
            assert!(entries.iter().all(|entry| entry.label() != "old.sql"));
            app.refresh_sql_library(cx);
            app.toggle_sql_file_favorite("one.sql".into(), cx);
            assert!(app.sql_library_favorite_pending);
            app.toggle_sql_file_favorite("two.sql".into(), cx); // Disabled until the first write settles.
        });
        drain_library(&app, cx);
        app.update(cx, |app, cx| {
            let entries = app.sql_library.cached_entries().unwrap();
            assert!(
                entries
                    .iter()
                    .any(|entry| entry.label() == "one.sql" && entry.favorite)
            );
            assert!(
                entries
                    .iter()
                    .any(|entry| entry.label() == "two.sql" && !entry.favorite)
            );
            app.toggle_sql_file_favorite("two.sql".into(), cx);
        });
        drain_library(&app, cx);
        app.update(cx, |app, _| {
            let entries = app.sql_library.cached_entries().unwrap();
            assert!(entries.iter().all(|entry| entry.favorite));
            assert_eq!(
                entries
                    .iter()
                    .map(|entry| entry.favorite)
                    .collect::<Vec<_>>(),
                app.sql_library
                    .entries()
                    .unwrap()
                    .iter()
                    .map(|entry| entry.favorite)
                    .collect::<Vec<_>>()
            );
        });
    }
}
