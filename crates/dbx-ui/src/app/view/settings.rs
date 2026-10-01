use super::super::*;

const TAG_PALETTE: [u32; 8] = [
    0xef6b73, 0xe5b567, 0x8fcf9c, 0x82aaff, 0xb48ead, 0x56b6c2, 0xe89bb5, 0xaab2bf,
];

impl DbxApp {
    /// Settings take over the content area rather than floating above it, so
    /// longer sections such as tag management have room to breathe.
    pub(super) fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let padding = if self.compact_layout {
            px(14.)
        } else {
            px(24.)
        };
        let appearance = self.render_settings_appearance(cx);
        let tags = self.render_settings_tags(cx);
        let updates = self.render_settings_updates(cx);
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
                    .flex_col()
                    .overflow_hidden()
                    .rounded(px(RADIUS_PANEL))
                    .border_1()
                    .border_color(theme().hairline)
                    .bg(theme().canvas)
                    .shadow(glass_shadow(10.))
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
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .max_w(px(640.))
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .pb(px(8.))
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                div()
                                                    .text_size(px(18.))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("Settings"),
                                            )
                                            .child(
                                                Button::new("close-settings")
                                                    .with_size(Size::XSmall)
                                                    .compact()
                                                    .ghost()
                                                    .tooltip("Close")
                                                    .child(icon(Icon::Close, theme().text_muted))
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.close_settings(cx)
                                                    })),
                                            ),
                                    )
                                    .child(settings_section("Appearance", appearance))
                                    .child(settings_section("Connection tags", tags))
                                    .child(settings_section("Updates", updates)),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_settings_appearance(&mut self, cx: &mut Context<Self>) -> Div {
        let current = self.appearance;
        let appearance_choices = Appearance::ALL.into_iter().map(|option| {
            segment(option.label(), option == current)
                .id(SharedString::from(format!(
                    "settings-appearance-{}",
                    option.label()
                )))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_appearance_preference(option, window, cx)
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .child(segmented_track().flex().children(appearance_choices)),
            )
            .when(!cfg!(target_os = "macos"), |view| {
                view.child(
                    gpui_component::switch::Switch::new("settings-reduce-transparency")
                        .label("Reduce transparency")
                        .checked(self.reduce_transparency)
                        .with_size(Size::Small)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_reduce_transparency(window, cx)
                        })),
                )
            })
    }

    fn render_settings_tags(&mut self, cx: &mut Context<Self>) -> Div {
        let editing = self.tag_editor.editing;
        let name_focus = self.tag_editor.name_editor.read(cx).focus_handle();
        let color_focus = self.tag_editor.color_editor.read(cx).focus_handle();
        let field = |label: &'static str| {
            div()
                .text_size(px(11.))
                .text_color(theme().text_muted)
                .child(label)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(self.connection_tags.iter().map(|tag| {
                        let selected = editing == Some(tag.id);
                        let edit = tag.clone();
                        div()
                            .id(SharedString::from(format!("settings-tag-{}", tag.id)))
                            .px(px(10.))
                            .h(px(26.))
                            .rounded_full()
                            .border_1()
                            .border_color(gpui::rgb(tag.color))
                            .when(selected, |view| view.bg(gpui::rgb(tag.color).alpha(0.15)))
                            .text_color(gpui::rgb(tag.color))
                            .text_size(px(11.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .tooltip(tip("Edit tag"))
                            .child(tag.name.clone())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.tag_editor.editing == Some(edit.id) {
                                    this.edit_tag(None, cx);
                                } else {
                                    this.edit_tag(Some(edit.clone()), cx);
                                }
                            }))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_end()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.))
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(field(if editing.is_some() {
                                "Tag name"
                            } else {
                                "New tag name"
                            }))
                            .child(editor::input(
                                self.tag_editor.name_editor.clone(),
                                name_focus,
                                false,
                            )),
                    )
                    .child(
                        div()
                            .w(px(110.))
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(field("Colour #"))
                            .child(editor::input(
                                self.tag_editor.color_editor.clone(),
                                color_focus,
                                false,
                            )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .gap(px(8.))
                            .when(editing.is_some(), |view| {
                                view.child(
                                    button("cancel-tag-edit", "Cancel", ButtonKind::Quiet)
                                        .h(px(32.))
                                        .cursor_pointer()
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.edit_tag(None, cx)),
                                        ),
                                )
                            })
                            .child(
                                button(
                                    "save-tag",
                                    if editing.is_some() {
                                        "Update tag"
                                    } else {
                                        "Add tag"
                                    },
                                    ButtonKind::Quiet,
                                )
                                .h(px(32.))
                                .cursor_pointer()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.save_connection_tag(cx)),
                                ),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .children(TAG_PALETTE.into_iter().map(|color| {
                        div()
                            .id(SharedString::from(format!("tag-colour-{color:06x}")))
                            .size(px(20.))
                            .rounded_full()
                            .bg(gpui::rgb(color))
                            .border_1()
                            .border_color(theme().border)
                            .tooltip(tip(format!("#{color:06X}")))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.tag_editor.color.update(cx, |value, cx| {
                                    *value = format!("{color:06X}");
                                    cx.notify();
                                });
                                cx.notify();
                            }))
                    })),
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
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme().text)
                                    .child(concat!("DBX ", env!("CARGO_PKG_VERSION"))),
                            )
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
                            }),
                    )
                    .child(
                        button("settings-update-action", action_label, action_kind)
                            .flex_none()
                            .disabled(busy)
                            .when(!busy, |button| {
                                button.cursor_pointer().on_click(
                                    cx.listener(|this, _, _, cx| this.activate_update(cx)),
                                )
                            }),
                    ),
            )
            .when_some(notes, |view, notes| {
                view.child(
                    div()
                        .id("settings-release-notes")
                        .max_h(px(320.))
                        .px(px(12.))
                        .py(px(10.))
                        .rounded(px(RADIUS_CONTROL))
                        .border_1()
                        .border_color(theme().border)
                        .bg(theme().panel)
                        .overflow_y_scroll()
                        .text_size(px(12.))
                        .child(
                            gpui_component::text::TextView::markdown(
                                "settings-release-notes-text",
                                notes,
                            )
                            .selectable(true),
                        ),
                )
            })
    }
}

fn settings_section(title: &'static str, content: Div) -> Div {
    div()
        .py(px(16.))
        .border_t_1()
        .border_color(theme().hairline)
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(content)
}
