//! Explicit registration and selection of community driver manifests.
use super::*;
use dbx_core::driver_plugin::DriverManifest;
use std::hash::{BuildHasher, Hash, Hasher};
use std::io::Write;

type DriverRegistration = (PathBuf, Result<DriverManifest, String>);

#[derive(Default)]
pub(super) struct DriverRegistry {
    entries: Vec<DriverRegistration>,
    loading: bool,
    reload_requested: bool,
    initialized: bool,
    error: Option<String>,
}
fn directory() -> PathBuf {
    crate::config::directory()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("drivers")
}
fn registered(directory: &std::path::Path) -> Result<Vec<DriverRegistration>, String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Cannot read drivers folder: {error}")),
    };
    let mut manifests = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        if manifests.len() == 256 {
            return Err("More than 256 driver manifests. Open the drivers folder to remove unused registrations.".into());
        }
        let manifest = DriverManifest::read(&path).map_err(|error| error.to_string());
        manifests.push((path, manifest));
    }
    manifests.sort_by_key(|(path, manifest)| {
        manifest
            .as_ref()
            .map(|manifest| manifest.name.clone())
            .unwrap_or_else(|_| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
    });
    Ok(manifests)
}
fn template(manifest: &DriverManifest) -> String {
    format!("{}://localhost/database", manifest.schemes[0])
}
fn preserve_url(value: &str, previous_template: &str, native_template: &str) -> bool {
    !value.trim().is_empty() && value != previous_template && value != native_template
}
// Keep no plaintext secret snapshot alive while registration performs filesystem IO.
struct DriverDraftGuard {
    hash_builder: std::collections::hash_map::RandomState,
    fingerprint: u64,
}
impl DriverDraftGuard {
    fn capture(app: &DbxApp, cx: &Context<DbxApp>) -> Self {
        let mut guard = Self {
            hash_builder: Default::default(),
            fingerprint: 0,
        };
        guard.fingerprint = guard.fingerprint(app, cx);
        guard
    }
    fn fingerprint(&self, app: &DbxApp, cx: &Context<DbxApp>) -> u64 {
        let draft = &app.draft;
        let mut state = self.hash_builder.build_hasher();
        app.credential_hydration_generation.hash(&mut state);
        app.profile_store
            .as_ref()
            .and_then(ProfileStore::vault)
            .map(|vault| vault.path().to_owned())
            .hash(&mut state);
        app.connection_picker_open.hash(&mut state);
        app.settings_open.hash(&mut state);
        draft.kind.hash(&mut state);
        (draft.mode == ConnectionFormMode::ConnectionString).hash(&mut state);
        draft.selected_profile.hash(&mut state);
        draft.driver_manifest.hash(&mut state);
        draft.choosing_kind.hash(&mut state);
        draft.read_only.hash(&mut state);
        format!("{:?}", draft.cloud_auth).hash(&mut state);
        draft
            .tag
            .as_ref()
            .map(|tag| (&tag.id, &tag.name, tag.color))
            .hash(&mut state);
        draft.transport.socket_enabled.hash(&mut state);
        draft.transport.ssh_enabled.hash(&mut state);
        for value in [
            &draft.connection_name,
            &draft.connection_url,
            &draft.import_url,
            &draft.folder,
            &draft.host,
            &draft.port,
            &draft.username,
            &draft.password,
            &draft.database,
            &draft.transport.socket,
            &draft.transport.ssh_host,
            &draft.transport.ssh_port,
            &draft.transport.ssh_user,
            &draft.transport.ssh_key,
            &draft.transport.ssh_jump,
            &draft.transport.ssh_password,
        ] {
            value.read(cx).hash(&mut state);
        }
        state.finish()
    }
    fn matches(&self, app: &DbxApp, cx: &Context<DbxApp>) -> bool {
        app.vault_state == Some(VaultState::Unlocked)
            && self.fingerprint == self.fingerprint(app, cx)
    }
}

