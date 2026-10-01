use super::super::*;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

impl DbxApp {
    pub(super) fn render_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let sidebar_visible = !self.sidebar_hidden;
        let workspace = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .pr(px(GLASS_INSET))
            .pb(px(GLASS_INSET))
            .gap(px(GLASS_INSET))
            .when(sidebar_visible, |view| {
                view.child(self.render_sidebar(window, cx))
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
                    .rounded(px(RADIUS_PANEL))
                    .border_1()
                    .border_color(theme().hairline)
                    .bg(theme().canvas)
                    .shadow(glass_shadow(10.))
                    .child(self.render_main(window, cx))
                    .child(self.render_status(cx)),
            );
        // Overlays live outside the gapped row so they never take up layout.
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .child(workspace)
            .child(self.render_table_context_menu(cx))
            .child(self.render_database_export_dialog(window, cx))
            .child(self.render_confirmation_dialog(cx))
            .child(self.render_mutation_error_dialog(cx))
    }

    pub(super) fn render_app_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_pane = self.active_session().map(|session| session.pane);
        let busy = self.active_session().is_some_and(|session| {
            session.busy || session.active_data_tab().is_some_and(|data| data.busy)
        });
        div()
            .w(px(48.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .pb(px(GLASS_INSET))
            .child(
                div()
                    .flex_1()
                    .pt(px(2.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
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
                        .size(px(8.))
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
        let connected = self
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
                    .pr(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.)),
                cx,
            )
            .when(!has_sessions, |view| {
                view.child(img(self.logo.clone()).id("topbar-logo").size(px(18.)))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("DBX"),
                    )
            })
            .when(show_sidebar_toggle, |view| {
                view.child(
                    glass_icon_button("toggle-sidebar", Icon::Sidebar, false)
                        .tooltip(tip(if self.sidebar_hidden {
                            "Show explorer"
                        } else {
                            "Hide explorer"
                        }))
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                )
            });
        div()
            .h(px(46.))
            .flex_none()
            .flex()
            .items_center()
            .child(identity)
            .child(self.render_connection_tabs(cx))
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
                    .pr(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .when(self.vault_state == Some(VaultState::Unlocked), |view| {
                        view.child(
                            glass_icon_button("lock-vault", Icon::Lock, false)
                                .tooltip(tip("Lock vault"))
                                .when(!self.vault_busy && !self.saving_connection, |button| {
                                    button.on_click(cx.listener(|this, _, window, cx| {
                                        this.lock_vault(cx);
                                        this.vault_editors
                                            .passphrase_editor
                                            .read(cx)
                                            .focus_handle()
                                            .focus(window, cx);
                                    }))
                                }),
                        )
                    })
                    .child(
                        glass_icon_button("open-settings", Icon::Settings, self.settings_open)
                            .tooltip(tip("Settings"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.settings_open {
                                    this.close_settings(cx);
                                } else {
                                    this.open_settings(cx);
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
            .gap(px(6.))
            .ml(px(4.))
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
                    if session.name.trim().is_empty() {
                        format!("{} {}", session.kind, index + 1)
                    } else {
                        session.name.clone()
                    },
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
            .flex_shrink(1.)
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(4.))
            .overflow_x_scroll()
            .children(
                sessions
                    .into_iter()
                    .map(|(session_id, label, busy, kind, _saved, tag)| {
                        let selected = active_session_id == Some(session_id);
                        connection_tab(kind, label, selected)
                            .id(SharedString::from(format!("connection-tab-{session_id}")))
                            .flex_none()
                            .cursor_pointer()
                            .when(busy, |tab| {
                                tab.child(div().size(px(6.)).rounded_full().bg(theme().warning))
                            })
                            .children(tag_badge(tag.as_ref()))
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "close-connection-tab-{session_id}"
                                    )))
                                    .size(px(18.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(theme().glass_hover))
                                    .tooltip(tip("Close connection"))
                                    .child(icon(Icon::Close, theme().text_muted).size(px(12.)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.close_session(session_id, cx)
                                    })),
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
        session.sidebar.list.refresh(
            session.kind,
            &session.tables,
            session.tables_revision,
            session.schema_filter.as_deref(),
            &search,
        );
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
            selected_table,
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
                session.active_data_tab().map(|data| data.table.clone()),
            )
        })
        else {
            return div().into_any_element();
        };
        let search_focus = search_editor.read(cx).focus_handle();
        let explorer_actions = cx.entity().downgrade();
        let table_count = visible_tables.len();
        glass(div(), RADIUS_GLASS, 8.)
            .w(if self.compact_layout {
                px(188.)
            } else {
                px(236.)
            })
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .px(px(10.))
                    .pt(px(8.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .border_b_1()
                    .border_color(theme().hairline)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .child(icon(Icon::Database, theme().text_muted))
                                    .child(
                                        div()
                                            .text_size(px(12.))
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
                                    .flex()
                                    .items_center()
                                    .child(
                                        Button::new("refresh-tables")
                                            .with_size(Size::XSmall)
                                            .compact()
                                            .ghost()
                                            .tooltip("Refresh explorer")
                                            .child(icon(Icon::Refresh, theme().text_muted))
                                            .on_click(cx.listener(move |this, _, _window, cx| {
                                                this.refresh_tables_for(session_id, cx)
                                            })),
                                    )
                                    .when(kind.is_sql(), |view| {
                                        let open_diagram = explorer_actions.clone();
                                        let export_database = explorer_actions.clone();
                                        let import_database = explorer_actions.clone();
                                        view.child(
                                            Button::new("create-table")
                                                .with_size(Size::XSmall)
                                                .compact()
                                                .ghost()
                                                .tooltip("New table")
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
                                                .tooltip("Explorer actions")
                                                .child(icon(Icon::More, theme().text_muted))
                                                .dropdown_menu(move |menu, _, _| {
                                                    let open_diagram = open_diagram.clone();
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
                                                    .item(
                                                        PopupMenuItem::new("Import database…")
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
                                                    )
                                                }),
                                        )
                                    }),
                            ),
                    )
                    .when(show_database_select, |view| {
                        view.child(
                            div()
                                .id("database-select")
                                .child(sidebar_select(&database_select)),
                        )
                    })
                    .when(show_schema_select, |view| {
                        view.child(
                            div()
                                .id("schema-select")
                                .child(sidebar_select(&schema_select)),
                        )
                    })
                    .when(kind != DatabaseKind::Redis, |view| {
                        view.child(
                            div()
                                .h(px(SIDEBAR_CONTROL_HEIGHT))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .rounded(px(RADIUS_CONTROL))
                                .bg(theme().glass_hover)
                                .border_1()
                                .border_color(theme().hairline)
                                .child(icon(Icon::Search, theme().text_muted))
                                .child(div().flex_1().min_w_0().child(editor::bare_input(
                                    search_editor,
                                    search_focus,
                                    px(SIDEBAR_CONTROL_HEIGHT - 2.),
                                ))),
                        )
                    }),
            )
            .child(
                div().flex_1().min_h_0().py(px(6.)).child(
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
                                        selected_table.as_ref(),
                                        selected_schema.as_deref(),
                                        cx,
                                    )
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .h_full(),
                ),
            )
            .into_any_element()
    }

    fn render_main(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                Pane::Data => self.render_data(cx).into_any_element(),
                Pane::Structure => self.render_structure(cx).into_any_element(),
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
                            SecondaryTabKind::Query(_) => {
                                query_number += 1;
                                (format!("Query {query_number}"), Icon::Query)
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
            .h(px(40.))
            .flex_none()
            .px(px(8.))
            .flex()
            .min_w_0()
            .items_center()
            .gap(px(4.))
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
                )
                .child(label)
                .child(
                    div()
                        .id(SharedString::from(format!("close-document-{tab_id}")))
                        .size(px(18.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .hover(|style| style.bg(theme().glass_hover))
                        .tooltip(tip("Close tab"))
                        .child(icon(Icon::Close, theme().text_muted).size(px(12.)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            if let Some(session_id) = session_id {
                                this.request_close_secondary_tab_for(
                                    session_id, tab_id, window, cx,
                                );
                            }
                        })),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(session_id) = session_id {
                        this.activate_secondary_tab_for(session_id, tab_id, window, cx);
                    }
                }))
            }))
            .child(
                glass_icon_button("add-query-document", Icon::Add, false)
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
                            return (None, String::new(), None, None, String::new());
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
                if let Some((_, _, page, _, _)) = table_pagination {
                    let page_number = page.saturating_add(1);
                    if result.rows.is_empty() {
                        format!(
                            "No rows · page {page_number} · {TABLE_BROWSE_PAGE_SIZE}/page"
                        )
                    } else {
                        let first_row = page
                            .saturating_mul(u64::from(TABLE_BROWSE_PAGE_SIZE))
                            .saturating_add(1);
                        let last_row = first_row + result.rows.len() as u64 - 1;
                        format!(
                            "Rows {first_row}–{last_row} · page {page_number} · {TABLE_BROWSE_PAGE_SIZE}/page"
                        )
                    }
                } else {
                    format!("{} rows", result.rows.len())
                }
            })
            .unwrap_or(summary);
        let pagination_controls =
            table_pagination.map(|(session_id, tab_id, page, has_next_page, busy)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
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
        div()
            .h(px(30.))
            .flex_none()
            .px(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .border_t_1()
            .border_color(theme().border)
            .follow_bottom_corners(RADIUS_PANEL)
            .bg(theme().panel)
            .text_size(px(11.))
            .text_color(theme().text_muted)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .when(error.is_some(), |view| view.text_color(theme().danger))
                    .child(error.unwrap_or(status)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(result_summary)
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
            .h(px(22.))
            .px(px(10.))
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
            .size(px(34.))
            .tooltip(tip(label))
            .on_click(listener)
    }
}

/// A pill-shaped document tab on the content sheet's tab strip.
fn document_tab(id: impl Into<ElementId>, kind: Icon, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .pl(px(10.))
        .pr(px(4.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .text_size(px(12.))
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
        .size(px(24.))
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
        .child(icon(kind, theme().text).size(px(12.)))
}

fn sidebar_select(state: &SidebarSelect) -> impl IntoElement {
    Select::new(state)
        .with_size(Size::Small)
        .h(px(SIDEBAR_CONTROL_HEIGHT))
        .rounded(px(RADIUS_CONTROL))
        .w_full()
        .menu_max_h(px(260.))
        .text_size(px(12.))
        .bg(theme().glass_hover)
        .border_color(theme().hairline)
        .text_color(theme().text)
}

fn sidebar_row(
    session_id: SessionId,
    table: &TableInfo,
    selected_table: Option<&TableRef>,
    selected_schema: Option<&str>,
    cx: &mut Context<DbxApp>,
) -> Div {
    let selected = selected_table
        .is_some_and(|current| current.name == table.name && current.schema == table.schema);
    let label = table_sidebar_label(table, selected_schema);
    let menu_table = table.clone();
    let table_ref = table.clone();
    div().w_full().px(px(6.)).child(
        div()
            .id(SharedString::from(table_sidebar_id(table)))
            .w_full()
            .h(px(28.))
            .px(px(8.))
            .rounded(px(RADIUS_CONTROL))
            .when(selected, |row| {
                row.bg(theme().accent_soft).font_weight(FontWeight::MEDIUM)
            })
            .text_color(if selected {
                theme().accent
            } else {
                theme().text
            })
            .text_size(px(12.))
            .flex()
            .items_center()
            .gap(px(7.))
            .cursor_pointer()
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
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_table_for(session_id, table_ref.clone(), window, cx);
            }))
            .on_aux_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if table_click_action(event) == TableClickAction::OpenContextMenu {
                    this.open_table_context_menu(
                        session_id,
                        menu_table.clone(),
                        event.position(),
                        cx,
                    );
                }
            })),
    )
}
