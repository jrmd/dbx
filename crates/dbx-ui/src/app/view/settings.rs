use super::super::*;

const TAG_PALETTE: [u32; 8] = [
    0xef6b73, 0xe5b567, 0x8fcf9c, 0x82aaff, 0xb48ead, 0x56b6c2, 0xe89bb5, 0xaab2bf,
];

const SETTINGS_SECTIONS: [(SettingsSection, &str, Icon); 5] = [
    (SettingsSection::Appearance, "Appearance", Icon::Appearance),
    (
        SettingsSection::QueryAgent,
        "Query assistant",
        Icon::Sparkles,
    ),
    (SettingsSection::Connections, "Connections", Icon::Download),
    (SettingsSection::Tags, "Connection tags", Icon::Tag),
    (SettingsSection::Updates, "Updates", Icon::Download),
];

impl DbxApp {
    /// Settings take over the content area rather than floating above it, so
    /// longer sections such as tag management have room to breathe. Wide
    /// windows get a sidebar; compact ones a segmented control.
    pub(super) fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let compact = self.compact_layout;
        let padding = if compact { px(14.) } else { px(28.) };
        let selected = self.settings_section;
        let title = SETTINGS_SECTIONS
            .iter()
            .find(|(section, ..)| *section == selected)
            .map_or("Settings", |(_, label, _)| *label);
        let content = match selected {
            SettingsSection::Appearance => self.render_settings_appearance(cx).into_any_element(),
            SettingsSection::QueryAgent => self.render_agent_settings(cx).into_any_element(),
            SettingsSection::Connections => self.render_settings_connections(cx).into_any_element(),
            SettingsSection::Tags => self.render_settings_tags(cx).into_any_element(),
            SettingsSection::Updates => self.render_settings_updates(cx).into_any_element(),
        };
        let close = Button::new("close-settings")
            .with_size(Size::XSmall)
            .compact()
            .ghost()
            .tooltip("Back to workspace")
            .child(icon(Icon::Close, theme().text_muted))
            .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx)));
        let header = div()
            .flex_none()
            .h(px(52.))
            .px(padding)
            .border_b_1()
            .border_color(theme().hairline)
            .flex()
            .items_center()
            .gap(px(12.))
            .map(|view| {
                if compact {
                    view.child(
                        div()
                            .id("settings-navigation")
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .overflow_x_scroll()
                            .child(segmented_track().flex_none().children(
                                SETTINGS_SECTIONS.into_iter().map(|(section, label, _)| {
                                    segment(label, section == selected)
                                        .id(SharedString::from(format!("settings-section-{label}")))
                                        .pressable()
                                        .debug_selector(move || format!("settings-section-{label}"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.show_settings_section(section, cx)
                                        }))
                                }),
                            )),
                    )
                } else {
                    view.child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                }
            })
            .when(!compact, |view| view.child(div().flex_1()))
            .child(close);
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .pr(px(GLASS_INSET))
            .pb(px(GLASS_INSET))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .overflow_hidden()
                    .rounded(px(RADIUS_PANEL))
                    .border_1()
                    .border_color(theme().hairline)
                    .bg(theme().canvas)
                    .shadow(glass_shadow(10.))
                    .when(!compact, |view| {
                        view.child(self.render_settings_sidebar(cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(header)
                            .child(
                                div()
                                    .id("settings-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .p(padding)
                                    .flex()
                                    .justify_center()
                                    .items_start()
                                    .child(div().w_full().min_w_0().max_w(px(640.)).child(content)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_settings_sidebar(&mut self, cx: &mut Context<Self>) -> Div {
        let selected = self.settings_section;
        div()
            .w(px(208.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(10.))
            .border_r_1()
            .border_color(theme().hairline)
            .child(
                div()
                    .h(px(32.))
                    .px(px(8.))
                    .mb(px(8.))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme().text_muted)
                    .child("Settings"),
            )
            .children(SETTINGS_SECTIONS.into_iter().map(|(section, label, kind)| {
                let active = section == selected;
                div()
                    .id(SharedString::from(format!("settings-section-{label}")))
                    .pressable()
                    .debug_selector(move || format!("settings-section-{label}"))
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .when(active, |view| {
                        view.bg(theme().glass_selected)
                            .text_color(theme().text)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!active, |view| {
                        view.text_color(theme().text_muted)
                            .hover(|style| style.bg(theme().glass_hover).text_color(theme().text))
                    })
                    .child(icon(
                        kind,
                        if active {
                            theme().accent
                        } else {
                            theme().text_muted
                        },
                    ))
                    .child(label)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.show_settings_section(section, cx)),
                    )
            }))
    }

    fn show_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        if self.settings_section != section {
            self.settings_section = section;
            self.edit_tag(None, cx);
        }
        cx.notify();
    }

    /// Jump straight to tag management, e.g. from the connection form.
    pub(super) fn open_tag_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = SettingsSection::Tags;
        self.open_settings(cx);
    }

    fn render_settings_appearance(&mut self, cx: &mut Context<Self>) -> Div {
        let current = self.appearance;
        let appearance_choices = Appearance::ALL.into_iter().map(|option| {
            segment(option.label(), option == current)
                .id(SharedString::from(format!(
                    "settings-appearance-{}",
                    option.label()
                )))
                .pressable()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_appearance_preference(option, window, cx)
                }))
        });
        let mut rows = vec![
            settings_row(
                "Theme",
                None,
                segmented_track().children(appearance_choices),
            )
            .into_any_element(),
        ];
        if !cfg!(target_os = "macos") {
            rows.push(
                settings_row(
                    "Reduce transparency",
                    None,
                    gpui_component::switch::Switch::new("settings-reduce-transparency")
                        .checked(self.reduce_transparency)
                        .with_size(Size::Small)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_reduce_transparency(window, cx)
                        })),
                )
                .into_any_element(),
            );
        }
        div().child(settings_group(rows))
    }

    fn render_settings_connections(&mut self, cx: &mut Context<Self>) -> Div {
        let unavailable = self.vault_state != Some(VaultState::Unlocked) || self.vault_busy;
        div().child(settings_group(
            [
                (
                    0,
                    "Export profiles",
                    "Save connection details without passwords.",
                    "Export…",
                ),
                (
                    1,
                    "Export encrypted bundle",
                    "Back up connections and credentials with a separate passphrase.",
                    "Export…",
                ),
                (
                    2,
                    "Import profiles",
                    "Review a DBX or TablePro export before importing.",
                    "Import…",
                ),
            ]
            .into_iter()
            .map(|(mode, label, detail, action)| {
                settings_row(
                    label,
                    Some(detail.into()),
                    button(
                        SharedString::from(format!("profile-portability-{mode}")),
                        action,
                        ButtonKind::Quiet,
                    )
                    .debug_selector(move || format!("profile-portability-{mode}"))
                    .disabled(unavailable)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_profile_transfer(mode, window, cx)
                    })),
                )
                .into_any_element()
            }),
        ))
    }

    fn render_settings_tags(&mut self, cx: &mut Context<Self>) -> Div {
        let mut rows = self
            .connection_tags
            .clone()
            .into_iter()
            .map(|tag| {
                if self.tag_editor.editing == Some(tag.id) {
                    self.render_tag_form(cx).into_any_element()
                } else if self.tag_editor.deleting == Some(tag.id) {
                    self.render_tag_delete(&tag, cx).into_any_element()
                } else {
                    self.render_tag_row(&tag, cx).into_any_element()
                }
            })
            .collect::<Vec<_>>();
        rows.push(if self.tag_editor.creating {
            self.render_tag_form(cx).into_any_element()
        } else {
            div()
                .id("new-tag")
                .pressable()
                .debug_selector(|| "new-tag".into())
                .h(px(48.))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(13.))
                .text_color(theme().accent)
                .cursor_pointer()
                .hover(|style| style.bg(theme().glass_hover))
                .child(icon(Icon::Add, theme().accent))
                .child("New tag")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.new_tag(cx);
                    let focus = this.tag_editor.name_editor.read(cx).focus_handle();
                    focus.focus(window, cx);
                }))
                .into_any_element()
        });
        div().child(settings_group(rows))
    }

    fn tag_usage(&self, id: Uuid) -> usize {
        self.saved_connections
            .iter()
            .filter(|profile| profile.tag.as_ref().is_some_and(|tag| tag.id == id))
            .count()
    }

    fn render_tag_row(&self, tag: &ConnectionTag, cx: &mut Context<Self>) -> Div {
        let used = self.tag_usage(tag.id);
        let edit = tag.clone();
        let id = tag.id;
        settings_row(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .size(px(10.))
                        .flex_none()
                        .rounded_full()
                        .bg(gpui::rgb(tag.color)),
                )
                .child(tag.name.clone()),
            Some(connection_count(used).into()),
            div()
                .flex()
                .gap(px(2.))
                .child(
                    glass_icon_button(
                        SharedString::from(format!("edit-tag-{id}")),
                        Icon::Pencil,
                        false,
                    )
                    .tooltip(tip("Edit"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_tag(Some(edit.clone()), cx);
                        let focus = this.tag_editor.name_editor.read(cx).focus_handle();
                        focus.focus(window, cx);
                    })),
                )
                .child(
                    glass_icon_button(
                        SharedString::from(format!("delete-tag-{id}")),
                        Icon::Trash,
                        false,
                    )
                    .debug_selector(move || format!("delete-tag-{id}"))
                    .tooltip(tip("Delete"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_tag(None, cx);
                        this.tag_editor.deleting = Some(id);
                    })),
                ),
        )
    }

    fn render_tag_delete(&self, tag: &ConnectionTag, cx: &mut Context<Self>) -> Div {
        let used = self.tag_usage(tag.id);
        let id = tag.id;
        settings_row(
            format!("Delete “{}”?", tag.name),
            (used > 0).then(|| format!("{} will lose this tag.", connection_count(used)).into()),
            div()
                .flex()
                .gap(px(8.))
                .child(
                    button("cancel-delete-tag", "Cancel", ButtonKind::Quiet)
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.edit_tag(None, cx))),
                )
                .child(
                    button("confirm-delete-tag", "Delete", ButtonKind::Danger)
                        .debug_selector(|| "confirm-delete-tag".into())
                        .cursor_pointer()
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.delete_connection_tag(id, cx)),
                        ),
                ),
        )
    }

    /// The inline form used both for a new tag and for editing one in place.
    fn render_tag_form(&self, cx: &mut Context<Self>) -> Div {
        let editing = self.tag_editor.editing.is_some();
        let name_focus = self.tag_editor.name_editor.read(cx).focus_handle();
        let color_focus = self.tag_editor.color_editor.read(cx).focus_handle();
        let current = u32::from_str_radix(
            self.tag_editor
                .color
                .read(cx)
                .trim()
                .trim_start_matches('#'),
            16,
        )
        .ok();
        div()
            .px(px(16.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .bg(theme().panel_raised)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w(px(140.)).child(editor::input(
                        self.tag_editor.name_editor.clone(),
                        name_focus,
                        false,
                    )))
                    .child(
                        div()
                            .w(px(104.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme().text_muted)
                                    .child("#"),
                            )
                            .child(div().flex_1().child(editor::input(
                                self.tag_editor.color_editor.clone(),
                                color_focus,
                                false,
                            ))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .children(TAG_PALETTE.into_iter().map(|color| {
                        let chosen = current == Some(color);
                        div()
                            .id(SharedString::from(format!("tag-colour-{color:06x}")))
                            .pressable()
                            .size(px(22.))
                            .flex_none()
                            .rounded_full()
                            .bg(gpui::rgb(color))
                            .when(chosen, |view| view.border_2().border_color(theme().text))
                            .when(!chosen, |view| {
                                view.border_1().border_color(theme().hairline)
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tag_editor.color.update(cx, |value, cx| {
                                    *value = format!("{color:06X}");
                                    cx.notify();
                                });
                                cx.notify();
                            }))
                    }))
                    .child(div().flex_1())
                    .child(
                        button("cancel-tag-edit", "Cancel", ButtonKind::Quiet)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.edit_tag(None, cx))),
                    )
                    .child(
                        button(
                            "save-tag",
                            if editing { "Save" } else { "Add tag" },
                            ButtonKind::Primary,
                        )
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.save_connection_tag(cx))),
                    ),
            )
    }

    fn render_settings_updates(&mut self, cx: &mut Context<Self>) -> Div {
        use crate::updater::UpdateState;
        let (status, failed) = match &self.update_state {
            UpdateState::Idle => (None, false),
            UpdateState::Checking => (Some("Checking for updates…".to_string()), false),
            UpdateState::Current => (Some("Up to date".into()), false),
            UpdateState::Available(update) => (
                Some(format!("Version {} is available", update.version)),
                false,
            ),
            UpdateState::Installing(progress) => (Some(progress.label()), false),
            UpdateState::Installed(_) => (
                Some("Restart to finish updating. Open sessions will close.".into()),
                false,
            ),
            UpdateState::Failed(error) => (Some(error.clone()), true),
        };
        let notes = match &self.update_state {
            UpdateState::Available(update) => update.notes.clone(),
            _ => None,
        };
        let (action_label, action_kind, busy) = match &self.update_state {
            UpdateState::Idle | UpdateState::Current => {
                ("Check for updates".to_string(), ButtonKind::Quiet, false)
            }
            UpdateState::Checking => ("Checking…".into(), ButtonKind::Quiet, true),
            UpdateState::Available(update) => (
                format!("Install {}", update.version),
                ButtonKind::Primary,
                false,
            ),
            UpdateState::Installing(_) => ("Updating…".into(), ButtonKind::Quiet, true),
            UpdateState::Installed(_) => ("Restart DBX".into(), ButtonKind::Primary, false),
            UpdateState::Failed(_) => ("Try again".into(), ButtonKind::Quiet, false),
        };
        let version = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(concat!("DBX ", env!("CARGO_PKG_VERSION")))
            .when_some(status, |view, status| {
                view.child(
                    div()
                        .id("settings-update-status")
                        .text_size(px(11.))
                        .text_color(if failed {
                            theme().danger
                        } else {
                            theme().text_muted
                        })
                        .truncate()
                        .when(failed, |view| view.tooltip(tip(status.clone())))
                        .child(status),
                )
            });
        let mut rows = vec![
            settings_row(
                version,
                None,
                button("settings-update-action", action_label, action_kind)
                    .disabled(busy)
                    .when(!busy, |button| {
                        button
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.activate_update(cx)))
                    }),
            )
            .into_any_element(),
        ];
        if let Some(notes) = notes {
            rows.push(
                div()
                    .id("settings-release-notes")
                    .max_h(px(360.))
                    .px(px(16.))
                    .py(px(12.))
                    .overflow_y_scroll()
                    .text_size(px(12.))
                    .child(
                        gpui_component::text::TextView::markdown(
                            "settings-release-notes-text",
                            notes,
                        )
                        .selectable(true),
                    )
                    .into_any_element(),
            );
        }
        div().child(settings_group(rows))
    }
}