impl DbxApp {
    fn refresh_driver_registry(&mut self, cx: &mut Context<Self>) {
        if self.driver_registry.loading {
            self.driver_registry.reload_requested = true;
            return;
        }
        self.driver_registry.loading = true;
        self.driver_registry.initialized = true;
        let task = cx
            .background_executor()
            .spawn(async { registered(&directory()) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.driver_registry.loading = false;
                match result {
                    Ok(entries) => {
                        this.driver_registry.entries = entries;
                        this.driver_registry.error = None;
                    }
                    Err(error) => this.driver_registry.error = Some(error),
                }
                if std::mem::take(&mut this.driver_registry.reload_requested) {
                    this.refresh_driver_registry(cx);
                }
                cx.notify();
            })
        })
        .detach();
        cx.notify();
    }
    fn register_driver(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let guard = DriverDraftGuard::capture(self, cx);
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Register a trusted DBX driver manifest".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match receiver.await { Ok(Ok(Some(paths))) => paths, _ => return Ok::<(), anyhow::Error>(()) };
            let Some(source) = paths.into_iter().next() else { return Ok(()); };
            let result = cx.background_executor().spawn(async move {
                let manifest = DriverManifest::read(&source).map_err(|error| error.to_string())?;
                if !manifest.executable.is_file() { return Err("The manifest executable does not exist".to_owned()); }
                let path = directory().join(format!("{}.json", manifest.id));
                std::fs::create_dir_all(directory()).map_err(|error| error.to_string())?;
                let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
                let mut file = options.open(&path).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists { "A driver with this id is already registered. Remove its registration before replacing it.".into() } else { error.to_string() }
                })?;
                if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(error.to_string());
                }
                Ok((path, manifest))
            }).await;
            this.update_in(cx, |this, _, cx| match result {
                Ok((path, manifest)) => {
                    this.finish_driver_registration(guard, path, manifest, cx);
                }
                Err(error) => this.show_toast(ToastKind::Error, error, cx),
            })?;
            Ok(())
        }).detach();
    }
    fn finish_driver_registration(
        &mut self,
        guard: DriverDraftGuard,
        path: PathBuf,
        manifest: DriverManifest,
        cx: &mut Context<Self>,
    ) {
        let unchanged = guard.matches(self, cx);
        if unchanged {
            self.select_driver(path, manifest, cx);
        }
        self.refresh_driver_registry(cx);
        self.show_toast(
            ToastKind::Success,
            if unchanged {
                "Driver registered and selected."
            } else {
                "Driver registered. Select it from Community drivers when ready."
            },
            cx,
        );
    }
    fn select_driver(&mut self, path: PathBuf, manifest: DriverManifest, cx: &mut Context<Self>) {
        let previous = self
            .driver_registry
            .entries
            .iter()
            .find_map(|(path, manifest)| {
                (Some(path) == self.draft.driver_manifest.as_ref())
                    .then(|| manifest.as_ref().ok().map(template))
                    .flatten()
            })
            .unwrap_or_default();
        let old_name = self
            .driver_registry
            .entries
            .iter()
            .find_map(|(path, manifest)| {
                (Some(path) == self.draft.driver_manifest.as_ref())
                    .then(|| {
                        manifest
                            .as_ref()
                            .ok()
                            .map(|manifest| manifest.name.as_str())
                    })
                    .flatten()
            });
        let name = self.draft.connection_name.read(cx).clone();
        let keep_name = !name.trim().is_empty() && Some(name.as_str()) != old_name;
        let url = self.draft.connection_url.read(cx).clone();
        let keep_url = preserve_url(&url, &previous, self.draft.kind.default_url());
        self.select_kind(manifest.family, cx);
        self.draft.driver_manifest = Some(path);
        self.draft.mode = ConnectionFormMode::ConnectionString;
        self.draft.connection_name.update(cx, |value, cx| {
            *value = if keep_name {
                name
            } else {
                manifest.name.clone()
            };
            cx.notify();
        });
        self.draft.connection_url.update(cx, |value, cx| {
            *value = if keep_url { url } else { template(&manifest) };
            cx.notify();
        });
        cx.notify();
    }
    pub(super) fn remove_driver_registration(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Only remove the copied registration; never a driver executable or an arbitrary path.
        if path.parent() != Some(directory().as_path())
            || path.extension().and_then(|s| s.to_str()) != Some("json")
        {
            return;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                if self.draft.driver_manifest.as_ref() == Some(&path) {
                    self.draft.driver_manifest = None;
                    self.draft.choosing_kind = true;
                }
                self.refresh_driver_registry(cx);
                self.show_toast(ToastKind::Success, "Driver registration removed.", cx);
            }
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
    }
    pub(super) fn render_driver_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if !self.driver_registry.initialized {
            self.refresh_driver_registry(cx);
        }
        div().flex().flex_col().gap(crate::theme::ui_px(8.))
            .child(div().flex().flex_wrap().items_center().gap(crate::theme::ui_px(8.)).child("Community drivers")
                .child(button("register-driver", "Register trusted driver…", ButtonKind::Quiet).on_click(cx.listener(|this, _, window, cx| this.register_driver(window, cx))))
                .child(button("refresh-drivers", "Refresh", ButtonKind::Quiet).disabled(self.driver_registry.loading).on_click(cx.listener(|this, _, _, cx| this.refresh_driver_registry(cx))))
                .child(button("drivers-folder", "Show drivers folder", ButtonKind::Quiet).on_click(cx.listener(|this, _, _, cx| {
                    match std::fs::create_dir_all(directory()) { Ok(()) => cx.reveal_path(&directory()), Err(error) => this.show_toast(ToastKind::Error, error.to_string(), cx) }
                }))))
            .child(div().text_size(crate::theme::text_px(11.)).text_color(theme().text_muted).child("Driver programs run locally with the credentials of connections you open. Register manifests only from authors you trust."))
            .when(self.driver_registry.loading, |view| view.child("Loading drivers…"))
            .when_some(self.driver_registry.error.clone(), |view, error| view.child(div().text_color(theme().danger).child(error)))
            .children(self.driver_registry.entries.iter().enumerate().map(|(index, (path, result))| {
                let selected = self.draft.driver_manifest.as_ref() == Some(path);
                let label = result.as_ref().map(|manifest| manifest.name.clone()).unwrap_or_else(|_| path.file_name().unwrap_or_default().to_string_lossy().into_owned());
                let mut row = div().flex().items_center().gap(crate::theme::ui_px(8.));
                match result {
                    Ok(manifest) => {
                        let path = path.clone(); let manifest = manifest.clone();
                        row = row.child(segment(label.clone(), selected).id(("registered-driver", index)).pressable(label.clone()).role(gpui::Role::RadioButton).aria_selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_driver(path.clone(), manifest.clone(), cx))));
                    }
                    Err(error) => row = row.child(div().flex_1().text_color(theme().danger).child(format!("{label} · Unavailable: {error}"))),
                }
                let path = path.clone();
                row.child(button(("remove-driver", index), "Remove…", ButtonKind::Quiet).accessible_name(format!("Remove driver {label}")).on_click(cx.listener(move |this, _, window, cx| {
                    this.ask_to_delete(format!("Remove {label}?"), "Removes only this registration. Saved connections using it cannot reconnect until it is registered again. The driver program and database are kept.", ConfirmationAction::RemoveDriver { path: path.clone() }, window, cx);
                })))
            })).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn late_registration_preserves_changed_or_locked_drafts(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        app.update(cx, |app, cx| {
            app.vault_state = Some(VaultState::Unlocked);
            let manifest = DriverManifest {
                protocol: 1,
                id: "fixture".into(),
                name: "Fixture driver".into(),
                family: DatabaseKind::PostgreSQL,
                executable: "/fixture/driver".into(),
                arguments: Vec::new(),
                schemes: vec!["fixture".into()],
            };
            let path = PathBuf::from("/fixture/driver.json");
            let guard = DriverDraftGuard::capture(app, cx);
            app.draft.connection_url.update(cx, |url, cx| {
                *url = "postgres://changed@localhost/database".into();
                cx.notify();
            });
            app.draft.read_only = true;
            app.finish_driver_registration(guard, path.clone(), manifest.clone(), cx);
            assert_eq!(
                app.draft.connection_url.read(cx),
                "postgres://changed@localhost/database"
            );
            assert!(app.draft.read_only);
            assert!(app.draft.driver_manifest.is_none());
            let guard = DriverDraftGuard::capture(app, cx);
            app.vault_state = Some(VaultState::Locked);
            app.finish_driver_registration(guard, path.clone(), manifest.clone(), cx);
            assert!(app.draft.driver_manifest.is_none());
            assert!(app.draft.read_only);
            // Even lock/unlock or profile reselection back to identical visible values
            // invalidates a pending registration through the existing draft generation.
            app.vault_state = Some(VaultState::Unlocked);
            let guard = DriverDraftGuard::capture(app, cx);
            app.credential_hydration_generation += 1;
            assert!(!guard.matches(app, cx));
            let guard = DriverDraftGuard::capture(app, cx);
            app.finish_driver_registration(guard, path.clone(), manifest, cx);
            assert_eq!(app.draft.driver_manifest.as_ref(), Some(&path));
        });
    }
}
