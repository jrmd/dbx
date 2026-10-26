use super::super::*;
use crate::theme::{SPACE_1, SPACE_2, SPACE_4};
use gpui_component::ActiveTheme as _;
use gpui_component::input::{InputEvent, InputState, NumberInput};

const TAG_PALETTE: [u32; 8] = [
    0xef6b73, 0xe5b567, 0x8fcf9c, 0x82aaff, 0xb48ead, 0x56b6c2, 0xe89bb5, 0xaab2bf,
];

const SETTINGS_SECTIONS: [(SettingsSection, &str, Icon); 7] = [
    (SettingsSection::Appearance, "Appearance", Icon::Appearance),
    (SettingsSection::Editor, "Editor", Icon::Query),
    (
        SettingsSection::Shortcuts,
        "Keyboard shortcuts",
        Icon::Query,
    ),
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
    pub(super) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_font_pickers(window, cx);
        let compact = self.compact_layout;
        let padding = if compact {
            crate::theme::ui_px(14.)
        } else {
            crate::theme::ui_px(28.)
        };
        let selected = self.settings_section;
        // GPUI clips overflow to a rectangle, so each filled region rounds its
        // own outer corners to sit inside the card's 1px border.
        let inner_radius = crate::theme::ui_px(crate::theme::sheet_radius() - 1.);
        let title = SETTINGS_SECTIONS
            .iter()
            .find(|(section, ..)| *section == selected)
            .map_or("Settings", |(_, label, _)| *label);
        let content = match selected {
            SettingsSection::Appearance => self
                .render_settings_appearance(window, cx)
                .into_any_element(),
            SettingsSection::Editor => self.render_settings_editor(cx).into_any_element(),
            SettingsSection::Shortcuts => self.render_settings_shortcuts(window, cx),
            SettingsSection::QueryAgent => self.render_agent_settings(cx).into_any_element(),
            SettingsSection::Connections => self.render_settings_connections(cx).into_any_element(),
            SettingsSection::Tags => self.render_settings_tags(cx).into_any_element(),
            SettingsSection::Updates => self.render_settings_updates(cx).into_any_element(),
        };
        let close = glass_icon_button("close-settings", Icon::Close, false)
            .aria_label("Close settings")
            .track_focus(&self.settings_focus)
            .debug_selector(|| "close-settings".into())
            .tooltip(tip("Back to workspace"))
            .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)));
        let header = div()
            .bg(crate::theme::navigation_surface())
            .rounded_tr(inner_radius)
            .when(compact, |view| view.rounded_tl(inner_radius))
            .flex_none()
            .h(crate::theme::control_height(52.))
            .px(padding)
            .border_b_1()
            .border_color(theme().hairline)
            .flex()
            .items_center()
            .gap(crate::theme::ui_px(12.))
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
                                        .pressable(label)
                                        .role(gpui::Role::Tab)
                                        .aria_selected(section == selected)
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
                            .text_size(crate::theme::text_px(16.))
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
            .pr(crate::theme::ui_px(GLASS_INSET))
            .pb(crate::theme::ui_px(GLASS_INSET))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .overflow_hidden()
                    .rounded(crate::theme::ui_px(crate::theme::sheet_radius()))
                    .border_1()
                    .border_color(theme().hairline)
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
                                    .bg(theme().canvas)
                                    .rounded_br(inner_radius)
                                    .when(compact, |view| view.rounded_bl(inner_radius))
                                    .track_scroll(&self.settings_scroll)
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
                                            .max_w(crate::theme::ui_px(640.))
                                            .child(content),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_settings_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.settings_section;
        div()
            .id("settings-sections")
            .bg(crate::theme::navigation_surface())
            .rounded_l(crate::theme::ui_px(crate::theme::sheet_radius() - 1.))
            .role(gpui::Role::TabList)
            .aria_label("Settings sections")
            .w(crate::theme::ui_px(208.))
            .flex_none()
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(2.))
            .p(crate::theme::ui_px(10.))
            .border_r_1()
            .border_color(theme().hairline)
            .child(
                div()
                    .h(crate::theme::control_height(32.))
                    .px(crate::theme::ui_px(8.))
                    .mb(crate::theme::ui_px(8.))
                    .flex()
                    .items_center()
                    .text_size(crate::theme::text_px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme().text_muted)
                    .child("Settings"),
            )
            .children(SETTINGS_SECTIONS.into_iter().map(|(section, label, kind)| {
                let active = section == selected;
                div()
                    .id(SharedString::from(format!("settings-section-{label}")))
                    .pressable(label)
                    .role(gpui::Role::Tab)
                    .aria_selected(section == selected)
                    .debug_selector(move || format!("settings-section-{label}"))
                    .h(crate::theme::control_height(32.))
                    .px(crate::theme::ui_px(10.))
                    .rounded(crate::theme::ui_px(RADIUS_CONTROL))
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(10.))
                    .text_size(crate::theme::text_px(13.))
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
    pub(in crate::app) fn open_tag_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings_section(SettingsSection::Tags, window, cx);
    }

    pub(in crate::app) fn scrolling_settings_group(
        &self,
        id: &'static str,
        rows: impl IntoIterator<Item = AnyElement>,
    ) -> Div {
        settings_group(rows.into_iter().enumerate().map(|(index, row)| {
            crate::accessibility::focus_reveal((id, index), &self.settings_scroll, row)
                .into_any_element()
        }))
    }

    fn ensure_font_pickers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.font_pickers.is_some() {
            return;
        }
        // Paint the controls immediately. Enumerating installed fonts can be slow
        // on real desktops and must not block the first Appearance interaction.
        let pickers = [0, 1, 2].map(|index| {
            let current = match index {
                0 => self
                    .font_family
                    .clone()
                    .unwrap_or_else(|| cx.theme().font_family.to_string()),
                1 => self
                    .mono_font_family
                    .clone()
                    .unwrap_or_else(|| cx.theme().mono_font_family.to_string()),
                _ => self
                    .row_font_family
                    .clone()
                    .unwrap_or_else(|| crate::theme::row_font_family().to_string()),
            };
            let options = vec![current.clone()];
            let selected_index = options.iter().position(|name| name == &current);
            let picker = cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(
                        options
                            .into_iter()
                            .map(SharedString::from)
                            .collect::<Vec<_>>(),
                    ),
                    selected_index.map(gpui_component::IndexPath::new),
                    window,
                    cx,
                )
                .searchable(true)
            });
            cx.subscribe(
                &picker,
                move |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        match index {
                            0 => this.font_family = Some(value.to_string()),
                            1 => this.mono_font_family = Some(value.to_string()),
                            _ => this.row_font_family = Some(value.to_string()),
                        }
                        this.persist_settings(cx);
                        cx.refresh_windows();
                        cx.notify();
                    }
                },
            )
            .detach();
            picker
        });
        self.font_pickers = Some(pickers.clone());
        self.font_names_loading = true;
        let text_system = cx.text_system().clone();
        let font_names = cx.background_executor().spawn(async move {
            let mut names = text_system.all_font_names();
            names.sort_by_key(|name| name.to_lowercase());
            names.dedup();
            names
        });
        cx.spawn_in(window, async move |this, cx| {
            let names = font_names.await;
            this.update_in(cx, |this, window, cx| {
                this.font_names_loading = false;
                for picker in pickers {
                    picker.update(cx, |picker, cx| {
                        let current = picker.selected_value().cloned();
                        let mut options = names
                            .iter()
                            .cloned()
                            .map(SharedString::from)
                            .collect::<Vec<_>>();
                        if let Some(current) = &current
                            && !options.contains(current)
                        {
                            options.insert(0, current.clone());
                        }
                        picker.set_items(SearchableVec::new(options), window, cx);
                        if let Some(current) = current {
                            picker.set_selected_value(&current, window, cx);
                        }
                    });
                }
                cx.notify();
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
        self.font_size_inputs = Some([0, 1, 2].map(|index| {
            let size = self.font_size_preference(index);
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(size.to_string())
                    .min(10.)
                    .max(20.)
                    .validate(|value, _| {
                        value.is_empty()
                            || (value.len() <= 3 && value.bytes().all(|c| c.is_ascii_digit()))
                    })
            });
            cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| {
                    let current = this.font_size_preference(index);
                    let value = input.read(cx).value().parse::<u16>().ok();
                    match event {
                        InputEvent::Change => {
                            if let Some(size @ 10..=20) = value {
                                this.set_font_size_preference(size, index, cx);
                            }
                        }
                        InputEvent::Blur | InputEvent::PressEnter { .. } => {
                            let size = value.unwrap_or(current).clamp(10, 20);
                            this.set_font_size_preference(size, index, cx);
                            if input.read(cx).value() != size.to_string() {
                                input.update(cx, |input, cx| {
                                    input.set_value(size.to_string(), window, cx)
                                });
                            }
                        }
                        _ => {}
                    }
                },
            )
            .detach();
            input
        }));
    }

    fn font_size_preference(&self, index: usize) -> u16 {
        match index {
            0 => self.font_size,
            1 => self.mono_font_size,
            _ => self.row_font_size,
        }
    }

    fn set_font_size_preference(&mut self, size: u16, index: usize, cx: &mut Context<Self>) {
        let old_grid_size = self.font_size.max(self.row_font_size);
        let preference = match index {
            0 => &mut self.font_size,
            1 => &mut self.mono_font_size,
            _ => &mut self.row_font_size,
        };
        if *preference == size {
            return;
        }
        *preference = size;
        let ratio = self.font_size.max(self.row_font_size) as f32 / old_grid_size as f32;
        if ratio != 1. {
            for session in &mut self.sessions {
                for tab in &mut session.secondary_tabs {
                    let grid = match &tab.kind {
                        SecondaryTabKind::Data(data) => &data.data_grid,
                        SecondaryTabKind::Query(query) => &query.result_grid,
                        _ => continue,
                    };
                    grid.update(cx, |table, cx| {
                        table.delegate_mut().grow_text_widths(ratio);
                        table.refresh(cx);
                        result_table::emit_widths(table, cx);
                    });
                }
            }
        }
        self.persist_settings(cx);
        cx.refresh_windows();
        cx.notify();
    }

    fn text_scale_choices(&self, editor: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = if editor {
            self.editor_scale
        } else {
            self.ui_scale
        };
        div()
            .id(if editor {
                "editor-text-sizes"
            } else {
                "interface-text-sizes"
            })
            .flex()
            .flex_wrap()
            .gap(crate::theme::ui_px(4.))
            .role(gpui::Role::RadioGroup)
            .aria_label(if editor {
                "SQL editor text size"
            } else {
                "Interface size"
            })
            .children([100, 125, 150, 175, 200].into_iter().map(|scale| {
                let label = format!("{scale}%");
                segment(label.clone(), scale == selected)
                    .id(("text-scale", scale as usize))
                    .pressable(format!(
                        "{} size {label}",
                        if editor {
                            "SQL editor text"
                        } else {
                            "Interface"
                        }
                    ))
                    .debug_selector(move || {
                        format!(
                            "{}-size-{scale}",
                            if editor { "editor" } else { "interface" }
                        )
                    })
                    .role(gpui::Role::RadioButton)
                    .aria_toggled(if scale == selected {
                        gpui::accesskit::Toggled::True
                    } else {
                        gpui::accesskit::Toggled::False
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_text_scale(scale, editor, window, cx)
                    }))
            }))
    }

    fn render_settings_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let current = self.appearance;
        let appearance_choices = Appearance::ALL.into_iter().map(|option| {
            segment(option.label(), option == current)
                .id(SharedString::from(format!(
                    "settings-appearance-{}",
                    option.label()
                )))
                .pressable(option.label())
                .debug_selector(move || format!("settings-appearance-{}", option.label()))
                .role(gpui::Role::RadioButton)
                .aria_toggled(if option == current {
                    gpui::accesskit::Toggled::True
                } else {
                    gpui::accesskit::Toggled::False
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_appearance_preference(option, window, cx)
                }))
        });
        let mut theme_rows = vec![
            settings_row(
                "Appearance",
                None,
                segmented_track()
                    .id("appearance-choices")
                    .role(gpui::Role::RadioGroup)
                    .aria_label("Appearance")
                    .children(appearance_choices),
            )
            .into_any_element(),
        ];
        theme_rows.extend(self.render_theme_rows(window, cx));
        let mut rows = Vec::new();
        rows.push(settings_row("Interface size", Some("Grow text and controls together. Command or Control with +, −, or 0 adjusts or resets the size.".into()), self.text_scale_choices(false, cx)).into_any_element());
        let mut behaviour = Vec::new();
        behaviour.push(settings_row("Motion", Some("Follow your system preference or choose static controls. System uses reduced motion when the desktop cannot report a preference.".into()),
            div().id("motion-choices").role(gpui::Role::RadioGroup).aria_label("Motion")
                .flex().flex_wrap().gap(crate::theme::ui_px(4.))
                .children(crate::motion::MotionPreference::ALL.into_iter().map(|option| {
                    segment(option.label(), option == crate::motion::preference(cx))
                        .id(SharedString::from(format!("motion-{}", option.label())))
                        .pressable(format!("Motion {}", option.label()))
                        .role(gpui::Role::RadioButton)
                        .aria_toggled(if option == crate::motion::preference(cx) { gpui::accesskit::Toggled::True } else { gpui::accesskit::Toggled::False })
                        .debug_selector(move || format!("motion-{}", option.label()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            crate::motion::set_preference(option, cx);
                            this.persist_settings(cx);
                            cx.notify();
                        }))
                }))).into_any_element());
        #[cfg(target_os = "macos")]
        {
            let supported = crate::native_glass::supported();
            let detail = if !supported {
                "Requires macOS 26 or later and a build with native glass support. Solid surfaces are in use."
            } else if self.native_glass.system_reduces_transparency() {
                "Paused by macOS Reduce Transparency or Increase Contrast. Your choice is saved."
            } else {
                "Native glass for navigation, with solid tables, editors and forms for readability."
            };
            behaviour.push(
                settings_row(
                    "Liquid Glass",
                    Some(detail.into()),
                    settings_switch(
                        "settings-liquid-glass",
                        "Liquid Glass",
                        !self.reduce_transparency,
                    )
                    .debug_selector(|| "settings-liquid-glass".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_reduce_transparency(window, cx)
                    })),
                )
                .into_any_element(),
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            behaviour.push(
                settings_row(
                    "Reduce transparency",
                    None,
                    settings_switch(
                        "settings-reduce-transparency",
                        "Reduce transparency",
                        self.reduce_transparency,
                    )
                    .debug_selector(|| "settings-reduce-transparency".into())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_reduce_transparency(window, cx)
                    })),
                )
                .into_any_element(),
            );
        }
        if let Some(pickers) = &self.font_pickers {
            for (index, label) in ["Interface font", "Monospace font", "Row font"]
                .into_iter()
                .enumerate()
            {
                rows.push(
                    settings_row(
                        label,
                        if self.font_names_loading {
                            Some("Loading installed fonts…".into())
                        } else if index == 2 {
                            Some("Values in tables and row details. Monospace by default.".into())
                        } else {
                            None
                        },
                        div().w(crate::theme::ui_px(240.)).child(
                            Select::new(&pickers[index])
                                .disabled(self.font_names_loading)
                                .h(crate::theme::control_height(32.))
                                .w_full()
                                .menu_max_h(crate::theme::ui_px(280.))
                                .search_placeholder("Search fonts…")
                                .accessible_select(label, &pickers[index]),
                        ),
                    )
                    .into_any_element(),
                );
                if let Some(input) = self
                    .font_size_inputs
                    .as_ref()
                    .and_then(|inputs| inputs.get(index))
                {
                    rows.push(
                        settings_row(
                            match index {
                                0 => "Interface font size",
                                1 => "Monospace font size",
                                _ => "Row font size",
                            },
                            Some("10–20 px, independent of interface zoom.".into()),
                            div()
                                .id(match index {
                                    0 => "interface-font-size",
                                    1 => "mono-font-size",
                                    _ => "row-font-size",
                                })
                                .debug_selector(move || match index {
                                    0 => "interface-font-size".into(),
                                    1 => "mono-font-size".into(),
                                    _ => "row-font-size".into(),
                                })
                                .w(crate::theme::ui_px(144.))
                                .child(
                                    NumberInput::new(input)
                                        .h(crate::theme::control_height(32.))
                                        .suffix("px")
                                        .w_full(),
                                ),
                        )
                        .into_any_element(),
                    );
                }
            }
        }
        behaviour.push(
            settings_row(
                "Drag to select rows",
                Some("Click and drag across rows in the data view to select a range.".into()),
                settings_switch(
                    "drag-row-selection",
                    "Drag to select rows",
                    self.drag_row_selection,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.drag_row_selection = !this.drag_row_selection;
                    this.row_drag = None;
                    this.persist_settings(cx);
                    cx.notify();
                })),
            )
            .into_any_element(),
        );
        div()
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(SPACE_4 + SPACE_2))
            .child(settings_section(
                "Theme",
                self.scrolling_settings_group("settings-theme-row", theme_rows),
            ))
            .child(settings_section(
                "Behaviour",
                self.scrolling_settings_group("settings-behaviour-row", behaviour),
            ))
            .child(settings_section(
                "Text",
                self.scrolling_settings_group("settings-main-row", rows),
            ))
    }

    fn render_settings_connections(&mut self, cx: &mut Context<Self>) -> Div {
        let unavailable = self.vault_state != Some(VaultState::Unlocked) || self.vault_busy;
        let automatic_unlock = div()
            .flex()
            .flex_col()
            .px(crate::theme::ui_px(SPACE_4))
            .py(crate::theme::ui_px(12.))
            .gap(crate::theme::ui_px(SPACE_2))
            .child(
                gpui_component::checkbox::Checkbox::new("settings-remember-device")
                    .label("Unlock automatically on this device")
                    .with_size(Size::Small)
                    .checked(self.remember_device)
                    .disabled(unavailable || self.touch_id_enabled)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_remember_device(cx))),
            )
            .child(
                div()
                    .text_size(crate::theme::text_px(11.))
                    .text_color(theme().text_muted)
                    .child(if self.touch_id_enabled {
                        "Automatic unlock is unavailable while Touch ID is enabled."
                    } else {
                        "Save the vault key in your system keychain to skip the passphrase on launch."
                    }),
            );
        div()
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(SPACE_4))
            .child(settings_section(
                "Vault",
                self.scrolling_settings_group(
                    "settings-vault-row",
                    [automatic_unlock.into_any_element()],
                ),
            ))
            .child(
                self.scrolling_settings_group(
                    "settings-connections-row",
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
                            .accessible_name(match mode {
                                0 => "Export password-free connection profiles",
                                1 => "Export encrypted connection bundle",
                                _ => "Import connection profiles",
                            })
                            .debug_selector(move || format!("profile-portability-{mode}"))
                            .disabled(unavailable)
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.open_profile_transfer(mode, window, cx)
                                },
                            )),
                        )
                        .into_any_element()
                    }),
                ),
            )
    }

    fn render_settings_editor(&mut self, cx: &mut Context<Self>) -> Div {
        div().child(self.scrolling_settings_group("settings-editor-row", [
            settings_row("SQL editor text size", Some("Additional enlargement of query text.".into()), self.text_scale_choices(true, cx)).into_any_element(),
            settings_row(
                "Vim mode",
                Some("Modal editing in SQL query tabs. Starts in Normal mode; press i to type and Esc to return. F6 moves focus out of the editor.".into()),
                div().id("vim-setting-control").debug_selector(|| "settings-vim-mode".into()).child(settings_switch("settings-vim-mode", "SQL Vim mode", self.vim_mode)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_vim_mode(cx)))),
            ).into_any_element(),
        ])).child(div().mt(crate::theme::ui_px(16.)).text_size(crate::theme::text_px(12.)).text_color(theme().text_muted)
            .child("Vim keys: h/j/k/l, w/b/e, W/B/E, 0/^/$, gg/G, counts, i/a/I/A, o/O, v/V, d/c/y with motions, dd/cc/yy, x/X, D/C, r, p/P, u and Ctrl+R. Query and clipboard shortcuts remain available."))
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
                .pressable("New tag")
                .debug_selector(|| "new-tag".into())
                .h(crate::theme::control_height(48.))
                .px(crate::theme::ui_px(16.))
                .flex()
                .items_center()
                .gap(crate::theme::ui_px(10.))
                .text_size(crate::theme::text_px(13.))
                .text_color(theme().accent_text)
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
        div().child(self.scrolling_settings_group("settings-main-row", rows))
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
                .gap(crate::theme::ui_px(10.))
                .child(
                    div()
                        .size(crate::theme::ui_px(10.))
                        .flex_none()
                        .rounded_full()
                        .bg(gpui::rgb(tag.color)),
                )
                .child(tag.name.clone()),
            Some(connection_count(used).into()),
            div()
                .flex()
                .gap(crate::theme::ui_px(2.))
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
                .gap(crate::theme::ui_px(8.))
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
            .px(crate::theme::ui_px(16.))
            .py(crate::theme::ui_px(12.))
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(12.))
            .bg(theme().panel_raised)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(crate::theme::ui_px(140.))
                            .child(editor::input(
                                self.tag_editor.name_editor.clone(),
                                name_focus,
                                false,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .w(crate::theme::ui_px(104.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(crate::theme::ui_px(4.))
                            .child(
                                div()
                                    .text_size(crate::theme::text_px(12.))
                                    .text_color(theme().text_muted)
                                    .child("#"),
                            )
                            .child(div().flex_1().child(editor::input(
                                self.tag_editor.color_editor.clone(),
                                color_focus,
                                false,
                                cx,
                            ))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(crate::theme::ui_px(8.))
                    .children(TAG_PALETTE.into_iter().map(|color| {
                        let chosen = current == Some(color);
                        div()
                            .id(SharedString::from(format!("tag-colour-{color:06x}")))
                            .pressable(format!("Tag colour #{color:06x}"))
                            .role(gpui::Role::RadioButton)
                            .aria_toggled(if chosen {
                                gpui::accesskit::Toggled::True
                            } else {
                                gpui::accesskit::Toggled::False
                            })
                            .size(crate::theme::ui_px(28.))
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
            .gap(crate::theme::ui_px(2.))
            .child(concat!("DBX ", env!("CARGO_PKG_VERSION")))
            .when_some(status, |view, status| {
                view.child(
                    div()
                        .id("settings-update-status")
                        .text_size(crate::theme::text_px(11.))
                        .text_color(if failed {
                            theme().danger_text
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
                    .max_h(crate::theme::ui_px(360.))
                    .px(crate::theme::ui_px(16.))
                    .py(crate::theme::ui_px(12.))
                    .overflow_y_scroll()
                    .text_size(crate::theme::text_px(12.))
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
        div().child(self.scrolling_settings_group("settings-main-row", rows))
    }
}

fn connection_count(count: usize) -> String {
    match count {
        0 => "No connections".into(),
        1 => "1 connection".into(),
        count => format!("{count} connections"),
    }
}

/// A titled group of settings rows; the label sits outside the inset group,
/// as in the platform's own preference panes.
fn settings_section(title: &'static str, group: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(crate::theme::ui_px(SPACE_2))
        .child(
            div()
                .px(crate::theme::ui_px(SPACE_1))
                .text_size(crate::theme::text_px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme().text_muted)
                .child(title),
        )
        .child(group)
}
