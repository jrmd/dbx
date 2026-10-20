use super::super::connection::{disclosure_header, picker_tile};
use super::super::*;
use crate::popups::DropdownMenu as _;
use gpui_component::{checkbox::Checkbox, menu::PopupMenuItem, switch::Switch};

const VAULT_TEXT_EDITOR_CONTEXT: &str = "DbxTextEditor VaultGate";

/// A tile on the new-connection grid: an engine, or a hosted service that
/// connects through one.
#[derive(Clone, Copy)]
enum PickerTile {
    Engine(DatabaseKind),
    Preset(&'static str, DatabaseKind),
}

/// The new-connection grid, grouped the way people describe their databases.
const KIND_GROUPS: [(&str, &[PickerTile]); 4] = [
    (
        "Relational",
        &[
            PickerTile::Engine(DatabaseKind::PostgreSQL),
            PickerTile::Engine(DatabaseKind::MySQL),
            PickerTile::Engine(DatabaseKind::SQLite),
            PickerTile::Engine(DatabaseKind::CockroachDB),
            PickerTile::Engine(DatabaseKind::DuckDB),
            PickerTile::Engine(DatabaseKind::ClickHouse),
            PickerTile::Engine(DatabaseKind::SqlServer),
            PickerTile::Engine(DatabaseKind::Snowflake),
        ],
    ),
    (
        "Document & key-value",
        &[
            PickerTile::Engine(DatabaseKind::Redis),
            PickerTile::Engine(DatabaseKind::MongoDB),
        ],
    ),
    (
        "Search & streaming",
        &[
            PickerTile::Engine(DatabaseKind::Elasticsearch),
            PickerTile::Engine(DatabaseKind::Kafka),
        ],
    ),
    (
        "Cloud & hosted",
        &[
            PickerTile::Engine(DatabaseKind::BigQuery),
            PickerTile::Engine(DatabaseKind::Turso),
            PickerTile::Engine(DatabaseKind::CloudflareD1),
            PickerTile::Preset("Redshift", DatabaseKind::PostgreSQL),
            PickerTile::Preset("Neon", DatabaseKind::PostgreSQL),
            PickerTile::Preset("Supabase", DatabaseKind::PostgreSQL),
            PickerTile::Preset("PlanetScale", DatabaseKind::MySQL),
        ],
    ),
];

impl DbxApp {
    /// A menu of existing folders with an inline form to create one. The
    /// draft keeps its folder until a new name is confirmed.
    fn render_folder_control(&self, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let current = self.draft.folder.read(cx).trim().to_owned();
        let options = self.folder_options(cx);
        let app = cx.entity().downgrade();
        let label = if current.is_empty() {
            "No folder".to_owned()
        } else {
            current.clone()
        };
        let trigger = button("connection-folder", "", ButtonKind::Quiet)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_left()
                    .child(label.clone()),
            )
            .tooltip(label.clone())
            .dropdown_caret(true)
            .w_full()
            .min_w_0()
            .h(editor::single_line_input_height(
                &self.draft.new_folder_editor,
                cx,
            ))
            .rounded(crate::theme::ui_px(5.))
            .font_weight(FontWeight::NORMAL)
            .accessible_name(format!("Folder, {label}"))
            .debug_selector(|| "connection-folder".into())
            .dropdown_menu(move |menu, _, _| {
                let none = app.clone();
                let create = app.clone();
                let mut menu = menu
                    .min_w(crate::theme::ui_px(240.))
                    .scrollable(true)
                    .max_h(crate::theme::ui_px(280.))
                    .item(
                        PopupMenuItem::new("No folder")
                            .checked(current.is_empty())
                            .on_click(move |_, _, cx| {
                                let _ = none.update(cx, |this, cx| this.choose_folder(None, cx));
                            }),
                    );
                if !options.is_empty() {
                    menu = menu.separator();
                }
                for folder in options.iter().cloned() {
                    let app = app.clone();
                    let chosen = folder == current;
                    let name = folder.clone();
                    menu = menu.item(PopupMenuItem::new(folder).checked(chosen).on_click(
                        move |_, _, cx| {
                            let name = name.clone();
                            let _ = app.update(cx, |this, cx| this.choose_folder(Some(name), cx));
                        },
                    ));
                }
                menu.separator()
                    .item(
                        PopupMenuItem::new("New folder…").on_click(move |_, window, cx| {
                            let _ = create
                                .update(cx, |this, cx| this.begin_folder_creation(window, cx));
                        }),
                    )
            });
        div()
            .id("connection-folder-control")
            .track_focus(&self.draft.folder_focus)
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(6.))
            .child(trigger)
            .when(self.draft.creating_folder, |view| {
                let focus = self.draft.new_folder_editor.read(cx).focus_handle();
                view.child(
                    div()
                        .id("connection-new-folder")
                        .flex()
                        .flex_col()
                        .gap(crate::theme::ui_px(6.))
                        // Return creates the folder; Escape abandons the form.
                        .on_action(cx.listener(|this, _: &editor::Enter, window, cx| {
                            this.confirm_folder_creation(window, cx)
                        }))
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape"
                                && !event.keystroke.modifiers.modified()
                            {
                                cx.stop_propagation();
                                this.reset_folder_creation(cx);
                                this.focus_folder_control(window, cx);
                                cx.notify();
                            }
                        }))
                        .child(editor::input(
                            self.draft.new_folder_editor.clone(),
                            focus,
                            false,
                            cx,
                        ))
                        .when_some(self.draft.folder_error.clone(), |view, error| {
                            view.child(
                                div()
                                    .id("connection-folder-error")
                                    .role(gpui::Role::Alert)
                                    .text_size(crate::theme::text_px(11.))
                                    .text_color(theme().danger_text)
                                    .child(error),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap(crate::theme::ui_px(6.))
                                .child(
                                    button("create-folder", "Create", ButtonKind::Primary)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm_folder_creation(window, cx)
                                        })),
                                )
                                .child(
                                    button("cancel-folder", "Cancel", ButtonKind::Quiet)
                                        .cursor_pointer()
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.reset_folder_creation(cx);
                                            this.focus_folder_control(window, cx);
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
            })
    }
}

