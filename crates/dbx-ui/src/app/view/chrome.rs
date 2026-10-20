use super::super::*;
use super::PaneResize;
use crate::app::row_count::{RowCount, approximate, grouped};
use crate::popups::DropdownMenu as _;
use gpui_component::menu::PopupMenuItem;

impl DbxApp {
    pub(super) fn render_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sidebar_visible = !self.sidebar_hidden && !self.inspector_page();
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .pr(crate::theme::ui_px(GLASS_INSET))
            .pb(crate::theme::ui_px(GLASS_INSET))
            .gap(crate::theme::ui_px(GLASS_INSET))
            .when(sidebar_visible, |view| {
                view.child(
                    div()
                        .relative()
                        .flex_none()
                        .flex()
                        .child(self.render_sidebar(window, cx))
                        .child(self.pane_resize_handle(PaneResize::Explorer, cx))
                        .on_drag_move(cx.listener(|this, event, _, cx| {
                            this.resize_pane(PaneResize::Explorer, event, cx)
                        })),
                )
            })
            .child(
                // The content sheet: the one opaque surface, so data never
                // competes with whatever is behind the window.
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(crate::theme::ui_px(RADIUS_PANEL))
                    .border_1()
                    .border_color(theme().hairline)
                    .bg(theme().canvas)
                    .shadow(glass_shadow(10.))
                    .child(self.render_main(window, cx))
                    .child(self.render_status(cx)),
            )
    }

    pub(super) fn render_app_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_pane = self.active_session().map(|session| session.pane);
        let busy = self.active_session().is_some_and(|session| {
            session.busy || session.active_data_tab().is_some_and(|data| data.busy)
        });
        div()
            .id("workspace-navigation")
            .role(gpui::Role::Navigation)
            .aria_label("Workspace views")
            .w(crate::theme::ui_px(48.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .pb(crate::theme::ui_px(GLASS_INSET))
            .child(
                div()
                    .flex_1()
                    .pt(crate::theme::ui_px(2.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(crate::theme::ui_px(6.))
                    .child(self.rail_button(
                        "rail-data",
                        Icon::Table,
                        "Data",
                        active_pane == Some(Pane::Data),
                        cx.listener(|this, _, window, cx| {
                            this.set_active_pane(Pane::Data, window, cx)
                        }),
                    ))
                    .child(self.rail_button(
                        "rail-structure",
                        Icon::Structure,
                        "Structure",
                        active_pane == Some(Pane::Structure),
                        cx.listener(|this, _, window, cx| {
                            this.set_active_pane(Pane::Structure, window, cx)
                        }),
                    ))
                    .child(self.rail_button(
                        "rail-query",
                        Icon::Query,
                        "New query",
                        active_pane == Some(Pane::Query),
                        cx.listener(|this, _, window, cx| {
                            if let Some(session_id) = this.active_session_id() {
                                this.add_query_tab_for(session_id, window, cx);
                            }
                        }),
                    )),
            )
            .when(busy, |rail| {
                rail.child(
                    div()
                        .id("rail-connection-health")
                        .size(crate::theme::ui_px(8.))
                        .rounded_full()
                        .bg(theme().warning)
                        .tooltip(tip("Working…")),
                )
            })
    }

    pub(super) fn set_active_pane(
        &mut self,
        pane: Pane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connection_picker_open = false;
        let Some(session_id) = self.active_session_id else {
            return;
        };
        match pane {
            Pane::Data => {
                let Some(session) = self.session(session_id) else {
                    return;
                };
                if session.active_data_tab_id().is_some() {
                    return;
                }
                // Return to the table viewed last, else the first open one.
                let tab_id = session
                    .recent_data_tab
                    .filter(|id| session.data_tab(*id).is_some())
                    .or_else(|| {
                        session.secondary_tabs.iter().find_map(|tab| {
                            matches!(tab.kind, SecondaryTabKind::Data(_)).then_some(tab.id)
                        })
                    });
                match tab_id {
                    Some(tab_id) => self.activate_secondary_tab_for(session_id, tab_id, window, cx),
                    None => self.show_toast(ToastKind::Info, "Select a table to browse rows", cx),
                }
            }
            Pane::Query | Pane::Diagram => {}
            Pane::Structure => {
                let table = self.session(session_id).and_then(|session| {
                    let selected = &session.recent_data()?.table;
                    session
                        .tables
                        .iter()
                        .find(|table| table_ref(table) == *selected)
                        .cloned()
                });
                if let Some(table) = table {
                    self.open_structure_tab_for(session_id, table, cx);
                    return;
                }
                self.show_toast(ToastKind::Info, "Select a table to view its structure", cx);
            }
        }
    }

    /// Arm the app-owned titlebar drag on any element: double-click zooms,
    /// right-click opens the compositor's window menu where one exists.
    fn titlebar_drag<E>(&self, element: E, cx: &mut Context<Self>) -> E
    where
        E: InteractiveElement + StatefulInteractiveElement + gpui_component::InteractiveElementExt,
    {
        element
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.window_drag_armed = false;
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.window_drag_armed = false;
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.window_drag_armed = true;
                }),
            )
            .on_mouse_down(MouseButton::Right, |event, window, _| {
                if window.window_controls().window_menu {
                    window.show_window_menu(event.position);
                }
            })
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.window_drag_armed {
                    this.window_drag_armed = false;
                    window.start_window_move();
                }
            }))
            .on_double_click(|_, window, _| window.zoom_window())
    }

    pub(super) fn render_topbar(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let has_sessions = !self.sessions.is_empty();
        let unlocked = self.vault_state == Some(VaultState::Unlocked);
        let connected = unlocked
            && self
                .active_session()
                .is_some_and(|session| session.engine.is_some());
        let show_sidebar_toggle = connected && !self.connection_picker_open;
        let identity = self
            .titlebar_drag(
                div()
                    .id("window-title-drag")
                    .h_full()
                    .flex_none()
                    .pl(px(if cfg!(target_os = "macos") { 80. } else { 14. }))
                    .pr(crate::theme::ui_px(6.))
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(8.)),
                cx,
            )
            .when(!has_sessions, |view| {
                view.child(
                    img(self.logo.clone())
                        .id("topbar-logo")
                        .size(crate::theme::ui_px(18.)),
                )
                .child(
                    div()
                        .text_size(crate::theme::text_px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("DBX"),
                )
            })
            .when(show_sidebar_toggle, |view| {
                view.child(
                    glass_icon_button("toggle-sidebar", Icon::Sidebar, false)
                        .aria_label(if self.inspector_page() {
                            "Back to table"
                        } else {
                            "Toggle explorer"
                        })
                        .tooltip(tip(if self.inspector_page() {
                            "Back to table"
                        } else if self.sidebar_hidden {
                            "Show explorer"
                        } else {
                            "Hide explorer"
                        }))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_sidebar_action(&ToggleSidebar, window, cx)
                        })),
                )
            });
        div()
            .h(crate::theme::control_height(46.))
            .flex_none()
            .flex()
            .items_center()
            .child(identity)
            .when(unlocked, |view| view.child(self.render_connection_tabs(cx)))
            .child(
                self.titlebar_drag(
                    div()
                        .id("window-title-drag-spacer")
                        .flex_1()
                        .min_w(px(if self.compact_layout { 24. } else { 48. }))
                        .h_full(),
                    cx,
                ),
            )
            .child(
                div()
                    .flex_none()
                    .pr(crate::theme::ui_px(10.))
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(4.))
                    .when(self.vault_state == Some(VaultState::Unlocked), |view| {
                        view.child(
                            glass_icon_button("lock-vault", Icon::Lock, false)
                                .debug_selector(|| "lock-vault".into())
                                .tooltip(tip("Lock vault"))
                                .when(!self.vault_busy && !self.saving_connection, |button| {
                                    button.on_click(cx.listener(|this, _, window, cx| {
                                        this.request_lock_vault(window, cx);
                                    }))
                                }),
                        )
                    })
                    .child(
                        glass_icon_button("open-settings", Icon::Settings, self.settings_open)
                            .tooltip(tip("Settings"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.settings_open {
                                    this.close_settings(window, cx);
                                } else {
                                    this.open_settings(window, cx);
                                }
                            })),
                    )
                    .when(!cfg!(target_os = "macos"), |view| {
                        view.child(self.render_window_controls(window))
                    }),
            )
    }

    /// Minimize / maximize / close for platforms where DBX draws its own
    /// titlebar, showing only the controls the compositor supports.
    fn render_window_controls(&self, window: &Window) -> impl IntoElement {
        let controls = window.window_controls();
        let maximized = window.is_maximized();
        div()
            .flex()
            .items_center()
            .gap(crate::theme::ui_px(6.))
            .ml(crate::theme::ui_px(4.))
            .when(controls.minimize, |view| {
                view.child(
                    window_control_button("window-minimize", Icon::Minimize, false)
                        .tooltip(tip("Minimize"))
                        .on_click(|_, window, _| window.minimize_window()),
                )
            })
            .when(controls.maximize, |view| {
                view.child(
                    window_control_button(
                        "window-maximize",
                        if maximized {
                            Icon::Restore
                        } else {
                            Icon::Maximize
                        },
                        false,
                    )
                    .tooltip(tip(if maximized { "Restore" } else { "Maximize" }))
                    .on_click(|_, window, _| window.zoom_window()),
                )
            })
            .child(
                window_control_button("window-close", Icon::Close, true)
                    .tooltip(tip("Close"))
                    .on_click(|_, window, _| window.remove_window()),
            )
    }

    fn render_connection_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_session_id = self
            .active_session_id()
            .filter(|_| !self.connection_picker_open);
        let sessions: Vec<_> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(index, session)| {
                (
                    session.id,
                    format!(
                        "{}{}",
                        if session.name.trim().is_empty() {
                            format!("{} {}", session.kind, index + 1)
                        } else {
                            session.name.clone()
                        },
                        if session
                            .engine
                            .as_ref()
                            .is_some_and(|engine| engine.is_read_only())
                        {
                            " · protected"
                        } else {
                            ""
                        }
                    ),
                    session.busy,
                    session.kind,
                    session.profile_id.is_some(),
                    session.tag.clone(),
                )
            })
            .collect();
        let has_sessions = !sessions.is_empty();
        div()
            .id("connection-tabs-scroll")
            .role(gpui::Role::TabList)
            .aria_label("Connections")
            .flex_shrink(1.)
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(crate::theme::ui_px(4.))
            .overflow_x_scroll()
            .children(
                sessions
                    .into_iter()
                    .map(|(session_id, label, busy, kind, _saved, tag)| {
                        let selected = active_session_id == Some(session_id);
                        connection_tab(kind, label.clone(), selected)
                            .debug_selector(|| "connection-tab".into())
                            .id(SharedString::from(format!("connection-tab-{session_id}")))
                            .pressable(label.clone())
                            .role(gpui::Role::Tab)
                            .aria_selected(selected)
                            .flex_none()
                            .cursor_pointer()
                            .when(busy, |tab| {
                                tab.child(
                                    div()
                                        .size(crate::theme::ui_px(6.))
                                        .rounded_full()
                                        .bg(theme().warning),
                                )
                            })
                            .children(tag_badge(tag.as_ref()))
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "close-connection-tab-{session_id}"
                                    )))
                                    .debug_selector(move || {
                                        format!("close-connection-tab-{session_id}")
                                    })
                                    .pressable(format!("Close connection {label}"))
                                    .size(crate::theme::ui_px(24.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(theme().glass_hover))
                                    .tooltip(tip("Close connection"))
                                    .child(
                                        icon(Icon::Close, theme().text_muted)
                                            .size(crate::theme::ui_px(12.)),
                                    )
                                    .on_click(cx.listener(
                                        move |this, event: &gpui::ClickEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.close_session(session_id, cx);
                                            if event.is_keyboard()
                                                && this.session(session_id).is_none()
                                            {
                                                // Walk the still-painted tab order before the removed
                                                // close control disappears and focus falls to the root.
                                                window.focus_next(cx);
                                            }
                                        },
                                    )),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_session(session_id, cx)
                            }))
                    }),
            )
            // The connection list already offers "new" while it is showing.
            .when(has_sessions && !self.connection_picker_open, |view| {
                view.child(
                    glass_icon_button("add-connection-tab", Icon::Add, false)
                        .aria_label("New connection")
                        .tooltip(tip("New connection"))
                        .on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx))),
                )
            })
    }

    fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some(search) = self
            .session(session_id)
            .map(|session| session.editors.sidebar_search.read(cx).clone())
        else {
            return div().into_any_element();
        };
        // Rebuild the row list only when its inputs changed, then keep the
        // dropdowns in step with the session.
        let Some(session) = self.session_mut(session_id) else {
            return div().into_any_element();
        };
        let changed = session.sidebar.list.refresh(
            session.kind,
            &session.tables,
            session.tables_revision,
            session.schema_filter.as_deref(),
            &search,
        );
        if changed {
            let visible = session
                .sidebar
                .list
                .visible
                .iter()
                .map(table_sidebar_id)
                .collect::<BTreeSet<_>>();
            session.sidebar.selected.retain(|id| visible.contains(id));
        }
        session.sidebar.sync_selectors(
            session.kind,
            &session.databases,
            session.current_database.as_deref(),
            session.schema_filter.as_deref(),
            window,
            cx,
        );
        let Some((
            kind,
            visible_tables,
            show_database_select,
            show_schema_select,
            database_select,
            schema_select,
            search_editor,
            selected_schema,
            selected_tables,
        )) = self.session(session_id).map(|session| {
            (
                session.kind,
                session.sidebar.list.visible.clone(),
                session.databases.len() > 1,
                session.kind.dialect() == DatabaseKind::PostgreSQL
                    && session.sidebar.list.schema_options.len() > 2,
                session.sidebar.database_select.clone(),
                session.sidebar.schema_select.clone(),
                session.editors.sidebar_search_editor.clone(),
                session.schema_filter.clone(),
                session.sidebar.selected.clone(),
            )
        })
        else {
            return div().into_any_element();
        };
        let search_focus = search_editor.read(cx).focus_handle();
        let explorer_actions = cx.entity().downgrade();
        let (explorer_focus, explorer_scroll, cursor_bounds, list_bounds) = self
            .session(session_id)
            .map(|session| {
                (
                    session.sidebar.focus.clone(),
                    session.sidebar.scroll.clone(),
                    session.sidebar.cursor_bounds.clone(),
                    session.sidebar.list_bounds.clone(),
                )
            })
            .expect("session checked above");
        let explorer_focused = explorer_focus.is_focused(window);
        let cursor_id = explorer_focused
            .then(|| self.explorer_cursor_id(session_id))
            .flatten();
        let table_count = visible_tables.len();
        let native_backup_available = self
            .session(session_id)
            .and_then(|session| session.engine.as_ref())
            .is_some_and(|engine| engine.capabilities().native_backup);
        let native_restore_available = native_backup_available
            && self
                .session(session_id)
                .and_then(|session| session.engine.as_ref())
                .is_some_and(|engine| !engine.is_read_only());
        let import_supported = self
            .session(session_id)
            .and_then(|session| session.engine.as_ref())
            .is_some_and(|engine| matches!(engine.as_ref(), DatabaseEngine::Sql(_)));
        let import_available = self.session(session_id).is_some_and(|session| {
            !session.busy
                && session
                    .engine
                    .as_ref()
                    .is_some_and(|engine| !engine.is_read_only())
        });
        let empty_note = visible_tables.is_empty().then(|| {
            let session = self.session(session_id);
            if session.is_some_and(|session| session.busy) {
                "Loading…"
            } else if !search.trim().is_empty() {
                "No matches"
            } else if session.is_some_and(|session| session.error.is_some()) {
                "Couldn’t load the list"
            } else {
                match kind {
                    DatabaseKind::Redis => "No keys",
                    DatabaseKind::Kafka => "No topics",
                    DatabaseKind::MongoDB => "No collections",
                    DatabaseKind::Elasticsearch => "No indices",
                    _ => "No tables",
                }
            }
        });
        glass(div(), RADIUS_GLASS, 8.).id(SharedString::from(format!("database-explorer-{session_id}"))).role(gpui::Role::Region).aria_label("Database explorer")
            .w(px(self.explorer_width()))
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .px(crate::theme::ui_px(10.))
                    .pt(crate::theme::ui_px(8.))
                    .pb(crate::theme::ui_px(8.))
                    .flex()
                    .flex_col()
                    .gap(crate::theme::ui_px(7.))
                    .border_b_1()
                    .border_color(theme().hairline)
                    .child(
                        div()
                            .id("explorer-header")
                            .debug_selector(|| "explorer-header".into())
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .gap(crate::theme::ui_px(6.))
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex_none().min_w_0().max_w_full()
                                    .flex()
                                    .items_center()
                                    .gap(crate::theme::ui_px(7.))
                                    .child(icon(Icon::Database, theme().text_muted))
                                    .child(
                                        div().id("explorer-title").debug_selector(|| "explorer-title".into()).min_w_0().truncate()
                                            .text_size(crate::theme::text_px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme().text)
                                            .child(if kind == DatabaseKind::Redis {
                                                "Keyspace"
                                            } else {
                                                "Explorer"
                                            }),
                                    )
                                    .when(!self.compact_layout, |view| {
                                        view.child(badge(
                                            format!("{table_count}"),
                                            theme().text_muted,
                                        ))
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .max_w_full()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .child(
                                        Button::new("refresh-tables")
                                            .with_size(Size::XSmall)
                                            .compact()
                                            .ghost()
                                            .accessible_name("Refresh explorer").tooltip("Refresh explorer")
                                            .child(icon(Icon::Refresh, theme().text_muted))
                                            .on_click(cx.listener(move |this, _, _window, cx| {
                                                this.refresh_tables_for(session_id, cx)
                                            })),
                                    )
                                    .when(kind.is_sql(), |view| {
                                        let open_diagram = explorer_actions.clone();
                                        let export_database = explorer_actions.clone();
                                        let import_database = explorer_actions.clone();
                                        let mcp_active = self.session(session_id).is_some_and(|session| session.mcp_pairing.is_some());
                                        view.child(
                                            Button::new("create-table")
                                                .with_size(Size::XSmall)
                                                .compact()
                                                .ghost()
                                                .accessible_name("New table").tooltip("New table")
                                                .child(icon(Icon::Add, theme().text_muted))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.create_table_template_for(
                                                            session_id, window, cx,
                                                        )
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new("explorer-more")
                                                .with_size(Size::XSmall)
                                                .compact()
                                                .ghost()
                                                .debug_selector(|| "explorer-more".into())
                                                .accessible_name("Explorer actions").tooltip("Explorer actions")
                                                .child(icon(Icon::More, theme().text_muted))
                                                .dropdown_menu(move |menu, _, _| {
                                                    let backup = export_database.clone();
                                                    let restore = export_database.clone();
                                                    let log = export_database.clone();
                                                    let open_diagram = open_diagram.clone();
                                                    let mcp_toggle = export_database.clone();
                                                    let mcp_copy = export_database.clone();
                                                    let mcp_log = export_database.clone();
                                                    let export_database = export_database.clone();
                                                    let import_database = import_database.clone();
                                                    menu.item(
                                                        PopupMenuItem::new("Open database diagram")
                                                            .on_click(move |_, window, cx| {
                                                                let _ = open_diagram.update(
                                                                    cx,
                                                                    |this, cx| {
                                                                        this.open_diagram_for(
                                                                            session_id, window, cx,
                                                                        );
                                                                    },
                                                                );
                                                            }),
                                                    )
                                                    .when(native_backup_available, |menu| menu.item(
                                                        PopupMenuItem::new("Native backup…")
                                                            .on_click(move |_, _, cx| {
                                                                let _ = backup.update(
                                                                    cx,
                                                                    |this, cx| {
                                                                        this.choose_native_backup(
                                                                            session_id, false, cx,
                                                                        )
                                                                    },
                                                                );
                                                            }),
                                                    )
                                                    .item(
                                                        PopupMenuItem::new(
                                                            "Restore native backup…",
                                                        ).disabled(!native_restore_available)
                                                        .on_click(move |_, _, cx| {
                                                            let _ =
                                                                restore.update(cx, |this, cx| {
                                                                    this.choose_native_backup(
                                                                        session_id, true, cx,
                                                                    )
                                                                });
                                                        }),
                                                    )
                                                    .item(
                                                        PopupMenuItem::new("View native job log")
                                                            .on_click(move |_, window, cx| {
                                                                let _ =
                                                                    log.update(cx, |this, cx| {
                                                                        this.show_transfer_log(
                                                                            session_id, window, cx,
                                                                        )
                                                                    });
                                                            }),
                                                    ))
                                                    .item(PopupMenuItem::new(if mcp_active { "Stop MCP sharing" } else { "Share this database read-only via MCP" }).on_click(move |_, _, cx| { let _ = mcp_toggle.update(cx, |this, cx| this.toggle_mcp(session_id, cx)); }))
                                                    .item(PopupMenuItem::new("Copy private MCP pairing configuration").disabled(!mcp_active).on_click(move |_, _, cx| { let _ = mcp_copy.update(cx, |this, cx| this.copy_mcp_pairing(session_id, cx)); }))
                                                    .item(PopupMenuItem::new("View MCP activity").disabled(!mcp_active).on_click(move |_, window, cx| { let _ = mcp_log.update(cx, |this, cx| this.show_mcp_activity(session_id, window, cx)); }))
                                                    .separator()
                                                    .item(
                                                        PopupMenuItem::new("Export database…")
                                                            .on_click(move |_, window, cx| {
                                                                let _ = export_database.update(
                                                                    cx,
                                                                    |this, cx| {
                                                                        this.begin_database_export(
                                                                            session_id, window, cx,
                                                                        );
                                                                    },
                                                                );
                                                            }),
                                                    )
                                                    .when(import_supported, |menu| menu.item(
                                                        PopupMenuItem::new("Import database…")
                                                            .disabled(!import_available)
                                                            .on_click(move |_, window, cx| {
                                                                let _ = import_database.update(
                                                                    cx,
                                                                    |this, cx| {
                                                                        this.begin_database_import(
                                                                            session_id, window, cx,
                                                                        );
                                                                    },
                                                                );
                                                            }),
                                                    ))
                                                }),
                                        )
                                    }),
                            ),
                    )
                    .when(show_database_select, |view| {
                        view.child(
                            div()
                                .id("database-select").role(gpui::Role::Group).aria_label("Database")
                                .child(sidebar_select(&database_select, "Database")),
                        )
                    })
                    .when(show_schema_select, |view| {
                        view.child(
                            div()
                                .id("schema-select").role(gpui::Role::Group).aria_label("Schema filter")
                                .child(sidebar_select(&schema_select, "Schema filter")),
                        )
                    })
                    .when(kind != DatabaseKind::Redis, |view| {
                        view.child(
                            div()
                                .h(crate::theme::control_height(SIDEBAR_CONTROL_HEIGHT))
                                .px(crate::theme::ui_px(8.))
                                .flex()
                                .items_center()
                                .gap(crate::theme::ui_px(6.))
                                .rounded(crate::theme::ui_px(RADIUS_CONTROL))
                                .bg(theme().glass_hover)
                                .border_1()
                                .border_color(theme().hairline)
                                .capture_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if event.keystroke.key == "down"
                                            && !event.keystroke.modifiers.modified()
                                        {
                                            this.focus_explorer_list(window, cx);
                                            cx.stop_propagation();
                                        }
                                    },
                                ))
                                .child(icon(Icon::Search, theme().text_muted))
                                .child(div().flex_1().min_w_0().child(editor::bare_input(
                                    search_editor,
                                    search_focus,
                                    px(SIDEBAR_CONTROL_HEIGHT - 2.),
                                 cx))),
                        )
                    }),
            )
            .child(
                div()
                    .id("explorer-list")
                    .role(gpui::Role::ListBox).aria_label("Database objects")
                    .a11y_synthetic_children(|builder| builder.parent_node().set_multiselectable())
                    .aria_description(format!("{table_count} objects. Control or Command click toggles selection. Right-click opens actions for selected objects. Arrow keys move the cursor, Enter opens the object, and the context menu key opens object actions."))
                    .on_a11y_action(gpui::AccessibleAction::ScrollDown, { let app = cx.entity(); move |_, window, cx| app.update(cx, |this, cx| this.scroll_explorer_accessibly(session_id, 20, window, cx)) })
                    .on_a11y_action(gpui::AccessibleAction::ScrollUp, { let app = cx.entity(); move |_, window, cx| app.update(cx, |this, cx| this.scroll_explorer_accessibly(session_id, -20, window, cx)) })
                    .key_context("DbxExplorer")
                    .track_focus(&explorer_focus)
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .py(crate::theme::ui_px(6.))
                    .child(
                        gpui::canvas(
                            move |bounds, _, _| list_bounds.set(Some(bounds)),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .child(
                    if let Some(note) = empty_note {
                        crate::accessibility::announcement("explorer-empty", note, false)
                            .px(crate::theme::ui_px(12.))
                            .py(crate::theme::ui_px(8.))
                            .text_size(crate::theme::text_px(11.))
                            .text_color(theme().text_muted)
                            .child(note)
                            .into_any_element()
                    } else {
                    uniform_list(
                        "sidebar-tables",
                        visible_tables.len(),
                        cx.processor(move |_this, range: Range<usize>, _window, cx| {
                            range
                                .filter_map(|index| visible_tables.get(index))
                                .map(|table| {
                                    sidebar_row(
                                        session_id,
                                        table,
                                        &selected_tables,
                                        selected_schema.as_deref(),
                                        cursor_id.as_deref(),
                                        &cursor_bounds,
                                        cx,
                                    )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&explorer_scroll)
                    .h_full()
                    .into_any_element()
                    },
                ),
            )
            .child(self.render_schema_objects_for(session_id, None, cx))
            .into_any_element()
    }

    fn render_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = self
            .active_session()
            .map(|session| session.pane)
            .unwrap_or(Pane::Data);
        // min_h_0 lets tall panes (a long row draft) scroll inside the sheet
        // instead of pushing the inspector actions and footer out of view.
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.render_tabs(cx))
            .child(match pane {
                Pane::Data => self.render_data(window, cx).into_any_element(),
                Pane::Structure => self.render_structure(window, cx).into_any_element(),
                Pane::Query => self.render_query(window, cx).into_any_element(),
                Pane::Diagram => self.render_diagram(window, cx).into_any_element(),
            })
    }

    fn render_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let session_id = self.active_session_id();
        let (active_secondary_tab, tabs) = self
            .active_session()
            .map(|session| {
                let mut query_number = 0;
                let tabs = session
                    .secondary_tabs
                    .iter()
                    .map(|tab| {
                        let (label, kind) = match &tab.kind {
                            SecondaryTabKind::Data(data) => (data.table.name.clone(), Icon::Table),
                            SecondaryTabKind::Query(query) => {
                                query_number += 1;
                                let name = query.name.read(cx);
                                (
                                    if name.as_str() == "Untitled" {
                                        format!("Query {query_number}")
                                    } else {
                                        name.clone()
                                    },
                                    Icon::Query,
                                )
                            }
                            SecondaryTabKind::Structure(structure) => (
                                format!("{} structure", structure.table.name),
                                Icon::Structure,
                            ),
                            SecondaryTabKind::Diagram(_) => {
                                ("Database diagram".into(), Icon::Diagram)
                            }
                        };
                        (tab.id, label, kind)
                    })
                    .collect::<Vec<_>>();
                (session.active_secondary_tab, tabs)
            })
            .unwrap_or_default();
        div()
            .id("document-tabs")
            .role(gpui::Role::TabList)
            .aria_label("Documents")
            .h(crate::theme::control_height(40.))
            .flex_none()
            .px(crate::theme::ui_px(8.))
            .flex()
            .min_w_0()
            .items_center()
            .gap(crate::theme::ui_px(4.))
            .overflow_x_scroll()
            .border_b_1()
            .border_color(theme().border)
            .follow_top_corners(RADIUS_PANEL)
            .bg(theme().panel)
            .children(tabs.into_iter().map(|(tab_id, label, kind)| {
                let selected = active_secondary_tab == Some(tab_id);
                document_tab(
                    SharedString::from(format!("document-{tab_id}")),
                    kind,
                    selected,
                    label.clone(),
                )
                .child(label.clone())
                .child(
                    div()
                        .id(SharedString::from(format!("close-document-{tab_id}")))
                        .debug_selector(move || format!("close-document-{tab_id}"))
                        .pressable(format!("Close document {label}"))
                        .size(crate::theme::ui_px(24.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|style| style.bg(theme().glass_hover))
                        .tooltip(tip("Close tab"))
                        .child(icon(Icon::Close, theme().text_muted).size(crate::theme::ui_px(12.)))
                        .on_click(cx.listener(
                            move |this, event: &gpui::ClickEvent, window, cx| {
                                cx.stop_propagation();
                                if let Some(session_id) = session_id {
                                    this.request_close_secondary_tab_for(
                                        session_id, tab_id, window, cx,
                                    );
                                    if event.is_keyboard()
                                        && this.session(session_id).is_some_and(|session| {
                                            !session
                                                .secondary_tabs
                                                .iter()
                                                .any(|tab| tab.id == tab_id)
                                        })
                                    {
                                        window.focus_next(cx);
                                    }
                                }
                            },
                        )),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(session_id) = session_id {
                        this.activate_secondary_tab_for(session_id, tab_id, window, cx);
                    }
                }))
            }))
            .child(
                glass_icon_button("add-query-document", Icon::Add, false)
                    .aria_label("New query document")
                    .tooltip(tip("New query"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(session_id) = session_id {
                            this.add_query_tab_for(session_id, window, cx);
                        }
                    })),
            )
    }

    /// The footer carries errors, in-progress work, and the result extent.
    /// Idle narration ("Ready", "Inspecting selected row") is left out.
    fn render_status(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (error, status, result, table_pagination, summary) = self
            .active_session()
            .map(|session| {
                if let Some(tab) = session
                    .active_secondary_tab
                    .and_then(|tab_id| session.secondary_tabs.iter().find(|tab| tab.id == tab_id))
                {
                    match &tab.kind {
                        SecondaryTabKind::Data(data) => {
                            let table_pagination = (session.kind != DatabaseKind::Redis
                                && data.result.is_some())
                            .then_some((
                                session.id,
                                tab.id,
                                data.table_page,
                                data.table_has_next_page,
                                data.busy,
                                session.kind.is_sql().then_some(data.row_count),
                            ));
                            let status = if data.busy {
                                data.status.clone()
                            } else if session.busy {
                                session.status.clone()
                            } else {
                                String::new()
                            };
                            return (
                                data.error.clone().or_else(|| session.error.clone()),
                                status,
                                data.result.clone(),
                                table_pagination,
                                String::new(),
                            );
                        }
                        // Query tabs show their outcome and errors inline.
                        SecondaryTabKind::Query(_) => {
                            return (
                                session.error.clone(),
                                if session.busy {
                                    session.status.clone()
                                } else {
                                    String::new()
                                },
                                None,
                                None,
                                String::new(),
                            );
                        }
                        SecondaryTabKind::Diagram(diagram) => {
                            let status = match (diagram.busy, &diagram.document) {
                                (true, Some(_)) => "Refreshing database diagram…".into(),
                                (true, None) => "Building database diagram…".into(),
                                (false, _) => String::new(),
                            };
                            let summary = diagram
                                .document
                                .as_ref()
                                .map(|document| {
                                    format!(
                                        "{} · {}",
                                        counted(document.nodes.len() as u64, "table", "tables"),
                                        counted(
                                            document.edges.len() as u64,
                                            "relationship",
                                            "relationships"
                                        )
                                    )
                                })
                                .unwrap_or_default();
                            return (diagram.error.clone(), status, None, None, summary);
                        }
                        SecondaryTabKind::Structure(structure) => {
                            let summary = if structure.busy {
                                String::new()
                            } else {
                                format!(
                                    "{} · {}",
                                    counted(structure.columns.len() as u64, "column", "columns"),
                                    counted(
                                        structure.foreign_keys.len() as u64,
                                        "foreign key",
                                        "foreign keys"
                                    )
                                )
                            };
                            let status = if structure.busy {
                                "Loading structure…".to_owned()
                            } else {
                                String::new()
                            };
                            return (structure.error.clone(), status, None, None, summary);
                        }
                    }
                }
                (
                    session.error.clone(),
                    if session.busy {
                        session.status.clone()
                    } else {
                        String::new()
                    },
                    None,
                    None,
                    String::new(),
                )
            })
            .unwrap_or_else(|| (self.error.clone(), String::new(), None, None, String::new()));
        let result_summary = result
            .as_ref()
            .map(|result| {
                if let Some((_, _, page, _, _, count)) = table_pagination {
                    let page_number = page.saturating_add(1);
                    let exact = match count {
                        Some(RowCount::Exact(total)) => Some(total),
                        _ => None,
                    };
                    let pages = exact
                        .map(|total| {
                            format!(
                                " of {}",
                                total.div_ceil(u64::from(TABLE_BROWSE_PAGE_SIZE)).max(1)
                            )
                        })
                        .unwrap_or_default();
                    if result.rows.is_empty() {
                        format!("No rows · page {page_number}{pages}")
                    } else {
                        let first_row = page
                            .saturating_mul(u64::from(TABLE_BROWSE_PAGE_SIZE))
                            .saturating_add(1);
                        let last_row = first_row + result.rows.len() as u64 - 1;
                        let total = exact
                            .map(|total| format!(" of {}", grouped(total)))
                            .unwrap_or_default();
                        format!(
                            "Rows {}–{}{total} · page {page_number}{pages}",
                            grouped(first_row),
                            grouped(last_row)
                        )
                    }
                } else {
                    format!("{} rows", result.rows.len())
                }
            })
            .unwrap_or(summary);
        let pagination_controls =
            table_pagination.map(|(session_id, tab_id, page, has_next_page, busy, count)| {
                let count_button = match count {
                    Some(RowCount::Unknown) => Some(("Count rows".to_owned(), true)),
                    Some(RowCount::Estimate(estimate)) => {
                        Some((format!("{} rows", approximate(estimate)), true))
                    }
                    Some(RowCount::Counting) => Some(("Counting…".to_owned(), false)),
                    Some(RowCount::Exact(_)) | None => None,
                };
                let estimated = matches!(count, Some(RowCount::Estimate(_)));
                div()
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(4.))
                    .when_some(count_button, |view, (label, enabled)| {
                        view.child(
                            div()
                                .id("table-row-count")
                                .tooltip(tip(if estimated {
                                    "Estimated from table statistics. Click for an exact count."
                                } else {
                                    "Count every matching row"
                                }))
                                .child(self.small_button_state(
                                    "table-row-count-button",
                                    label,
                                    enabled && !busy,
                                    cx.listener(move |this, _, _, cx| {
                                        this.count_rows_for(session_id, tab_id, cx)
                                    }),
                                )),
                        )
                    })
                    .child(self.small_button_state(
                        "table-page-previous",
                        "Previous",
                        !busy && page > 0,
                        cx.listener(move |this, _, _, cx| {
                            this.set_table_page(session_id, tab_id, page.saturating_sub(1), cx)
                        }),
                    ))
                    .child(self.small_button_state(
                        "table-page-next",
                        "Next",
                        !busy && has_next_page,
                        cx.listener(move |this, _, _, cx| {
                            this.set_table_page(session_id, tab_id, page.saturating_add(1), cx)
                        }),
                    ))
            });
        let has_error = error.is_some();
        let message = error.unwrap_or(status);
        div()
            .h(crate::theme::control_height(30.))
            .flex_none()
            .px(crate::theme::ui_px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(crate::theme::ui_px(12.))
            .border_t_1()
            .border_color(theme().border)
            .follow_bottom_corners(RADIUS_PANEL)
            .bg(theme().panel)
            .text_size(crate::theme::text_px(11.))
            .text_color(theme().text_muted)
            .child(
                crate::accessibility::announcement("workbench-status", message.clone(), has_error)
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .when(has_error, |view| view.text_color(theme().danger_text))
                    .child(message),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(8.))
                    .when_some(
                        self.active_session_id.filter(|id| {
                            self.session(*id)
                                .is_some_and(|session| session.transfer_control.is_some())
                        }),
                        |view, session_id| {
                            view.child(
                                Button::new("cancel-transfer")
                                    .label("Cancel transfer")
                                    .ghost()
                                    .with_size(Size::XSmall)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.cancel_transfer_for(session_id, cx)
                                    })),
                            )
                        },
                    )
                    .child(
                        crate::accessibility::announcement(
                            "result-summary",
                            result_summary.clone(),
                            false,
                        )
                        .child(result_summary),
                    )
                    .when_some(pagination_controls, |view, controls| view.child(controls)),
            )
    }

    pub(super) fn small_button_state(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        enabled: bool,
        listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        Button::new(id)
            .label(label)
            .with_size(Size::XSmall)
            .h(crate::theme::control_height(22.))
            .px(crate::theme::ui_px(10.))
            .rounded_full()
            .outline()
            .disabled(!enabled)
            .border_color(theme().hairline)
            .bg(theme().glass_hover)
            .text_color(if enabled {
                theme().text
            } else {
                theme().text_muted
            })
            .when(enabled, |view| view.cursor_pointer())
            .on_click(listener)
    }

    fn rail_button(
        &self,
        id: &'static str,
        kind: Icon,
        label: &'static str,
        selected: bool,
        listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        glass_icon_button(id, kind, selected)
            .size(crate::theme::ui_px(34.))
            .aria_label(label)
            .aria_selected(selected)
            .tooltip(tip(label))
            .on_click(listener)
    }
}

