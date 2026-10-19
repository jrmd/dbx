use super::*;
use crate::workspace::SavedQuery;

impl DbxApp {
    pub(super) fn choose_sql_library_folder(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a folder of plaintext .sql files".into()),
        });
        cx.spawn(async move |this, cx| {
            match receiver.await {
                Ok(Ok(Some(paths))) => if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| {
                        match this.sql_library.choose_directory(path) {
                            Ok(()) => this.show_toast(ToastKind::Success, "SQL library folder selected; files open as drafts and never run automatically", cx),
                            Err(error) => this.show_toast(ToastKind::Error, error, cx),
                        }
                        cx.notify();
                    })?;
                },
                Ok(Ok(None)) => {},
                Ok(Err(error)) => { this.update(cx, |this, cx| this.show_toast(ToastKind::Error, error.to_string(), cx))?; },
                Err(error) => { this.update(cx, |this, cx| this.show_toast(ToastKind::Error, error.to_string(), cx))?; },
            }
            Ok::<(), anyhow::Error>(())
        }).detach();
    }

    pub(super) fn toggle_sql_file_favorite(&mut self, relative: PathBuf, cx: &mut Context<Self>) {
        match self.sql_library.toggle_favorite(&relative) {
            Ok(()) => self.show_toast(ToastKind::Success, "SQL file favorites updated", cx),
            Err(error) => self.show_toast(ToastKind::Error, error, cx),
        }
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
                    this.sql_library.invalidate();
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
