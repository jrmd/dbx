//! Theme controls and local file import/export. Parsing lives in theme::catalog.
use super::*;
use crate::theme::catalog::{self, Preset, TOKENS};
use crate::theme::{SPACE_2, SPACE_3, SPACE_4};
use gpui_component::color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState};
use gpui_component::input::{Input, InputEvent, InputState};

pub(super) struct ThemeEditor {
    picker: Entity<SelectState<SearchableVec<SharedString>>>,
    color: Entity<InputState>,
    visual: Entity<ColorPickerState>,
    token: usize,
    error: Option<String>,
    expanded: bool,
}

impl DbxApp {
    fn ensure_theme_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.theme_editor.is_some() {
            return;
        }
        let picker = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(
                    TOKENS
                        .iter()
                        .map(|t| SharedString::from(format!("{} · {}", t.group, t.label)))
                        .collect::<Vec<_>>(),
                ),
                Some(gpui_component::IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let color = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("#rrggbb or #rrggbbaa")
                .default_value(catalog::color_hex((TOKENS[0].get)(&theme())))
        });
        let visual =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value((TOKENS[0].get)(&theme())));
        cx.subscribe_in(
            &visual,
            window,
            |this, _, event: &ColorPickerEvent, window, cx| {
                if let ColorPickerEvent::Change(Some(color)) = event
                    && let Some(editor) = &mut this.theme_editor
                {
                    let value = catalog::color_hex((*color).into());
                    editor
                        .color
                        .update(cx, |input, cx| input.set_value(value, window, cx));
                    editor.error = None;
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &picker,
            window,
            |this, _, event: &SelectEvent<SearchableVec<SharedString>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event
                    && let Some(index) = TOKENS
                        .iter()
                        .position(|t| format!("{} · {}", t.group, t.label) == value.as_ref())
                    && let Some(editor) = &mut this.theme_editor
                {
                    editor.token = index;
                    this.refresh_theme_color_input(window, cx);
                    cx.notify();
                }
            },
        )
        .detach();
        cx.subscribe_in(&color, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.apply_theme_color(window, cx);
            }
        })
        .detach();
        self.theme_editor = Some(ThemeEditor {
            picker,
            color,
            visual,
            token: 0,
            error: None,
            expanded: false,
        });
    }

    pub(super) fn refresh_theme_color_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor) = &mut self.theme_editor {
            editor.error = None;
            editor.visual.update(cx, |picker, cx| {
                picker.set_value((TOKENS[editor.token].get)(&theme()), window, cx)
            });
            let value = catalog::color_hex((TOKENS[editor.token].get)(&theme()));
            editor
                .color
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    fn apply_theme_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        catalog::set_preferences(&self.theme_preferences);
        self.apply_material(window, cx);
        self.persist_settings(cx);
        cx.notify();
    }

    /// Activates custom theme `index`. Themes made for one appearance (VS
    /// Code imports) switch DBX to it, as VS Code does.
    fn select_custom_theme(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(theme) = self.theme_preferences.custom_themes.get(index) else {
            return;
        };
        if let Some(appearance) = theme.appearance() {
            self.appearance = appearance;
            set_appearance(appearance);
        }
        self.theme_preferences.active_custom = Some(index);
        self.apply_theme_preferences(window, cx);
    }

    fn choose_theme_preset(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_preferences.preset = preset;
        self.theme_preferences.active_custom = None;
        self.apply_theme_preferences(window, cx);
    }

    fn apply_theme_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.theme_editor else {
            return;
        };
        let result = self.theme_preferences.edit(
            TOKENS[editor.token].key,
            &editor.color.read(cx).value(),
            crate::theme::is_dark(),
        );
        match result {
            Ok(()) => self.apply_theme_preferences(window, cx),
            Err(error) => {
                editor.error = Some(error);
                cx.notify();
            }
        }
    }

    fn import_theme_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import a VS Code, Zed or DBX theme JSON".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) => return Ok(()),
                result => {
                    this.update_in(cx, |this, _, cx| {
                        this.show_toast(
                            ToastKind::Error,
                            format!("Could not open theme picker: {result:?}"),
                            cx,
                        )
                    })?;
                    return Ok(());
                }
            };
            let Some(path) = paths.into_iter().next() else {
                return Ok(());
            };
            let imported = cx
                .background_executor()
                .spawn(async move {
                    use std::io::Read;
                    let file = std::fs::File::open(&path)
                        .map_err(|e| format!("Could not read theme: {e}"))?;
                    let mut source = String::new();
                    file.take(catalog::MAX_THEME_BYTES + 1)
                        .read_to_string(&mut source)
                        .map_err(|e| format!("Could not read theme: {e}"))?;
                    catalog::import_themes(
                        &source,
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Imported theme"),
                    )
                })
                .await;
            this.update_in(cx, |this, window, cx| match imported {
                Ok(imported) => {
                    let count = imported.len();
                    let name = imported[0].theme.name.clone();
                    this.theme_preferences.preset = Preset::Default;
                    let mut first = None;
                    for imported in imported {
                        let index = this.theme_preferences.add(imported.theme);
                        first.get_or_insert(index);
                    }
                    let index = first.expect("validated nonempty theme family");
                    this.select_custom_theme(index, window, cx);
                    let message = if count > 1 {
                        format!("Imported {count} themes · Applied {name}")
                    } else {
                        format!("Applied {name}")
                    };
                    this.show_toast(ToastKind::Success, message, cx);
                }
                Err(error) => this.show_toast(ToastKind::Error, error, cx),
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    fn export_theme_file(&mut self, cx: &mut Context<Self>) {
        let source = match catalog::export_theme(&self.theme_preferences) {
            Ok(source) => source,
            Err(error) => {
                self.show_toast(ToastKind::Error, error, cx);
                return;
            }
        };
        let receiver = cx.prompt_for_new_path(
            &dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")),
            Some("theme.dbx.json"),
        );
        cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return Ok(()),
                result => {
                    this.update(cx, |this, cx| {
                        this.show_toast(
                            ToastKind::Error,
                            format!("Could not open save dialog: {result:?}"),
                            cx,
                        )
                    })?;
                    return Ok(());
                }
            };
            let result = cx
                .background_executor()
                .spawn(async move { std::fs::write(path, source) })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.show_toast(ToastKind::Success, "Theme exported", cx),
                Err(error) => this.show_toast(
                    ToastKind::Error,
                    format!("Could not export theme: {error}"),
                    cx,
                ),
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach();
    }

    /// Rows for the Theme group on the Appearance page: palette choice,
    /// import/export, and the optional per-colour editor.
    pub(super) fn render_theme_rows(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        self.ensure_theme_editor(window, cx);
        let dark = crate::theme::is_dark();
        let current = theme();
        let custom_active = self.theme_preferences.active_custom;
        let presets: Vec<AnyElement> = Preset::ALL
            .into_iter()
            .map(|preset| {
                let active = custom_active.is_none() && self.theme_preferences.preset == preset;
                theme_tile(
                    format!("theme-preset-{}", preset.label()),
                    preset.label().into(),
                    catalog::preset_theme(preset, dark, true),
                    active,
                    None,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_theme_preset(preset, window, cx)
                }))
                .into_any_element()
            })
            .collect();
        let mut yours: Vec<AnyElement> = self
            .theme_preferences
            .custom_themes
            .iter()
            .enumerate()
            .map(|(index, custom)| {
                let name: SharedString = custom.name.chars().take(24).collect::<String>().into();
                let tile = format!("theme-custom-{index}");
                let button = SharedString::from(format!("remove-theme-{index}"));
                let remove = div()
                    .id(button.clone())
                    .group(button.clone())
                    .debug_selector(move || format!("remove-theme-{index}"))
                    .pressable(format!("Remove {name}"))
                    .tooltip(tip("Remove theme"))
                    .size(crate::theme::ui_px(18.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(current.panel_raised)
                    .border_1()
                    .border_color(current.border_strong)
                    .shadow(glass_shadow(3.))
                    .cursor_pointer()
                    // Revealed with its tile (or keyboard focus); red when targeted.
                    .opacity(0.)
                    .group_hover(tile.clone(), |style| style.opacity(1.))
                    .focus_visible(|style| style.opacity(1.).shadow(crate::theme::focus_ring()))
                    .hover(|style| style.bg(current.danger).border_color(current.danger))
                    .child(
                        icon(Icon::Close, current.text_muted)
                            .size(crate::theme::ui_px(9.))
                            .group_hover(button, |style| {
                                style.text_color(current.accent_foreground)
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.confirm_remove_custom_theme(index, window, cx)
                    }));
                theme_tile(
                    tile,
                    name,
                    // Preview a single-appearance theme in its own variant.
                    self.theme_preferences.resolve_custom(
                        index,
                        custom
                            .appearance()
                            .map_or(dark, |appearance| appearance == Appearance::Dark),
                        true,
                    ),
                    custom_active == Some(index),
                    Some(remove),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_custom_theme(index, window, cx)
                }))
                .into_any_element()
            })
            .collect();
        yours.push(
            import_tile()
                .on_click(cx.listener(|this, _, window, cx| this.import_theme_file(window, cx)))
                .into_any_element(),
        );
        let tile_row = |title: &'static str,
                        detail: Option<&'static str>,
                        group: &'static str,
                        tiles: Vec<AnyElement>| {
            div()
                .px(crate::theme::ui_px(SPACE_4))
                .py(crate::theme::ui_px(SPACE_3))
                .flex()
                .flex_col()
                .gap(crate::theme::ui_px(SPACE_3))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(crate::theme::ui_px(2.))
                        .child(
                            div()
                                .text_size(crate::theme::text_px(13.))
                                .text_color(current.text)
                                .child(title),
                        )
                        .when_some(detail, |view, detail| {
                            view.child(
                                div()
                                    .text_size(crate::theme::text_px(11.))
                                    .text_color(current.text_muted)
                                    .child(detail),
                            )
                        }),
                )
                .child(
                    div()
                        .id(group)
                        .role(gpui::Role::RadioGroup)
                        .aria_label(title)
                        .flex()
                        .flex_wrap()
                        .gap(crate::theme::ui_px(SPACE_2))
                        .children(tiles),
                )
                .into_any_element()
        };
        let editor = self.theme_editor.as_ref().unwrap();
        let expanded = editor.expanded;
        let mut rows = vec![
            tile_row("Colour theme", None, "theme-presets", presets),
            tile_row(
                "Your themes",
                (self.theme_preferences.custom_themes.is_empty())
                    .then_some("Import a VS Code, Zed or DBX theme, or edit a preset’s colours."),
                "theme-custom",
                yours,
            ),
            settings_row(
                "Colours",
                Some(
                    if custom_active.is_some() {
                        "Edit or export the selected theme."
                    } else {
                        "Editing a preset saves a copy under Your themes."
                    }
                    .into(),
                ),
                div()
                    .flex()
                    .flex_wrap()
                    .gap(crate::theme::ui_px(SPACE_2))
                    .child(
                        button("export-theme", "Export…", ButtonKind::Quiet)
                            .on_click(cx.listener(|this, _, _, cx| this.export_theme_file(cx))),
                    )
                    .child(
                        button(
                            "edit-theme-colours",
                            if expanded { "Done" } else { "Edit…" },
                            ButtonKind::Quiet,
                        )
                        .debug_selector(|| "edit-theme-colours".into())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(editor) = &mut this.theme_editor {
                                editor.expanded = !editor.expanded;
                            }
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element(),
        ];
        if expanded {
            rows.push(self.render_theme_editor(cx).into_any_element());
        }
        rows
    }

    fn render_theme_editor(&self, cx: &mut Context<Self>) -> Div {
        let editor = self.theme_editor.as_ref().unwrap();
        let current = theme();
        let mono = gpui_component::Theme::global(cx).mono_font_family.clone();
        let variant = if crate::theme::is_dark() {
            "dark"
        } else {
            "light"
        };
        div()
            .px(crate::theme::ui_px(SPACE_4))
            .py(crate::theme::ui_px(SPACE_3))
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(SPACE_3))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(crate::theme::ui_px(SPACE_2))
                    .child(
                        div().flex_1().min_w(crate::theme::ui_px(200.)).child(
                            Select::new(&editor.picker)
                                .w_full()
                                .search_placeholder("Find a colour…")
                                .menu_max_h(crate::theme::ui_px(260.))
                                .accessible_select("Theme colour", &editor.picker),
                        ),
                    )
                    .child(ColorPicker::new(&editor.visual).small())
                    .child(
                        div()
                            .id("theme-color-value")
                            .debug_selector(|| "theme-color-value".into())
                            .w(crate::theme::ui_px(120.))
                            .child(Input::new(&editor.color).aria_label("Colour value")),
                    )
                    .child(
                        button("apply-theme-color", "Apply", ButtonKind::Primary)
                            .debug_selector(|| "apply-theme-color".into())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_theme_color(window, cx)
                            })),
                    )
                    .child(
                        button("reset-theme-color", "Reset", ButtonKind::Quiet).on_click(
                            cx.listener(|this, _, window, cx| this.reset_theme_color(window, cx)),
                        ),
                    ),
            )
            .child(
                div()
                    .text_size(crate::theme::text_px(11.))
                    .text_color(current.text_muted)
                    .child(format!(
                        "Editing the {variant} variant. Switch appearance to edit the other."
                    )),
            )
            .when_some(editor.error.clone(), |view, error| {
                view.child(
                    div()
                        .id("theme-color-error")
                        .role(gpui::Role::Alert)
                        .text_size(crate::theme::text_px(12.))
                        .text_color(current.danger_text)
                        .child(error),
                )
            })
            .child(theme_specimen(&current, mono))
    }

    fn confirm_remove_custom_theme(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(custom) = self.theme_preferences.custom_themes.get(index) else {
            return;
        };
        let focus = cx.focus_handle();
        let return_focus = window.focused(cx);
        focus.focus(window, cx);
        self.confirmation_dialog = Some(ConfirmationDialog {
            title: format!("Remove “{}”?", custom.name),
            detail: "Its colours are deleted from this device. Export it first to keep a copy."
                .into(),
            confirm_label: "Remove theme",
            tone: ConfirmationTone::Danger,
            action: ConfirmationAction::RemoveCustomTheme { index },
            focus,
            return_focus,
            sql: None,
        });
        cx.notify();
    }

    pub(super) fn remove_custom_theme(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.theme_preferences.remove(index);
        self.apply_theme_preferences(window, cx);
        self.refresh_theme_color_input(window, cx);
    }

    fn reset_theme_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.theme_editor else {
            return;
        };
        let token = &TOKENS[editor.token];
        let dark = crate::theme::is_dark();
        let palette = catalog::preset_theme(self.theme_preferences.preset, dark, false);
        let _ = self.theme_preferences.edit(
            token.key,
            &catalog::color_hex((token.get)(&palette)),
            dark,
        );
        self.apply_theme_preferences(window, cx);
    }
}