/// A pill-shaped document tab on the content sheet's tab strip.
fn document_tab(
    id: impl Into<ElementId>,
    kind: Icon,
    selected: bool,
    label: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .pressable(label)
        .role(gpui::Role::Tab)
        .aria_selected(selected)
        .h(crate::theme::control_height(28.))
        .pl(crate::theme::ui_px(10.))
        .pr(crate::theme::ui_px(4.))
        .flex_none()
        .flex()
        .items_center()
        .gap(crate::theme::ui_px(6.))
        .rounded_full()
        .text_size(crate::theme::text_px(12.))
        .cursor_pointer()
        .when(selected, |tab| {
            tab.bg(theme().glass_selected)
                .border_1()
                .border_color(theme().hairline)
                .shadow(glass_shadow(3.))
                .text_color(theme().text)
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!selected, |tab| {
            tab.text_color(theme().text_muted)
                .hover(|style| style.bg(theme().glass_hover).text_color(theme().text))
        })
        .child(icon(
            kind,
            if selected {
                theme().accent
            } else {
                theme().text_muted
            },
        ))
}

/// A circular titlebar control. Close gets a destructive hover.
fn window_control_button(id: &'static str, kind: Icon, destructive: bool) -> Stateful<Div> {
    div()
        .id(id)
        .pressable(kind.label())
        .size(crate::theme::ui_px(28.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme().glass_hover)
        .cursor_pointer()
        .hover(move |style| {
            if destructive {
                style.bg(theme().danger)
            } else {
                style.bg(theme().glass_selected)
            }
        })
        .child(icon(kind, theme().text).size(crate::theme::ui_px(12.)))
}

fn sidebar_select(state: &SidebarSelect, name: &'static str) -> impl IntoElement {
    Select::new(state)
        .with_size(Size::Small)
        .h(crate::theme::control_height(SIDEBAR_CONTROL_HEIGHT))
        .rounded(crate::theme::ui_px(RADIUS_CONTROL))
        .w_full()
        .menu_max_h(crate::theme::ui_px(260.))
        .text_size(crate::theme::text_px(12.))
        .bg(theme().glass_hover)
        .border_color(theme().hairline)
        .text_color(theme().text)
        .accessible_select(name, state)
}

fn sidebar_row(
    session_id: SessionId,
    table: &TableInfo,
    selected_tables: &BTreeSet<String>,
    selected_schema: Option<&str>,
    cursor_id: Option<&str>,
    cursor_bounds: &CursorBounds,
    cx: &mut Context<DbxApp>,
) -> Div {
    let row_id = table_sidebar_id(table);
    let at_cursor = cursor_id == Some(row_id.as_str());
    let cursor_bounds = cursor_bounds.clone();
    let selected = selected_tables.contains(&row_id);
    let label = table_sidebar_label(table, selected_schema);
    let menu_table = table.clone();
    let table_ref = table.clone();
    div().w_full().px(crate::theme::ui_px(6.)).child(
        div()
            .id(SharedString::from(row_id.clone()))
            .role(gpui::Role::ListBoxOption)
            .aria_label(table_sidebar_label(table, None))
            .aria_selected(selected)
            .when(at_cursor, |row| row.aria_active_descendant())
            .on_a11y_action(gpui::AccessibleAction::Focus, {
                let table = table.clone();
                let app = cx.entity();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.set_explorer_cursor(session_id, &table, cx);
                        if let Some(session) = this.session(session_id) {
                            session.sidebar.focus.focus(window, cx);
                        }
                        cx.notify();
                    })
                }
            })
            .relative()
            .w_full()
            .h(crate::theme::control_height(28.))
            .px(crate::theme::ui_px(8.))
            .rounded(crate::theme::ui_px(RADIUS_CONTROL))
            .when(selected, |row| {
                row.bg(theme().accent_soft).font_weight(FontWeight::MEDIUM)
            })
            .text_color(if selected {
                theme().accent_text
            } else {
                theme().text
            })
            .text_size(crate::theme::text_px(12.))
            .flex()
            .items_center()
            .gap(crate::theme::ui_px(7.))
            .cursor_pointer()
            .when(at_cursor, |row| {
                row.border_1().border_color(theme().focus_ring).child(
                    gpui::canvas(
                        move |bounds, _, _| cursor_bounds.set(Some((row_id, bounds))),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .when(!selected, |row| {
                row.hover(|style| style.bg(theme().glass_hover))
            })
            .child(icon(
                if table.kind == EntityKind::Table {
                    Icon::Table
                } else {
                    Icon::Search
                },
                if selected {
                    theme().accent
                } else {
                    theme().text_muted
                },
            ))
            .child(div().truncate().child(label))
            .on_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    let modifiers = event.modifiers();
                    this.click_explorer_table(
                        session_id,
                        table_ref.clone(),
                        modifiers.control || modifiers.platform,
                        window,
                        cx,
                    );
                }),
            )
            .on_aux_click(
                cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                    if table_click_action(event) == TableClickAction::OpenContextMenu {
                        // GPUI on macOS rewrites Control-click as a right-click
                        // and strips Control from both mouse events.
                        if event.modifiers().control || this.explorer_control_held {
                            this.click_explorer_table(
                                session_id,
                                menu_table.clone(),
                                true,
                                window,
                                cx,
                            );
                            return;
                        }
                        this.set_explorer_cursor(session_id, &menu_table, cx);
                        this.open_table_context_menu(
                            session_id,
                            menu_table.clone(),
                            event.position(),
                            window,
                            cx,
                        );
                    }
                }),
            ),
    )
}