impl DbxApp {
    pub(super) fn render_connection(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.vault_state != Some(VaultState::Unlocked) {
            return self.render_vault_gate(cx).into_any_element();
        }
        let name_focus = self.draft.connection_name_editor.read(cx).focus_handle();
        let url_focus = self.draft.connection_editor.read(cx).focus_handle();
        let host_focus = self.draft.host_editor.read(cx).focus_handle();
        let port_focus = self.draft.port_editor.read(cx).focus_handle();
        let username_focus = self.draft.username_editor.read(cx).focus_handle();
        let password_focus = self.draft.password_editor.read(cx).focus_handle();
        let database_focus = self.draft.database_editor.read(cx).focus_handle();
        let kind = self.draft.kind;
        let details = self.draft.driver_manifest.is_none()
            && self.draft.mode == ConnectionFormMode::Details
            && kind.supports_details();
        let tags_editor = self.render_connection_tags(cx);
        let saved_connections = self.saved_connections.clone();
        let selected_profile = self.draft.selected_profile;
        let editing = selected_profile.is_some();
        let choosing = !editing && self.draft.choosing_kind;
        let kind_picker = choosing.then(|| self.render_kind_picker(cx));
        let transport = self.render_connection_transport(cx);
        let auth = self.render_connection_auth(cx);
        let protected = self.draft.read_only;

        if compact_connection_picker_visible(
            self.compact_layout,
            self.compact_connection_form_open,
            saved_connections.len(),
        ) {
            return self
                .render_compact_connection_picker(saved_connections, selected_profile, cx)
                .into_any_element();
        }

        let compact = self.compact_layout;
        let section_title = |title: &'static str| {
            div()
                .text_size(crate::theme::text_px(11.))
                .text_color(theme().text_muted)
                .child(title)
        };
        let field_label = |label: &'static str| {
            div()
                .text_size(crate::theme::text_px(11.))
                .text_color(theme().text_muted)
                .child(label)
        };
        let busy = self.saving_connection || self.credential_hydrating;
        let more_menu = (choosing).then(|| {
            let app = cx.entity().downgrade();
            let saving = self.saving_connection;
            let loading = self.drivers_loading();
            button("connection-picker-more", "More", ButtonKind::Quiet)
                .dropdown_caret(true)
                .h(crate::theme::control_height(26.))
                .text_size(crate::theme::text_px(11.))
                .accessible_name("More connection actions")
                .debug_selector(|| "connection-picker-more".into())
                .dropdown_menu(move |menu, _, _| {
                    let import = app.clone();
                    let demo = app.clone();
                    let register = app.clone();
                    let reveal = app.clone();
                    let refresh = app.clone();
                    menu.item(PopupMenuItem::new("Import connections…").on_click(
                        move |_, window, cx| {
                            let _ = import
                                .update(cx, |this, cx| this.open_profile_transfer(2, window, cx));
                        },
                    ))
                    .item(
                        PopupMenuItem::new(if saving {
                            "Creating demo…"
                        } else {
                            "Try a demo database"
                        })
                        .disabled(saving)
                        .on_click(move |_, window, cx| {
                            let _ =
                                demo.update(cx, |this, cx| this.create_demo_connection(window, cx));
                        }),
                    )
                    .separator()
                    .item(PopupMenuItem::new("Register community driver…").on_click(
                        move |_, window, cx| {
                            let _ =
                                register.update(cx, |this, cx| this.register_driver(window, cx));
                        },
                    ))
                    .item(
                        PopupMenuItem::new("Show drivers folder").on_click(move |_, _, cx| {
                            let _ = reveal.update(cx, |this, cx| this.reveal_drivers_folder(cx));
                        }),
                    )
                    .item(
                        PopupMenuItem::new("Refresh drivers")
                            .disabled(loading)
                            .on_click(move |_, _, cx| {
                                let _ =
                                    refresh.update(cx, |this, cx| this.refresh_driver_registry(cx));
                            }),
                    )
                })
        });
        // A saved connection keeps its engine; a new one can go back to the grid.
        let header = div()
            .id("connection-form-header")
            .flex()
            .items_center()
            .justify_between()
            .gap(crate::theme::ui_px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(crate::theme::text_px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if editing {
                        "Edit connection"
                    } else {
                        "New connection"
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(8.))
                    .children(more_menu)
                    .when(!choosing, |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(crate::theme::ui_px(6.))
                                .text_size(crate::theme::text_px(12.))
                                .text_color(theme().text_muted)
                                .child(database_logo(kind, theme().text_muted))
                                .child(kind.to_string()),
                        )
                    })
                    .when(!choosing && !editing, |view| {
                        view.child(
                            button("change-connection-kind", "Change", ButtonKind::Quiet)
                                .h(crate::theme::control_height(26.))
                                .text_size(crate::theme::text_px(11.))
                                .cursor_pointer()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.change_connection_kind(cx)),
                                ),
                        )
                    })
                    .when(editing, |view| {
                        view.child(
                            button("duplicate-connection", "Duplicate", ButtonKind::Quiet)
                                .h(crate::theme::control_height(26.))
                                .text_size(crate::theme::text_px(11.))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.duplicate_profile(cx))),
                        )
                        .child(
                            button("delete-connection", "Delete…", ButtonKind::Quiet)
                                .h(crate::theme::control_height(26.))
                                .text_size(crate::theme::text_px(11.))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request_delete_profile(window, cx)
                                })),
                        )
                    }),
            );
        let (auth_in_details, auth_in_string) = if details { (auth, None) } else { (None, auth) };
        let format_track =
            (kind.supports_details() && self.draft.driver_manifest.is_none()).then(|| {
                div().flex().child(
                    segmented_track()
                        .id("connection-format")
                        .role(gpui::Role::RadioGroup)
                        .aria_label("Connection format")
                        .child(
                            segment("Details", details)
                                .id("connection-details-mode")
                                .pressable("Connection details")
                                .role(gpui::Role::RadioButton)
                                .aria_toggled(if details {
                                    gpui::accesskit::Toggled::True
                                } else {
                                    gpui::accesskit::Toggled::False
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_connection_form_mode(ConnectionFormMode::Details, cx)
                                })),
                        )
                        .child(
                            segment("Connection string", !details)
                                .id("connection-string-mode")
                                .pressable("Connection string")
                                .role(gpui::Role::RadioButton)
                                .aria_toggled(if !details {
                                    gpui::accesskit::Toggled::True
                                } else {
                                    gpui::accesskit::Toggled::False
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_connection_form_mode(
                                        ConnectionFormMode::ConnectionString,
                                        cx,
                                    )
                                })),
                        ),
                )
            });
        let cloud_auth = self.draft.cloud_auth.is_some();
        let safety = div()
            .border_t_1()
            .border_color(theme().hairline)
            .pt(crate::theme::ui_px(12.))
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(8.))
            .child(section_title("Safety"))
            .child(
                Switch::new("connection-protected")
                    .label("Protected connection · disable writes")
                    .checked(protected)
                    .with_size(Size::Small)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.read_only = *checked;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("connection-capabilities")
                    .text_size(crate::theme::text_px(11.))
                    .text_color(theme().text_muted)
                    .child(if self.draft.driver_manifest.is_some() {
                        dbx_core::driver_plugin::capabilities(kind, protected).summary()
                    } else {
                        kind.capabilities(protected).summary()
                    }),
            )
            .when(protected, |view| {
                view.child(
                    div()
                        .text_size(crate::theme::text_px(11.))
                        .text_color(theme().text_muted)
                        .child("Use a read-only database account to enforce permissions."),
                )
            });
        let folder_control = self.render_folder_control(cx);
        let organize = div()
            .border_t_1()
            .border_color(theme().hairline)
            .pt(crate::theme::ui_px(12.))
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(8.))
            .child(section_title("Organize"))
            .child(
                div()
                    .flex()
                    .when(compact, |row| row.flex_col())
                    .gap(crate::theme::ui_px(12.))
                    .child(
                        div()
                            .when(!compact, |field| {
                                field.w(crate::theme::ui_px(240.)).flex_none()
                            })
                            .flex()
                            .flex_col()
                            .gap(crate::theme::ui_px(5.))
                            .child(field_label("Folder (optional)"))
                            .child(folder_control),
                    )
                    .child(div().flex_1().min_w_0().child(tags_editor)),
            );
        let form = div()
            .bg(theme().canvas)
            .py(crate::theme::ui_px(16.))
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(crate::theme::ui_px(5.))
                    .child(field_label("Connection name"))
                    .child(editor::input(
                        self.draft.connection_name_editor.clone(),
                        name_focus,
                        false,
                        cx,
                    )),
            )
            .children(format_track)
            .when(details, |view| {
                view.child(
                    div()
                        .flex()
                        .gap(crate::theme::ui_px(8.))
                        .child(div().flex_1().min_w_0().child(field_label("Host")).child(
                            editor::input(self.draft.host_editor.clone(), host_focus, false, cx),
                        ))
                        .child(
                            div()
                                .w(crate::theme::ui_px(110.))
                                .flex_none()
                                .child(field_label("Port"))
                                .child(editor::input(
                                    self.draft.port_editor.clone(),
                                    port_focus,
                                    false,
                                    cx,
                                )),
                        ),
                )
                .children(auth_in_details)
                .child(
                    div()
                        .flex()
                        .gap(crate::theme::ui_px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(field_label("Username"))
                                .child(editor::input(
                                    self.draft.username_editor.clone(),
                                    username_focus,
                                    false,
                                    cx,
                                )),
                        )
                        // Token authentication has no password to enter.
                        .when(!cloud_auth, |row| {
                            row.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(field_label("Password"))
                                    .child(editor::input(
                                        self.draft.password_editor.clone(),
                                        password_focus.clone(),
                                        false,
                                        cx,
                                    )),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(crate::theme::ui_px(5.))
                        .child(field_label(if kind == DatabaseKind::Redis {
                            "Database index (optional)"
                        } else {
                            "Database"
                        }))
                        .child(editor::input(
                            self.draft.database_editor.clone(),
                            database_focus,
                            false,
                            cx,
                        )),
                )
            })
            .when(!kind.connection_help().is_empty(), |view| {
                view.child(
                    div()
                        .text_size(crate::theme::text_px(11.))
                        .text_color(theme().text_muted)
                        .child(kind.connection_help()),
                )
            })
            .when(!details, |view| {
                view.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(crate::theme::ui_px(5.))
                        .child(field_label(if kind.is_file() {
                            "Database file or connection string"
                        } else {
                            "Connection string"
                        }))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(crate::theme::ui_px(8.))
                                .child(div().flex_1().min_w_0().child(editor::input(
                                    self.draft.connection_editor.clone(),
                                    url_focus,
                                    false,
                                    cx,
                                )))
                                .when(
                                    self.draft.driver_manifest.is_none() && kind.is_file(),
                                    |view| {
                                        view.child(
                                            button(
                                                "choose-sqlite-file",
                                                "Choose file…",
                                                ButtonKind::Quiet,
                                            )
                                            .h(crate::theme::control_height(32.))
                                            .flex_none()
                                            .cursor_pointer()
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.choose_sqlite_file(cx)
                                                }),
                                            ),
                                        )
                                    },
                                ),
                        ),
                )
                .children(auth_in_string)
            })
            .when(
                matches!(
                    kind,
                    DatabaseKind::BigQuery | DatabaseKind::Turso | DatabaseKind::CloudflareD1
                ),
                |view| {
                    view.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(crate::theme::ui_px(5.))
                            .child(field_label(if kind == DatabaseKind::BigQuery {
                                "OAuth token (optional; leave blank for Google credentials)"
                            } else {
                                "API token"
                            }))
                            .child(editor::input(
                                self.draft.password_editor.clone(),
                                password_focus,
                                false,
                                cx,
                            )),
                    )
                },
            )
            .when(
                self.draft.driver_manifest.is_none() && kind.supports_transport(),
                |view| view.child(transport),
            )
            .child(safety)
            .child(organize);

        div().id(SharedString::from(format!("connection-setup-{kind}-{editing}-{choosing}"))).role(gpui::Role::Pane).aria_label(if choosing {
            "Choose a database for a new connection".to_owned()
        } else if editing {
            format!("Edit {kind} connection")
        } else {
            format!("New {kind} connection")
        }).flex_1().min_w_0().min_h_0().flex().gap(crate::theme::ui_px(GLASS_INSET)).pr(crate::theme::ui_px(GLASS_INSET)).pb(crate::theme::ui_px(GLASS_INSET))
            .when(!self.compact_layout, |view| view.child(
                glass(div(), RADIUS_GLASS, 8.).w(crate::theme::ui_px(264.)).flex_none().overflow_hidden()
                    .flex().flex_col()
                    .child(div().h(crate::theme::control_height(48.)).flex_none().pl(crate::theme::ui_px(16.)).pr(crate::theme::ui_px(10.)).flex().items_center().justify_between().border_b_1().border_color(theme().hairline)
                        .child(div().flex().items_center().gap(crate::theme::ui_px(8.)).child(div().text_size(crate::theme::text_px(13.)).font_weight(FontWeight::SEMIBOLD).child("Connections")).when(!saved_connections.is_empty(), |view| view.child(badge(saved_connections.len().to_string(), theme().text_muted))))
                        .child(glass_icon_button("new-connection-from-list", Icon::Add, false).tooltip(tip("New connection")).on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx)))))
                    .child(div().id("saved-connections").flex_1().min_h_0().overflow_y_scroll().p(crate::theme::ui_px(8.)).flex().flex_col().gap(crate::theme::ui_px(2.))
                        .when(saved_connections.is_empty(), |view| view.child(div().p(crate::theme::ui_px(10.)).text_size(crate::theme::text_px(12.)).text_color(theme().text_muted).child("No saved connections")))
                        .children(grouped_connections(saved_connections).into_iter().map(|(folder, profile)| {
                            let id = profile.id; let selected = selected_profile == Some(id); let choose = profile.clone();
                            let row = div().id(SharedString::from(format!("saved-connection-{id}"))).pressable(profile.name.clone()).h(crate::theme::control_height(50.)).px(crate::theme::ui_px(10.)).rounded(px(RADIUS_PANEL - 2.)).when(selected, |row| row.bg(theme().accent_soft)).when(!selected, |row| row.hover(|style| style.bg(theme().glass_hover))).cursor_pointer().flex().items_center().gap(crate::theme::ui_px(10.))
                                .child(database_logo(profile.kind, if selected { theme().accent } else { theme().text_muted }))
                                .child(div().flex_1().min_w_0().flex().flex_col().gap(crate::theme::ui_px(1.)).child(div().truncate().text_size(crate::theme::text_px(12.)).font_weight(FontWeight::MEDIUM).child(profile.name)).child(div().truncate().text_size(crate::theme::text_px(11.)).text_color(theme().text_muted).child(display_url(&profile.url))))
                                .children(tag_badge(profile.tag.as_ref()))
                                .on_click(cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                                    let click_count = match event {
                                        gpui::ClickEvent::Mouse(mouse) => mouse.up.click_count,
                                        // Enter opens the connection, like a double-click.
                                        gpui::ClickEvent::Keyboard(key) if key.button == gpui::KeyboardButton::Enter => 2,
                                        gpui::ClickEvent::Keyboard(_) | gpui::ClickEvent::Touch(_) => 1,
                                    };
                                    match saved_connection_click_action(click_count) {
                                        SavedConnectionClickAction::Select => this.select_saved_connection(choose.clone(), cx),
                                        SavedConnectionClickAction::Open => this.open_saved_connection(choose.clone(), window, cx),
                                    }
                                }));
                            connection_group_row(folder, row.into_any_element())
                        }))),
            ))
            .child(div().flex_1().min_w_0().flex().flex_col().overflow_hidden().rounded(crate::theme::ui_px(RADIUS_PANEL)).border_1().border_color(theme().hairline).bg(theme().canvas).shadow(glass_shadow(10.))
                .child(div().id("connection-form-scroll").flex_1().min_h_0().overflow_y_scroll().p(if self.compact_layout { crate::theme::ui_px(14.) } else { crate::theme::ui_px(24.) }).flex().justify_center().items_start()
                    .child(div().w_full().min_w_0().max_w(crate::theme::ui_px(720.)).flex().flex_col().gap(crate::theme::ui_px(14.))
                        .when(self.compact_layout && !self.saved_connections.is_empty(), |view| view.child(
                            div().flex().child(button("back-to-saved-connections", "Saved connections", ButtonKind::Quiet).cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.show_saved_connections(cx))))
                        ))
                        .child(header)
                        .when_some(self.draft.driver_manifest.clone(), |view, path| {
                            let driver = dbx_core::driver_plugin::DriverManifest::read(&path).map(|manifest| format!("Community driver: {} · {}", manifest.name, manifest.executable.display())).unwrap_or_else(|_| "Community driver manifest unavailable".into());
                            view.child(div().text_size(crate::theme::text_px(12.)).text_color(theme().text_muted).child(driver))
                        })
                        .children(kind_picker)
                        .when(!choosing, |view| view.child(form))))
                .when(!choosing, |view| view.child(div().flex_none().border_t_1().border_color(theme().border).follow_bottom_corners(RADIUS_PANEL).bg(theme().panel).px(if self.compact_layout { crate::theme::ui_px(14.) } else { crate::theme::ui_px(24.) }).py(crate::theme::ui_px(12.)).flex().items_center().justify_between().gap(crate::theme::ui_px(12.))
                    .child(div().min_w_0().flex_1().when_some(self.error.clone(), |view, error| view.child(crate::accessibility::announcement("connection-error", error.clone(), true).truncate().text_size(crate::theme::text_px(12.)).text_color(theme().danger_text).tooltip(tip(error.clone())).child(error))))
                    .child(div().flex_none().flex().items_center().gap(crate::theme::ui_px(8.))
                        .child(button("test-connection", if self.testing_connection { "Cancel test" } else if self.credential_hydrating { "Loading password…" } else { "Test connection" }, ButtonKind::Quiet).disabled(self.credential_hydrating && !self.testing_connection).when(!self.credential_hydrating || self.testing_connection, |button| button.cursor_pointer().on_click(cx.listener(|this, _, _, cx| if this.testing_connection { this.cancel_connection_test(cx) } else { this.test_connection(cx) }))))
                        .child(button("save-connection", if self.saving_connection { "Saving…" } else { "Save" }, ButtonKind::Quiet).disabled(self.vault_busy || self.saving_connection).when(!self.vault_busy && !self.saving_connection, |button| button.cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.save_connection(cx)))))
                        .child(button("connect", if self.credential_hydrating { "Loading password…" } else { "Connect" }, ButtonKind::Primary).when(!self.credential_hydrating, |button| button.cursor_pointer().on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))))))))).into_any_element()
    }

    /// The first step of a new connection: paste a URL to detect its type, or
    /// pick one from the grid.
    fn render_kind_picker(&mut self, cx: &mut Context<Self>) -> Div {
        let url_focus = self.draft.import_url_editor.read(cx).focus_handle();
        let detected = self.import_url_kind(cx);
        let url_empty = self.draft.import_url.read(cx).is_empty();
        let offer_demo = self.saved_connections.is_empty();
        let saving = self.saving_connection;
        let compact = self.compact_layout;
        div()
            .pt(crate::theme::ui_px(2.))
            .pb(crate::theme::ui_px(16.))
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(18.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(crate::theme::ui_px(6.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(crate::theme::ui_px(8.))
                            .child(
                                // Return in the field continues once the URL is recognised.
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .on_action(cx.listener(|this, _: &editor::Enter, _, cx| {
                                        this.continue_with_import_url(cx)
                                    }))
                                    .child(editor::input(
                                        self.draft.import_url_editor.clone(),
                                        url_focus,
                                        false,
                                        cx,
                                    ))
                                    .when(url_empty, |field| {
                                        field.child(
                                            div()
                                                .absolute()
                                                .top_0()
                                                .bottom_0()
                                                .left(crate::theme::ui_px(9.))
                                                .right(crate::theme::ui_px(9.))
                                                .flex()
                                                .items_center()
                                                .truncate()
                                                .text_size(crate::theme::text_px(12.))
                                                .text_color(theme().text_muted)
                                                .child("Paste a connection URL or file path"),
                                        )
                                    }),
                            )
                            .child(
                                button(
                                    "continue-with-url",
                                    match detected {
                                        Some(kind) => format!("Continue as {kind}"),
                                        None => "Continue".into(),
                                    },
                                    if detected.is_some() {
                                        ButtonKind::Primary
                                    } else {
                                        ButtonKind::Quiet
                                    },
                                )
                                .h(crate::theme::control_height(32.))
                                .flex_none()
                                .disabled(detected.is_none())
                                .when(
                                    detected.is_some(),
                                    |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            |this, _, _, cx| this.continue_with_import_url(cx),
                                        ))
                                    },
                                ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x(crate::theme::ui_px(6.))
                            .text_size(crate::theme::text_px(11.))
                            .text_color(theme().text_muted)
                            .child(match detected {
                                Some(kind) => format!("Detected {kind}. Press Return to continue."),
                                None => "Or choose a database below.".into(),
                            })
                            .when(offer_demo, |line| {
                                line.child(
                                    div()
                                        .id("try-demo")
                                        .debug_selector(|| "try-demo".into())
                                        .pressable("Try a demo database")
                                        .text_color(theme().accent)
                                        .rounded(crate::theme::ui_px(3.))
                                        .when(!saving, |link| {
                                            link.cursor_pointer()
                                                .hover(|style| style.underline())
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.create_demo_connection(window, cx)
                                                }))
                                        })
                                        .child(if saving {
                                            "Creating demo…"
                                        } else {
                                            "Try a demo database"
                                        }),
                                )
                            }),
                    ),
            )
            .children(KIND_GROUPS.into_iter().map(|(title, tiles)| {
                div()
                    .flex()
                    .flex_col()
                    .gap(crate::theme::ui_px(8.))
                    .child(
                        div()
                            .text_size(crate::theme::text_px(11.))
                            .text_color(theme().text_muted)
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(crate::theme::ui_px(8.))
                            .children(tiles.iter().copied().map(|tile| {
                                match tile {
                                    PickerTile::Engine(kind) => picker_tile(
                                        SharedString::from(format!("engine-{kind}")),
                                        kind,
                                        kind.to_string(),
                                        None,
                                        false,
                                        compact,
                                    )
                                    .debug_selector(move || format!("engine-{kind}"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_kind(kind, cx)
                                    })),
                                    PickerTile::Preset(name, kind) => picker_tile(
                                        SharedString::from(format!("preset-{name}")),
                                        kind,
                                        name,
                                        Some(if kind == DatabaseKind::MySQL {
                                            "MySQL"
                                        } else {
                                            "PostgreSQL"
                                        }),
                                        false,
                                        compact,
                                    )
                                    .debug_selector(move || format!("preset-{name}"))
                                    .aria_label(format!("{name} ({kind})"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_connection_preset(name, cx)
                                    })),
                                }
                            })),
                    )
            }))
            .child(self.render_driver_picker(cx))
    }

    /// Tags are picked here and managed in Settings, so the form stays short.
    fn render_connection_tags(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(crate::theme::ui_px(5.))
            .child(
                div()
                    .text_size(crate::theme::text_px(11.))
                    .text_color(theme().text_muted)
                    .child("Tags"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(crate::theme::ui_px(6.))
                    .children(self.connection_tags.iter().map(|tag| {
                        let selected = self
                            .draft
                            .tag
                            .as_ref()
                            .is_some_and(|item| item.id == tag.id);
                        let choose = tag.clone();
                        div()
                            .id(SharedString::from(format!("tag-{}", tag.id)))
                            .pressable(tag.name.clone())
                            .px(crate::theme::ui_px(10.))
                            .h(crate::theme::control_height(26.))
                            .rounded_full()
                            .border_1()
                            .border_color(gpui::rgb(tag.color))
                            .when(selected, |view| view.bg(gpui::rgb(tag.color).alpha(0.15)))
                            .text_color(gpui::rgb(tag.color))
                            .text_size(crate::theme::text_px(11.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .child(format!("{}{}", if selected { "✓ " } else { "" }, tag.name))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_tag(choose.clone(), cx)
                            }))
                    }))
                    .child(
                        button("manage-tags", "Manage tags…", ButtonKind::Quiet)
                            .h(crate::theme::control_height(26.))
                            .text_size(crate::theme::text_px(11.))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_tag_settings(window, cx)
                            })),
                    ),
            )
    }

    /// Authentication for PostgreSQL and MySQL: a password, or a short-lived
    /// token the cloud CLI issues on each connect. Community drivers reject it.
    fn render_connection_auth(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let kind = self.draft.kind;
        if !matches!(kind, DatabaseKind::PostgreSQL | DatabaseKind::MySQL)
            || self.draft.driver_manifest.is_some()
        {
            return None;
        }
        let current = self.draft.cloud_auth;
        let option = |id: &'static str,
                      label: &'static str,
                      value: Option<dbx_core::CloudAuthentication>| {
            let selected = current == value;
            segment(label, selected)
                .id(id)
                .pressable(label)
                .role(gpui::Role::RadioButton)
                .aria_toggled(if selected {
                    gpui::accesskit::Toggled::True
                } else {
                    gpui::accesskit::Toggled::False
                })
                .on_click(cx.listener(move |this, _, _, cx| this.set_cloud_auth(value, cx)))
        };
        let caption = match current {
            None => None,
            Some(dbx_core::CloudAuthentication::AwsRdsIam) => Some(
                "DBX gets a fresh token from the AWS CLI on each connect. Sign in with `aws` first; the username is the IAM database user.",
            ),
            Some(dbx_core::CloudAuthentication::AzureEntra) => Some(
                "DBX gets a fresh token from the Azure CLI on each connect. Sign in with `az login` first.",
            ),
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(crate::theme::ui_px(6.))
                .child(
                    div()
                        .text_size(crate::theme::text_px(11.))
                        .text_color(theme().text_muted)
                        .child("Authentication"),
                )
                .child(
                    div().flex().child(
                        segmented_track()
                            .id("connection-auth")
                            .role(gpui::Role::RadioGroup)
                            .aria_label("Authentication")
                            .child(option("auth-password", "Password", None))
                            .child(option(
                                "auth-aws-iam",
                                "AWS IAM",
                                Some(dbx_core::CloudAuthentication::AwsRdsIam),
                            ))
                            .when(kind == DatabaseKind::PostgreSQL, |track| {
                                track.child(option(
                                    "auth-entra",
                                    if self.compact_layout {
                                        "Entra"
                                    } else {
                                        "Microsoft Entra"
                                    },
                                    Some(dbx_core::CloudAuthentication::AzureEntra),
                                ))
                            }),
                    ),
                )
                .children(caption.map(|caption| {
                    div()
                        .text_size(crate::theme::text_px(11.))
                        .text_color(theme().text_muted)
                        .child(caption)
                }))
                .into_any_element(),
        )
    }

    /// Unix socket and SSH tunnel, folded away until one is configured.
    fn render_connection_transport(&self, cx: &mut Context<Self>) -> AnyElement {
        let transport = &self.draft.transport;
        let socket_enabled = transport.socket_enabled;
        let ssh_enabled = transport.ssh_enabled;
        let open = self
            .connection_network_open
            .unwrap_or(socket_enabled || ssh_enabled);
        let detail: SharedString = match (ssh_enabled, socket_enabled) {
            (true, socket) => {
                let host = transport.ssh_host.read(cx).trim().to_owned();
                let ssh = if host.is_empty() {
                    "SSH tunnel".to_owned()
                } else {
                    format!("SSH via {host}")
                };
                if socket {
                    format!("{ssh} · Unix socket").into()
                } else {
                    ssh.into()
                }
            }
            (false, true) => "Unix socket".into(),
            (false, false) => "SSH tunnel, Unix socket".into(),
        };
        let input = |label: &'static str, entity: Entity<TextEditor>| {
            let focus = entity.read(cx).focus_handle();
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(crate::theme::ui_px(5.))
                .child(
                    div()
                        .text_size(crate::theme::text_px(11.))
                        .text_color(theme().text_muted)
                        .child(label),
                )
                .child(editor::input(entity, focus, false, cx))
                .into_any_element()
        };
        let socket = input(
            if self.draft.kind == DatabaseKind::PostgreSQL {
                "Socket directory (e.g. /var/run/postgresql)"
            } else {
                "Socket file (e.g. /tmp/mysql.sock)"
            },
            transport.socket_editor.clone(),
        );
        let host = input("SSH host", transport.ssh_host_editor.clone());
        let port = input("SSH port", transport.ssh_port_editor.clone());
        let user = input("SSH username", transport.ssh_user_editor.clone());
        let key = input(
            "Private key file (optional; leave blank to use SSH agent/config)",
            transport.ssh_key_editor.clone(),
        );
        let password = input(
            "SSH password (optional; stored in vault)",
            transport.ssh_password_editor.clone(),
        );
        let jump = input(
            "Jump hosts (optional; user@bastion:22, comma-separated)",
            transport.ssh_jump_editor.clone(),
        );
        div().debug_selector(|| "connection-transport-form".into()).border_t_1().border_color(theme().hairline).pt(crate::theme::ui_px(12.))
            .flex().flex_col().gap(crate::theme::ui_px(10.))
            .child(disclosure_header("connection-network", "Network", None, Some(detail), open)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.connection_network_open = Some(!open);
                    cx.notify();
                })))
            .when(open, |view| view
                .child(Switch::new("connection-unix-socket").label("Use Unix socket")
                    .checked(socket_enabled).with_size(Size::Small)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.transport.socket_enabled = *checked;
                        this.error = None;
                        cx.notify();
                    })))
                .when(socket_enabled, |view| view.child(socket).child(
                    div().text_size(crate::theme::text_px(11.)).text_color(theme().text_muted)
                        .child(if ssh_enabled { "Socket on the SSH server. PostgreSQL uses the database port to select its socket." }
                            else { "Local socket. Host is ignored; PostgreSQL uses the database port to select its socket." })))
                .child(Switch::new("connection-ssh-tunnel").label("Connect through SSH tunnel")
                    .checked(ssh_enabled).with_size(Size::Small)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.transport.ssh_enabled = *checked;
                        this.error = None;
                        cx.notify();
                    })))
                .when(ssh_enabled, |view| view
                    .child(div().flex().gap(crate::theme::ui_px(8.)).child(host).child(div().w(crate::theme::ui_px(110.)).child(port)))
                    .child(user)
                    .child(div().flex().items_end().gap(crate::theme::ui_px(8.)).child(key)
                        .child(button("choose-ssh-key", "Choose key…", ButtonKind::Quiet)
                            .cursor_pointer().on_click(cx.listener(|this, _, _, cx| this.choose_ssh_key(cx)))))
                    .child(jump).child(password)
                    .child(div().text_size(crate::theme::text_px(11.)).text_color(theme().text_muted)
                        .child("Database host is resolved from the SSH server. Uses your SSH agent or key file."))
                    .child(div().text_size(crate::theme::text_px(11.)).text_color(theme().text_muted)
                        .child("Load encrypted keys into the agent, and verify a new SSH host in a terminal first."))))
            .into_any_element()
    }

    fn render_compact_connection_picker(
        &mut self,
        saved_connections: Vec<SavedConnection>,
        selected_profile: Option<Uuid>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let profile_count = saved_connections.len();
        let has_selected_profile = selected_profile.is_some();
        div()
            .id("compact-connection-picker")
            .flex_1()
            .min_h_0()
            .flex()
            .justify_center()
            .p(crate::theme::ui_px(14.))
            .pt(crate::theme::ui_px(4.))
            .child(
                div()
                    .w_full()
                    .max_w(crate::theme::ui_px(720.))
                    .h_full()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .gap(crate::theme::ui_px(14.))
                    .child(
                        div()
                            .text_size(crate::theme::text_px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Connections"),
                    )
                    .child(button("compact-import-connections", "Import connections…", ButtonKind::Quiet).on_click(cx.listener(|this, _, window, cx| this.open_profile_transfer(2, window, cx))))
                    .child(
                        glass(div(), RADIUS_GLASS, 8.)
                            .flex_1()
                            .min_h_0()
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .h(crate::theme::control_height(44.))
                                    .flex_none()
                                    .px(crate::theme::ui_px(12.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .border_b_1()
                                    .border_color(theme().hairline)
                                    .child(
                                        div()
                                            .text_size(crate::theme::text_px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Saved connections"),
                                    )
                                    .child(
                                        div()
                                            .text_size(crate::theme::text_px(11.))
                                            .text_color(theme().text_muted)
                                            .child(profile_count.to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .id("saved-connections")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .p(crate::theme::ui_px(8.))
                                    .flex()
                                    .flex_col()
                                    .gap(crate::theme::ui_px(3.))
                                    .children(grouped_connections(saved_connections).into_iter().map(|(folder, profile)| {
                                        let id = profile.id;
                                        let selected = selected_profile == Some(id);
                                        let choose = profile.clone();
                                        let row = div()
                                            .id(SharedString::from(format!(
                                                "saved-connection-{id}"
                                            )))
                                            .pressable(profile.name.clone())
                                            .h(crate::theme::control_height(50.))
                                            .flex_none()
                                            .px(crate::theme::ui_px(10.))
                                            .rounded(px(RADIUS_PANEL - 2.))
                                            .when(selected, |row| row.bg(theme().accent_soft))
                                            .when(!selected, |row| {
                                                row.hover(|style| style.bg(theme().glass_hover))
                                            })
                                            .cursor_pointer()
                                            .flex()
                                            .items_center()
                                            .gap(crate::theme::ui_px(8.))
                                            .child(database_logo(
                                                profile.kind,
                                                if selected {
                                                    theme().accent
                                                } else {
                                                    theme().text_muted
                                                },
                                            ))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_size(crate::theme::text_px(12.))
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .child(profile.name),
                                                    )
                                                    .child(
                                                        div()
                                                            .truncate()
                                                            .text_size(crate::theme::text_px(11.))
                                                            .text_color(theme().text_muted)
                                                            .child(display_url(&profile.url)),
                                                    ),
                                            )
                                            .children(tag_badge(profile.tag.as_ref()))
                                            .on_click(cx.listener(
                                                move |this,
                                                      event: &gpui::ClickEvent,
                                                      window,
                                                      cx| {
                                                    let click_count = match event {
                                                        gpui::ClickEvent::Mouse(mouse) => {
                                                            mouse.up.click_count
                                                        }
                                                        // Enter opens, like a double-click.
                                                        gpui::ClickEvent::Keyboard(key)
                                                            if key.button == gpui::KeyboardButton::Enter => 2,
                                                        gpui::ClickEvent::Keyboard(_)
                                                        | gpui::ClickEvent::Touch(_) => 1,
                                                    };
                                                    match saved_connection_click_action(click_count)
                                                    {
                                                        SavedConnectionClickAction::Select => this
                                                            .select_saved_connection_in_compact_picker(
                                                                choose.clone(),
                                                                cx,
                                                            ),
                                                        SavedConnectionClickAction::Open => this
                                                            .open_saved_connection(
                                                                choose.clone(),
                                                                window,
                                                                cx,
                                                            ),
                                                    }
                                                },
                                            ));
                                        connection_group_row(folder, row.into_any_element())
                                    })),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .border_t_1()
                                    .border_color(theme().hairline)
                                    .p(crate::theme::ui_px(12.))
                                    .child(
                                        div()
                                            .flex()
                                            .gap(crate::theme::ui_px(8.))
                                            .when(has_selected_profile, |view| {
                                                view.child(
                                                    button(
                                                        "compact-edit-connection",
                                                        "Edit selected",
                                                        ButtonKind::Quiet,
                                                    )
                                                    .flex_1()
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.show_selected_connection_form(cx)
                                                    })),
                                                )
                                            })
                                            .child(
                                                button(
                                                    "compact-new-connection",
                                                    "New connection",
                                                    ButtonKind::Primary,
                                                )
                                                .flex_1()
                                                .cursor_pointer()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.begin_new_connection(cx)
                                                })),
                                            ),
                                    ),
                            ),
                    ),
            )
    }

    /// The vault gate borrows the macOS lock screen: a centred identity, one
    /// compact passphrase pill with an inline submit arrow, and quiet options
    /// beneath. Nothing is boxed in a card; the window material is the stage.
    fn render_vault_gate(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.vault_state;
        let creating = state == Some(VaultState::Uninitialized);
        let unavailable = state.is_none();
        let busy = self.vault_busy;
        // Touch ID state is owned by the app; this view only reflects it.
        let touch_id_available = self.touch_id_available;
        let touch_id_enabled = self.touch_id_enabled;
        let touch_id_ready = touch_id_enabled && touch_id_available && self.touch_id_enrolled;
        let show_touch_id_unlock = touch_id_ready && !creating && !unavailable;
        let passphrase_focus = self.vault_editors.passphrase_editor.read(cx).focus_handle();
        let confirmation_focus = self
            .vault_editors
            .confirmation_editor
            .read(cx)
            .focus_handle();

        let field_width = crate::theme::ui_px(272.);
        // The editor shell itself becomes the pill, keeping its scale-aware
        // height and vertical padding so text never clips at large UI sizes.
        let pill =
            |editor: Entity<TextEditor>, focus: FocusHandle, trailing_room: bool, cx: &App| {
                editor::input_with_key_context(editor, focus, false, VAULT_TEXT_EDITOR_CONTEXT, cx)
                    .rounded_full()
                    .bg(theme().glass_hover)
                    .border_color(theme().hairline)
                    // Restate the focus ring after recolouring the resting edge.
                    .focus(|style| style.border_color(theme().focus_ring))
                    .pl(crate::theme::ui_px(14.))
                    .pr(crate::theme::ui_px(if trailing_room { 38. } else { 14. }))
            };
        // Compact external labels name each field while creating, where two
        // identical masked pills would otherwise be indistinguishable.
        let labeled = |label: &'static str, field: AnyElement| {
            div()
                .w(field_width)
                .max_w_full()
                .flex()
                .flex_col()
                .gap(crate::theme::ui_px(4.))
                .child(
                    div()
                        .pl(crate::theme::ui_px(14.))
                        .text_size(crate::theme::text_px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme().text_muted)
                        .child(label),
                )
                .child(field)
        };
        let submit_label = match (busy, creating) {
            (true, true) => "Creating vault…",
            (true, false) => "Unlocking…",
            (false, true) => "Create vault",
            (false, false) => "Unlock",
        };
        // A real button: a Tab stop with Return/Space activation and the
        // shared focus ring, not just a clickable circle.
        let submit = Button::new("submit-vault-passphrase")
            .debug_selector(|| "vault-submit".into())
            .with_size(Size::XSmall)
            .compact()
            .accessible_name(submit_label)
            .tooltip(submit_label)
            .size(crate::theme::ui_px(24.))
            .p_0()
            .flex_none()
            .rounded_full()
            .border_0()
            .bg(theme().accent)
            .child(icon(Icon::ArrowRight, theme().accent_foreground).size(crate::theme::ui_px(14.)))
            .disabled(busy)
            .when(busy, |button| button.opacity(0.45))
            .when(!busy, |button| {
                button.cursor_pointer().on_click(
                    cx.listener(move |this, _, _, cx| this.submit_vault_passphrase(creating, cx)),
                )
            });
        // The arrow rides inside the last field, as on the macOS login window.
        let (submit_editor, submit_focus) = if creating {
            (
                self.vault_editors.confirmation_editor.clone(),
                confirmation_focus,
            )
        } else {
            (
                self.vault_editors.passphrase_editor.clone(),
                passphrase_focus.clone(),
            )
        };
        let submit_field = div()
            .relative()
            .w_full()
            .child(pill(submit_editor, submit_focus, true, cx))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right(crate::theme::ui_px(5.))
                    .flex()
                    .items_center()
                    .child(submit),
            );
        let fields = if creating {
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(crate::theme::ui_px(12.))
                .child(labeled(
                    "Passphrase",
                    pill(
                        self.vault_editors.passphrase_editor.clone(),
                        passphrase_focus,
                        false,
                        cx,
                    )
                    .into_any_element(),
                ))
                .child(labeled(
                    "Confirm passphrase",
                    submit_field.into_any_element(),
                ))
        } else {
            div().w(field_width).max_w_full().child(submit_field)
        };
        let caption = |text: SharedString| {
            div()
                .text_size(crate::theme::text_px(11.))
                .text_color(theme().text_muted)
                .text_center()
                .child(text)
        };

        let title = if unavailable {
            "DBX Vault unavailable"
        } else if creating {
            "Create DBX Vault"
        } else {
            "DBX Vault"
        };
        let subtitle = if unavailable {
            "Saved connection passwords can’t be read on this device."
        } else if creating {
            "Choose a passphrase to protect saved connection passwords."
        } else {
            "Enter your passphrase to unlock saved connections."
        };
        // Below the field: progress first, then the most useful hint.
        let hint: Option<SharedString> = if busy {
            Some(submit_label.into())
        } else if creating {
            Some("At least 12 characters. If it’s lost, saved passwords can’t be recovered.".into())
        } else if show_touch_id_unlock {
            Some("Use Touch ID or enter your passphrase".into())
        } else if touch_id_enabled && !touch_id_available {
            Some("Touch ID isn’t available right now. Enter your passphrase.".into())
        } else {
            None
        };

        let use_touch_id = Checkbox::new("vault-use-touch-id")
            .label("Use Touch ID")
            .checked(touch_id_enabled)
            .with_size(Size::Small)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_touch_id(cx)));
        let touch_id_unlock = button(
            "vault-touch-id-unlock",
            "Unlock with Touch ID",
            ButtonKind::Quiet,
        )
        .debug_selector(|| "vault-touch-id-unlock".into())
        .when(busy, |button| button.disabled(true))
        .when(!busy, |button| {
            button
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.unlock_with_touch_id(cx)))
        });

        div().id("vault-gate").role(gpui::Role::Pane)
            .aria_label(if creating { "Create DBX Vault" } else { "Unlock DBX Vault" })
            .aria_description("Passphrases must contain at least 12 characters. If the passphrase is lost, saved passwords cannot be recovered.")
            .debug_selector(|| "vault-gate".into()).key_context("VaultGate").overflow_y_scroll()
            .on_action(cx.listener(move |this, _: &SubmitVault, _, cx| {
                if !this.vault_busy {
                    this.submit_vault_passphrase(creating, cx);
                }
            }))
            .flex_1().min_h_0().h_full()
            // Let the inner layout grow beyond the viewport so oversized
            // setup content contributes to the scrollable bounds.
            .child(div().w_full().min_h_full().flex().items_center().justify_center()
            .p(crate::theme::ui_px(24.))
            .child(div().id("vault-card").debug_selector(|| "vault-card".into())
                .min_w_0().flex_none().w_full().max_w(crate::theme::ui_px(360.))
                .flex().flex_col().items_center()
                // Nudge the composition above true centre, where the eye rests.
                .pb(crate::theme::ui_px(if self.compact_layout { 0. } else { 48. }))
                .child(
                    div()
                        .size(crate::theme::ui_px(88.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(theme().glass_raised)
                        .border_1()
                        .border_color(theme().hairline)
                        .shadow(glass_shadow(16.))
                        .child(img(self.logo.clone()).id("vault-logo").size(crate::theme::ui_px(54.))),
                )
                .child(
                    div()
                        .mt(crate::theme::ui_px(16.))
                        .text_size(crate::theme::text_px(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme().text)
                        .text_center()
                        .child(title),
                )
                .child(div().mt(crate::theme::ui_px(4.)).max_w(crate::theme::ui_px(300.)).child(caption(subtitle.into())))
                .when(!unavailable, |view| view
                    .child(div().mt(crate::theme::ui_px(22.)).w_full().flex().justify_center().child(fields))
                    .when_some(hint, |view, hint| view.child(div().mt(crate::theme::ui_px(10.)).max_w(field_width).child(caption(hint))))
                    .when_some(self.error.clone(), |view, error| view.child(
                        crate::accessibility::announcement("vault-error", error.clone(), true)
                            .mt(crate::theme::ui_px(6.))
                            .max_w(field_width)
                            .text_center()
                            .text_size(crate::theme::text_px(11.))
                            .text_color(theme().danger_text)
                            .child(error),
                    ))
                    .when(show_touch_id_unlock, |view| view.child(div().mt(crate::theme::ui_px(16.)).child(touch_id_unlock)))
                    .when(touch_id_available || touch_id_enabled, |view| view.child(
                        div()
                            .mt(crate::theme::ui_px(20.))
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(crate::theme::ui_px(8.))
                            // Stay visible while enabled so it can be turned off even
                            // when the sensor is unavailable (lid closed, no keyboard).
                            .child(use_touch_id),
                    )))
                .when(unavailable, |view| view.when_some(self.error.clone(), |view, error| view.child(
                    crate::accessibility::announcement("vault-error", error.clone(), true)
                        .mt(crate::theme::ui_px(10.))
                        .text_center()
                        .text_size(crate::theme::text_px(11.))
                        .text_color(theme().danger_text)
                        .child(error),
                )))))
    }
}

fn grouped_connections(
    mut profiles: Vec<SavedConnection>,
) -> Vec<(Option<String>, SavedConnection)> {
    profiles.sort_by_key(|profile| {
        (
            profile.folder.clone().unwrap_or_default().to_lowercase(),
            profile.name.to_lowercase(),
        )
    });
    let mut previous = None;
    profiles
        .into_iter()
        .map(|profile| {
            let folder = profile.folder.clone().unwrap_or_else(|| "Ungrouped".into());
            let heading = if previous.as_ref() == Some(&folder) {
                None
            } else {
                previous = Some(folder.clone());
                Some(folder)
            };
            (heading, profile)
        })
        .collect()
}

fn connection_group_row(folder: Option<String>, row: AnyElement) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_none()
        .when_some(folder, |view, folder| {
            view.child(
                div()
                    .px(crate::theme::ui_px(10.))
                    .pt(crate::theme::ui_px(10.))
                    .pb(crate::theme::ui_px(5.))
                    .text_size(crate::theme::text_px(11.))
                    .text_color(theme().text_muted)
                    .child(folder),
            )
        })
        .child(row)
}
