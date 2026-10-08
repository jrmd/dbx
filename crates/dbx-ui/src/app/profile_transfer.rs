use super::*;
use zeroize::Zeroizing;

pub(super) struct ProfileTransferDialog {
    mode: u8, // 0 password-free export, 1 encrypted export, 2 reviewed import
    path: Option<PathBuf>,
    passphrase: Entity<TextEditor>,
    confirmation: Entity<TextEditor>,
    preview: Option<crate::profile_transfer::ImportPreview>,
    pub(super) busy: bool,
    error: Option<String>,
}
impl DbxApp {
    pub(super) fn open_profile_transfer(
        &mut self,
        mode: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vault_state != Some(VaultState::Unlocked) {
            return;
        }
        let passphrase = cx.new(|cx| TextEditor::empty(false, window, cx).password());
        let confirmation = cx.new(|cx| TextEditor::empty(false, window, cx).password());
        passphrase.read(cx).focus_handle().focus(window, cx);
        self.profile_transfer_dialog = Some(ProfileTransferDialog {
            mode,
            path: None,
            passphrase,
            confirmation,
            preview: None,
            busy: false,
            error: None,
        });
        cx.notify();
    }
    fn choose_profile_transfer_path(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.profile_transfer_dialog else {
            return;
        };
        let mode = dialog.mode;
        if mode == 2 {
            let receiver = cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Choose DBX or TablePro profile export".into()),
            });
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(paths))) = receiver.await
                    && let Some(path) = paths.into_iter().next()
                {
                    this.update(cx, |this, cx| {
                        if let Some(dialog) = &mut this.profile_transfer_dialog {
                            dialog.path = Some(path);
                            dialog.preview = None;
                            dialog.error = None;
                        }
                        cx.notify();
                    })?;
                }
                Ok::<(), anyhow::Error>(())
            })
            .detach();
        } else {
            let directory = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));
            let receiver = cx.prompt_for_new_path(
                &directory,
                Some(if mode == 1 {
                    "connections.dbxbundle"
                } else {
                    "connections.dbx.json"
                }),
            );
            cx.spawn(async move |this, cx| {
                if let Ok(Ok(Some(path))) = receiver.await {
                    this.update(cx, |this, cx| {
                        if let Some(dialog) = &mut this.profile_transfer_dialog {
                            dialog.path = Some(path);
                        }
                        cx.notify();
                    })?;
                }
                Ok::<(), anyhow::Error>(())
            })
            .detach();
        }
    }
    fn execute_profile_transfer(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &mut self.profile_transfer_dialog else {
            return;
        };
        if dialog.busy {
            return;
        }
        let Some(path) = dialog.path.clone() else {
            dialog.error = Some("Choose a file first".into());
            cx.notify();
            return;
        };
        let passphrase = Zeroizing::new(dialog.passphrase.read(cx).text(cx));
        let mode = dialog.mode;
        if mode == 1
            && (passphrase.chars().count() < 12
                || *passphrase != dialog.confirmation.read(cx).text(cx))
        {
            dialog.error = Some("Use a matching passphrase of at least 12 characters".into());
            cx.notify();
            return;
        }
        let Some(store) = self.profile_store.clone() else {
            return;
        };
        let preview = dialog.preview.take();
        dialog.busy = true;
        dialog.error = None;
        let runtime = self.runtime.clone();
        cx.spawn(async move |this, cx| {
            let result = runtime
                .spawn_blocking(move || {
                    if mode == 2 {
                        match preview {
                            Some(preview) => crate::profile_transfer::import(
                                &store,
                                preview.profiles,
                            )
                            .map(|count| (None, format!("Imported {count} protected connections"))),
                            None => {
                                let bytes = crate::profile_transfer::read_file(&path)?;
                                crate::profile_transfer::decode(&bytes, &passphrase)
                                    .map(|preview| (Some(preview), String::new()))
                            }
                        }
                    } else {
                        crate::profile_transfer::export(
                            &store,
                            &path,
                            (mode == 1).then_some(passphrase.as_str()),
                        )
                        .map(|()| (None, "Exported connections".into()))
                    }
                })
                .await?;
            this.update(cx, |this, cx| {
                let Some(dialog) = &mut this.profile_transfer_dialog else {
                    return;
                };
                dialog.busy = false;
                match result {
                    Ok((Some(preview), _)) => dialog.preview = Some(preview),
                    Ok((None, message)) => {
                        this.profile_transfer_dialog = None;
                        if let Some(store) = &this.profile_store
                            && let Ok(profiles) = store.list()
                        {
                            this.saved_connections = profiles;
                        }
                        this.show_toast(ToastKind::Success, message, cx);
                    }
                    Err(error) => dialog.error = Some(error),
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }
    pub(super) fn render_profile_transfer(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = &self.profile_transfer_dialog else {
            return div().into_any_element();
        };
        let title = match dialog.mode {
            0 => "Export password-free connections",
            1 => "Export encrypted connections and credentials",
            _ => "Review connection import",
        };
        div().absolute().inset_0().bg(theme().overlay).flex().items_center().justify_center()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .child(glass_raised(div(), RADIUS_GLASS).w(px(660.)).max_w(relative(0.95)).max_h(relative(0.9)).id("profile-transfer-dialog").overflow_y_scroll().p(px(20.)).flex().flex_col().gap(px(12.))
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_sm().child(match dialog.mode {
                    0 => "Includes names, hosts, tags, TLS and SSH file references. Passwords and unknown URL options are excluded. Keep this file private.",
                    1 => "Includes database and SSH credentials, protected by a separate passphrase. Key and certificate files are not copied.",
                    _ => "Choose a DBX or TablePro export. Imported profiles start protected and never connect automatically. TablePlus migration uses pasted connection URLs.",
                }))
                .child(button("profile-transfer-path", dialog.path.as_ref().map(|path| path.display().to_string()).unwrap_or_else(|| "Choose file…".into()), ButtonKind::Quiet).disabled(dialog.busy).on_click(cx.listener(|this, _, _, cx| this.choose_profile_transfer_path(cx))))
                .when(dialog.mode != 0 && dialog.preview.is_none(), |view| view
                    .child(div().text_sm().child(if dialog.mode == 1 { "New bundle passphrase (12+ characters)" } else { "File passphrase (leave empty for plaintext exports)" }))
                    .child(editor::input(dialog.passphrase.clone(), dialog.passphrase.read(cx).focus_handle(), false))
                    .when(dialog.mode == 1, |view| view.child(div().text_sm().child("Confirm bundle passphrase")).child(editor::input(dialog.confirmation.clone(), dialog.confirmation.read(cx).focus_handle(), false))))
                .when_some(dialog.preview.as_ref(), |view, preview| view.child(div().id("profile-import-review").max_h(px(280.)).overflow_y_scroll().flex().flex_col().gap(px(8.))
                    .children(preview.profiles.iter().map(|profile| div().text_sm().child(crate::profile_transfer::review_label(profile))))
                    .children(preview.warnings.iter().map(|warning| div().text_sm().text_color(theme().text_muted).child(warning.clone())))))
                .when_some(dialog.error.clone(), |view, error| view.child(div().text_sm().text_color(theme().danger).child(error)))
                .child(div().flex().justify_end().gap(px(8.))
                    .child(button("profile-transfer-cancel", "Cancel", ButtonKind::Quiet).disabled(dialog.busy).on_click(cx.listener(|this, _, window, cx| { this.profile_transfer_dialog = None; this.focus_handle.focus(window, cx); cx.notify(); })))
                    .child(button("profile-transfer-run", if dialog.busy { "Working…" } else if dialog.mode == 2 && dialog.preview.is_none() { "Preview import" } else if dialog.mode == 2 { "Import protected profiles" } else { "Export" }, ButtonKind::Primary).disabled(dialog.busy).on_click(cx.listener(|this, _, _, cx| this.execute_profile_transfer(cx))))))
            .into_any_element()
    }
}