fn connection_count(count: usize) -> String {
    match count {
        0 => "No connections".into(),
        1 => "1 connection".into(),
        count => format!("{count} connections"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn connection_actions_are_available_on_their_own_screens(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.saved_connections.clear();
            app.begin_new_connection(cx);
            app
        });
        for width in [1200., 720., 480.] {
            cx.simulate_resize(gpui::size(px(width), px(800.)));
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    app.close_settings(cx);
                    app.begin_new_connection(cx);
                })
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let demo = cx
                .debug_bounds("try-demo")
                .expect("demo belongs on New connection");
            assert!(demo.left() >= px(0.) && demo.right() <= px(width));
            for selector in [
                "profile-portability-0",
                "profile-portability-1",
                "profile-portability-2",
            ] {
                assert!(cx.debug_bounds(selector).is_none());
            }
            cx.update(|_, cx| app.update(cx, |app, cx| app.select_kind(DatabaseKind::SQLite, cx)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(
                cx.debug_bounds("try-demo").is_none(),
                "demo is only offered when choosing a new connection"
            );

            cx.update(|_, cx| app.update(cx, |app, cx| app.open_settings(cx)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let connections = cx.debug_bounds("settings-section-Connections").unwrap();
            assert!(connections.left() >= px(0.) && connections.right() <= px(width));
            cx.simulate_click(connections.center(), Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("try-demo").is_none());
            for selector in [
                "profile-portability-0",
                "profile-portability-1",
                "profile-portability-2",
            ] {
                let action = cx.debug_bounds(selector).unwrap();
                assert!(action.left() >= px(0.) && action.right() <= px(width));
                cx.simulate_click(action.center(), Default::default());
                assert!(app.read_with(cx, |app, _| app.profile_transfer_dialog.is_some()));
                cx.update(|_, cx| {
                    app.update(cx, |app, cx| {
                        app.profile_transfer_dialog = None;
                        cx.notify();
                    })
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
        }
    }
}