/// A selectable thumbnail that sketches the palette as a tiny workspace:
/// rail, active row, muted text, and a line of syntax.
fn theme_tile(
    id: String,
    label: SharedString,
    palette: crate::theme::Theme,
    active: bool,
    remove: Option<gpui::Stateful<Div>>,
) -> gpui::Stateful<Div> {
    let current = theme();
    let bar = |width: f32, color| {
        div()
            .w(crate::theme::ui_px(width))
            .h(crate::theme::ui_px(4.))
            .rounded_full()
            .bg(color)
    };
    let selector = id.clone();
    div()
        .id(SharedString::from(id.clone()))
        .group(id)
        .debug_selector(move || selector.clone())
        .pressable(format!("{label} theme"))
        .role(gpui::Role::RadioButton)
        .aria_toggled(if active {
            gpui::accesskit::Toggled::True
        } else {
            gpui::accesskit::Toggled::False
        })
        .w(crate::theme::ui_px(104.))
        .flex()
        .flex_col()
        .gap(crate::theme::ui_px(6.))
        .cursor_pointer()
        .child(
            div()
                .relative()
                .p(crate::theme::ui_px(2.))
                .rounded(crate::theme::ui_px(10.))
                .border_2()
                .border_color(if active {
                    current.accent
                } else {
                    gpui::transparent_black().into()
                })
                .child(
                    div()
                        .h(crate::theme::ui_px(58.))
                        .rounded(crate::theme::ui_px(7.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(current.hairline)
                        .bg(palette.canvas)
                        .flex()
                        .child(
                            div()
                                .w(crate::theme::ui_px(14.))
                                .bg(palette.rail)
                                .border_r_1()
                                .border_color(palette.border)
                                .flex()
                                .justify_center()
                                .pt(crate::theme::ui_px(6.))
                                .child(
                                    div()
                                        .size(crate::theme::ui_px(5.))
                                        .rounded_full()
                                        .bg(palette.accent),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(crate::theme::ui_px(5.))
                                .p(crate::theme::ui_px(6.))
                                .child(bar(44., palette.text))
                                .child(
                                    div()
                                        .h(crate::theme::ui_px(10.))
                                        .rounded(crate::theme::ui_px(2.))
                                        .bg(palette.grid_active)
                                        .flex()
                                        .items_center()
                                        .px(crate::theme::ui_px(3.))
                                        .child(bar(36., palette.text_muted)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap(crate::theme::ui_px(3.))
                                        .child(bar(16., palette.sql_keyword))
                                        .child(bar(24., palette.sql_string))
                                        .child(bar(8., palette.sql_number)),
                                ),
                        ),
                )
                .when_some(remove, |view, remove| {
                    view.child(
                        remove
                            .absolute()
                            .top(crate::theme::ui_px(8.))
                            .right(crate::theme::ui_px(8.)),
                    )
                }),
        )
        .child(
            div()
                .px(crate::theme::ui_px(2.))
                .truncate()
                .text_size(crate::theme::text_px(12.))
                .when(active, |view| {
                    view.text_color(current.text)
                        .font_weight(FontWeight::MEDIUM)
                })
                .when(!active, |view| view.text_color(current.text_muted))
                .child(label),
        )
}

/// The trailing tile under Your themes: same footprint as a theme, so
/// importing reads as adding to the list.
fn import_tile() -> gpui::Stateful<Div> {
    let current = theme();
    div()
        .id("import-theme")
        .debug_selector(|| "import-theme".into())
        .pressable("Import theme")
        .w(crate::theme::ui_px(104.))
        .flex()
        .flex_col()
        .gap(crate::theme::ui_px(6.))
        .cursor_pointer()
        .child(
            div().p(crate::theme::ui_px(4.)).child(
                div()
                    .h(crate::theme::ui_px(58.))
                    .rounded(crate::theme::ui_px(7.))
                    .border_1()
                    .border_dashed()
                    .border_color(current.border_strong)
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|style| style.bg(current.glass_hover))
                    .child(icon(Icon::Add, current.text_muted)),
            ),
        )
        .child(
            div()
                .px(crate::theme::ui_px(2.))
                .text_size(crate::theme::text_px(12.))
                .text_color(current.text_muted)
                .child("Import…"),
        )
}

/// A small result grid and query line painted with the live palette, so
/// edits to grid and syntax colours are visible before leaving settings.
fn theme_specimen(current: &crate::theme::Theme, mono: SharedString) -> Div {
    let cell = |text: &'static str, width: f32| {
        div()
            .w(crate::theme::ui_px(width))
            .flex_none()
            .truncate()
            .child(text)
    };
    let row = |index: &'static str, email, plan, background| {
        div()
            .h(crate::theme::ui_px(24.))
            .px(crate::theme::ui_px(SPACE_2))
            .flex()
            .items_center()
            .bg(background)
            .border_t_1()
            .border_color(current.border)
            .text_color(current.text)
            .child(
                div()
                    .w(crate::theme::ui_px(28.))
                    .flex_none()
                    .text_color(current.text_muted)
                    .child(index),
            )
            .child(cell(email, 180.))
            .child(cell(plan, 60.))
    };
    let token = |text: &'static str, color| div().text_color(color).child(text);
    div()
        .rounded(crate::theme::ui_px(RADIUS_CONTROL))
        .overflow_hidden()
        .border_1()
        .border_color(current.border)
        .font_family(mono)
        .text_size(crate::theme::text_px(11.))
        .child(
            div()
                .px(crate::theme::ui_px(SPACE_2))
                .py(crate::theme::ui_px(6.))
                .bg(current.canvas)
                .flex()
                .flex_wrap()
                .gap_x(crate::theme::ui_px(5.))
                .child(token("SELECT", current.sql_keyword))
                .child(token("email,", current.sql_identifier))
                .child(token("plan", current.sql_identifier))
                .child(token("FROM", current.sql_keyword))
                .child(token("users", current.sql_identifier))
                .child(token("WHERE", current.sql_keyword))
                .child(token("plan =", current.text))
                .child(token("'pro'", current.sql_string))
                .child(token("LIMIT", current.sql_keyword))
                .child(token("50", current.sql_number))
                .child(token("-- preview", current.sql_comment)),
        )
        .child(
            div()
                .h(crate::theme::ui_px(24.))
                .px(crate::theme::ui_px(SPACE_2))
                .flex()
                .items_center()
                .bg(current.panel_raised)
                .border_t_1()
                .border_color(current.border)
                .text_color(current.text_muted)
                .child(div().w(crate::theme::ui_px(28.)).flex_none().child("#"))
                .child(cell("email", 180.))
                .child(cell("plan", 60.)),
        )
        .child(row("1", "ada@example.com", "pro", current.canvas))
        .child(row("2", "grace@example.com", "pro", current.grid_alternate))
        .child(row("3", "linus@example.com", "pro", current.grid_selected))
        .child(row("4", "margaret@example.com", "pro", current.grid_active))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn one_click_applies_dark_theme_and_reopening_settings_starts_at_appearance(
        cx: &mut gpui::TestAppContext,
    ) {
        let _appearance = crate::theme::appearance_test_lock();
        cx.update(gpui_component::init);
        set_appearance(Appearance::Light);
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.appearance = Appearance::Light;
            app.settings_store = Some(store.clone());
            app.apply_material(window, cx);
            app.open_settings(window, cx);
            app
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let dark = cx.debug_bounds("settings-appearance-Dark").unwrap();
        let light = cx.debug_bounds("settings-appearance-Light").unwrap();
        cx.simulate_click(dark.center(), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.debug_bounds("settings-appearance-Dark").unwrap(), dark);
        assert_eq!(cx.debug_bounds("settings-appearance-Light").unwrap(), light);
        app.read_with(cx, |app, cx| {
            assert_eq!(app.appearance, Appearance::Dark);
            assert!(crate::theme::is_dark());
            assert!(gpui_component::Theme::global(cx).is_dark());
            assert_eq!(store.load().unwrap().appearance, Appearance::Dark);
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.settings_section = SettingsSection::Editor;
                app.close_settings(window, cx);
                app.open_settings(window, cx);
                assert!(app.settings_section == SettingsSection::Appearance);
                app.open_tag_settings(window, cx);
                assert!(app.settings_section == SettingsSection::Tags);
            })
        });
        catalog::set_preferences(&Default::default());
        set_appearance(Appearance::Dark);
    }
}
