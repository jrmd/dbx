use super::super::*;

impl DbxApp {
    pub(super) fn render_workspace(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .bg(THEME.canvas)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(self.render_main(window, cx)),
            )
            .child(self.render_status(cx))
            .child(self.render_table_context_menu(cx))
            .child(self.render_database_export_dialog(window, cx))
    }

    pub(super) fn render_app_rail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_pane = self.active_session().map(|session| session.pane);
        div()
            .w(px(46.))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .border_r_1()
            .border_color(THEME.border)
            .bg(THEME.rail)
            .child(
                div()
                    .h(px(42.))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_b_1()
                    .border_color(THEME.border)
                    .child(img(self.logo.clone()).id("rail-logo").size(px(24.))),
            )
            .child(
                div()
                    .flex_1()
                    .py(px(8.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(self.rail_button(
                        "rail-data",
                        Icon::Table,
                        active_pane == Some(Pane::Data),
                        cx.listener(|this, _, _, cx| this.set_active_pane(Pane::Data, cx)),
                    ))
                    .child(self.rail_button(
                        "rail-structure",
                        Icon::Structure,
                        active_pane == Some(Pane::Structure),
                        cx.listener(|this, _, _, cx| this.set_active_pane(Pane::Structure, cx)),
                    ))
                    .child(self.rail_button(
                        "rail-query",
                        Icon::Query,
                        active_pane == Some(Pane::Query),
                        cx.listener(|this, _, window, cx| {
                            if let Some(session_id) = this.active_session_id() {
                                this.add_query_tab_for(session_id, window, cx);
                            }
                        }),
                    )),
            )
            .child(
                div()
                    .mb(px(11.))
                    .size(px(7.))
                    .rounded_full()
                    .bg(THEME.success),
            )
    }

    pub(super) fn set_active_pane(&mut self, pane: Pane, cx: &mut Context<Self>) {
        self.connection_picker_open = false;
        let Some(session_id) = self.active_session_id else {
            return;
        };
        match pane {
            Pane::Data => {
                if let Some(session) = self.session_mut(session_id) {
                    session.active_secondary_tab = None;
                    session.pane = Pane::Data;
                }
            }
            Pane::Query => return,
            Pane::Structure => {
                let table = self.session(session_id).and_then(|session| {
                    session.selected_table.as_ref().and_then(|selected| {
                        session
                            .tables
                            .iter()
                            .find(|table| table_ref(table) == *selected)
                            .cloned()
                    })
                });
                if let Some(table) = table {
                    self.open_structure_tab_for(session_id, table, cx);
                    return;
                }
                if let Some(session) = self.session_mut(session_id) {
                    session.status = "Select a table before opening its structure".into();
                }
            }
        }
        cx.notify();
    }

    pub(super) fn render_topbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self
            .active_session()
            .is_some_and(|session| session.engine.is_some());
        div()
            .h(px(42.))
            .flex_none()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(THEME.border)
            .bg(THEME.rail)
            .when(!connected, |view| {
                view.child(
                    div()
                        .w(if self.compact_layout {
                            px(96.)
                        } else {
                            px(122.)
                        })
                        .flex_none()
                        .px(px(12.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(img(self.logo.clone()).id("topbar-logo").size(px(18.)))
                        .child(
                            div()
                                .id("window-title-drag")
                                .flex_1()
                                .h_full()
                                .flex()
                                .items_center()
                                .window_control_area(WindowControlArea::Drag)
                                .on_mouse_down(MouseButton::Left, |_, window, _| {
                                    window.start_window_move();
                                })
                                .on_double_click(|_, window, _| window.zoom_window())
                                .text_size(px(15.))
                                .font_weight(FontWeight::BOLD)
                                .child("DBX"),
                        ),
                )
            })
            .child(self.render_connection_tabs(cx))
            .child(
                div()
                    .id("window-title-drag-spacer")
                    .w(if self.compact_layout {
                        px(24.)
                    } else {
                        px(48.)
                    })
                    .h_full()
                    .flex_none()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    })
                    .on_double_click(|_, window, _| window.zoom_window()),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .when(connected && !self.compact_layout, |view| {
                        view.child(self.rail_button(
                            "refresh",
                            Icon::Refresh,
                            false,
                            cx.listener(|this, _, _, cx| this.refresh_table(cx)),
                        ))
                    })
                    .child(
                        button("connections", "New connection", ButtonKind::Primary)
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx))),
                    )
                    .child(window_close_button().on_click(|_, _, cx| cx.quit())),
            )
    }

    fn render_connection_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_session_id = self.active_session_id();
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
                    session.environment,
                )
            })
            .collect();
        div()
            .id("connection-tabs-scroll")
            .flex_1()
            .min_w_0()
            .px(px(6.))
            .h_full()
            .flex()
            .items_end()
            .gap(px(3.))
            .overflow_scroll()
            .children(sessions.into_iter().map(
                |(session_id, label, busy, kind, saved, environment)| {
                    let selected = active_session_id == Some(session_id);
                    connection_tab(kind, label, selected)
                        .id(SharedString::from(format!("connection-tab-{session_id}")))
                        .flex_none()
                        .cursor_pointer()
                        .child(div().size(px(5.)).rounded_full().bg(if busy {
                            THEME.warning
                        } else {
                            THEME.success
                        }))
                        .when(saved, |tab| tab.child(environment_badge(environment)))
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "close-connection-tab-{session_id}"
                                )))
                                .size(px(18.))
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.panel_raised))
                                .child(icon(Icon::Close, THEME.text_muted))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.close_session(session_id, cx)
                                })),
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.activate_session(session_id, cx)
                            }),
                        )
                },
            ))
            .child(
                div()
                    .id("add-connection-tab")
                    .flex_none()
                    .mb(px(5.))
                    .size(px(26.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(THEME.panel_raised))
                    .child(icon(Icon::Add, THEME.accent))
                    .on_click(cx.listener(|this, _, _, cx| this.begin_new_connection(cx))),
            )
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((kind, tables, databases, current_database, selected_schema, selected_table)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.tables.clone(),
                    session.databases.clone(),
                    session.current_database.clone(),
                    session.schema_filter.clone(),
                    session.selected_table.clone(),
                )
            })
        else {
            return div().into_any_element();
        };
        let schema_options = schema_filter_options(kind, &tables);
        let visible_tables = schema_filtered_tables(kind, &tables, selected_schema.as_deref());
        div()
            .w(if self.compact_layout {
                px(180.)
            } else {
                px(224.)
            })
            .flex_none()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(THEME.border)
            .bg(THEME.panel)
            .child(
                div()
                    .px(px(10.))
                    .py(px(7.))
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .border_b_1()
                    .border_color(THEME.border)
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
                                    .child(icon(Icon::Search, THEME.text_muted))
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(THEME.text_muted)
                                            .child(if kind == DatabaseKind::Redis {
                                                "KEYSPACE"
                                            } else {
                                                "EXPLORER"
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .child(
                                        div()
                                            .id("refresh-tables")
                                            .size(px(24.))
                                            .rounded(px(5.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(|style| style.bg(THEME.panel_raised))
                                            .child(icon(Icon::Refresh, THEME.accent))
                                            .on_click(cx.listener(move |this, _, _window, cx| {
                                                this.refresh_tables_for(session_id, cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("create-table")
                                            .size(px(24.))
                                            .rounded(px(5.))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .cursor_pointer()
                                            .hover(|style| style.bg(THEME.panel_raised))
                                            .child(icon(Icon::Add, THEME.accent))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.create_table_template_for(
                                                    session_id, window, cx,
                                                )
                                            })),
                                    )
                                    .when(kind.is_sql(), |view| {
                                        view.child(
                                            div()
                                                .id("export-database")
                                                .px(px(7.))
                                                .py(px(5.))
                                                .rounded(px(5.))
                                                .text_size(px(9.))
                                                .text_color(THEME.accent)
                                                .cursor_pointer()
                                                .hover(|style| style.bg(THEME.accent_soft))
                                                .child("Export")
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.begin_database_export(
                                                            session_id, window, cx,
                                                        )
                                                    },
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id("import-database")
                                                .px(px(7.))
                                                .py(px(5.))
                                                .rounded(px(5.))
                                                .text_size(px(9.))
                                                .text_color(THEME.text_muted)
                                                .cursor_pointer()
                                                .hover(|style| style.bg(THEME.panel_raised))
                                                .child("Import")
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.begin_database_import(
                                                            session_id, window, cx,
                                                        )
                                                    },
                                                )),
                                        )
                                    }),
                            ),
                    )
                    .when(databases.len() > 1, |view| {
                        view.child(
                            div()
                                .id("database-switcher-scroll")
                                .flex()
                                .gap(px(4.))
                                .overflow_x_scroll()
                                .children(databases.into_iter().map(|database| {
                                    let selected =
                                        current_database.as_deref() == Some(database.as_str());
                                    let label = if kind == DatabaseKind::Redis {
                                        format!("db{database}")
                                    } else {
                                        database.clone()
                                    };
                                    div()
                                        .id(SharedString::from(format!("db-{database}")))
                                        .px(px(7.))
                                        .py(px(3.))
                                        .rounded(px(4.))
                                        .bg(if selected {
                                            THEME.accent_soft
                                        } else {
                                            THEME.panel_raised
                                        })
                                        .text_color(if selected {
                                            THEME.accent
                                        } else {
                                            THEME.text_muted
                                        })
                                        .text_size(px(9.))
                                        .cursor_pointer()
                                        .hover(|style| {
                                            style.bg(THEME.panel_raised).text_color(THEME.text)
                                        })
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.switch_database_for(
                                                session_id,
                                                database.clone(),
                                                cx,
                                            )
                                        }))
                                })),
                        )
                    })
                    .when(kind == DatabaseKind::PostgreSQL, |view| {
                        view.child(
                            div()
                                .id("schema-filter-scroll")
                                .flex()
                                .gap(px(4.))
                                .overflow_x_scroll()
                                .children(schema_options.into_iter().map(|schema| {
                                    let selected = selected_schema.as_deref() == schema.as_deref();
                                    let label = schema.as_deref().unwrap_or("All").to_owned();
                                    let schema_id = schema_filter_id(schema.as_deref());
                                    div()
                                        .id(SharedString::from(schema_id))
                                        .px(px(7.))
                                        .py(px(3.))
                                        .rounded(px(4.))
                                        .bg(if selected {
                                            THEME.accent_soft
                                        } else {
                                            THEME.panel_raised
                                        })
                                        .text_color(if selected {
                                            THEME.accent
                                        } else {
                                            THEME.text_muted
                                        })
                                        .text_size(px(9.))
                                        .cursor_pointer()
                                        .hover(|style| {
                                            style.bg(THEME.panel_raised).text_color(THEME.text)
                                        })
                                        .child(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.select_schema_filter_for(
                                                session_id,
                                                schema.clone(),
                                                cx,
                                            )
                                        }))
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .py(px(5.))
                    .children(visible_tables.into_iter().map(|table| {
                        let selected = selected_table.as_ref().is_some_and(|current| {
                            current.name == table.name && current.schema == table.schema
                        });
                        let label = table_sidebar_label(&table, selected_schema.as_deref());
                        let menu_table = table.clone();
                        div()
                            .id(SharedString::from(table_sidebar_id(&table)))
                            .mx(px(5.))
                            .h(px(28.))
                            .px(px(8.))
                            .rounded(px(5.))
                            .bg(if selected {
                                THEME.accent_soft
                            } else {
                                THEME.panel
                            })
                            .text_color(if selected {
                                THEME.accent
                            } else {
                                THEME.text_muted
                            })
                            .text_size(px(11.))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .cursor_pointer()
                            .hover(|style| style.bg(THEME.panel_raised).text_color(THEME.text))
                            .child(icon(
                                if table.kind == EntityKind::Table {
                                    Icon::Table
                                } else {
                                    Icon::Search
                                },
                                if selected {
                                    THEME.accent
                                } else {
                                    THEME.text_muted
                                },
                            ))
                            .child(div().truncate().child(label))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_table_for(session_id, table.clone(), window, cx);
                            }))
                            .on_aux_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, _, cx| {
                                    if table_click_action(event)
                                        == TableClickAction::OpenContextMenu
                                    {
                                        this.open_table_context_menu(
                                            session_id,
                                            menu_table.clone(),
                                            event.position(),
                                            cx,
                                        );
                                    }
                                },
                            ))
                    })),
            )
            .into_any_element()
    }

    fn render_main(&mut self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = self
            .active_session()
            .map(|session| session.pane)
            .unwrap_or(Pane::Data);
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(self.render_tabs(cx))
            .child(match pane {
                Pane::Data => self.render_data(cx).into_any_element(),
                Pane::Structure => self.render_structure(cx).into_any_element(),
                Pane::Query => self.render_query(window, cx).into_any_element(),
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
                        let label = match &tab.kind {
                            SecondaryTabKind::Query(_) => {
                                query_number += 1;
                                format!("Query {query_number}")
                            }
                            SecondaryTabKind::Structure(structure) => {
                                format!("{} structure", structure.table.name)
                            }
                        };
                        (
                            tab.id,
                            label,
                            matches!(&tab.kind, SecondaryTabKind::Query(_)),
                        )
                    })
                    .collect::<Vec<_>>();
                (session.active_secondary_tab, tabs)
            })
            .unwrap_or_default();
        div()
            .id("document-tabs")
            .h(px(36.))
            .px(px(9.))
            .flex()
            .min_w_0()
            .items_end()
            .gap(px(3.))
            .overflow_x_scroll()
            .border_b_1()
            .border_color(THEME.border)
            .bg(THEME.panel)
            .child(
                div()
                    .id("document-data")
                    .h(px(31.))
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded_t(px(5.))
                    .border_1()
                    .border_color(if active_secondary_tab.is_none() {
                        THEME.border_strong
                    } else {
                        THEME.panel
                    })
                    .bg(if active_secondary_tab.is_none() {
                        THEME.canvas
                    } else {
                        THEME.panel
                    })
                    .text_color(if active_secondary_tab.is_none() {
                        THEME.text
                    } else {
                        THEME.text_muted
                    })
                    .text_size(px(11.))
                    .cursor_pointer()
                    .child(icon(
                        Icon::Table,
                        if active_secondary_tab.is_none() {
                            THEME.accent
                        } else {
                            THEME.text_muted
                        },
                    ))
                    .child("Data")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_active_pane(Pane::Data, cx)),
                    ),
            )
            .children(tabs.into_iter().map(|(tab_id, label, is_query)| {
                let selected = active_secondary_tab == Some(tab_id);
                div()
                    .id(SharedString::from(format!("document-{tab_id}")))
                    .h(px(31.))
                    .px(px(11.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded_t(px(5.))
                    .border_1()
                    .border_color(if selected {
                        THEME.border_strong
                    } else {
                        THEME.panel
                    })
                    .bg(if selected { THEME.canvas } else { THEME.panel })
                    .text_color(if selected {
                        THEME.text
                    } else {
                        THEME.text_muted
                    })
                    .text_size(px(11.))
                    .cursor_pointer()
                    .child(icon(
                        if is_query {
                            Icon::Query
                        } else {
                            Icon::Structure
                        },
                        if selected {
                            THEME.accent
                        } else {
                            THEME.text_muted
                        },
                    ))
                    .child(label)
                    .child(
                        div()
                            .id(SharedString::from(format!("close-document-{tab_id}")))
                            .ml(px(2.))
                            .size(px(18.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(THEME.text_muted)
                            .hover(|style| style.bg(THEME.panel_raised).text_color(THEME.danger))
                            .child(icon(Icon::Close, THEME.text_muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(session_id) = session_id {
                                    this.close_secondary_tab_for(session_id, tab_id, cx);
                                }
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(session_id) = session_id {
                            this.activate_secondary_tab_for(session_id, tab_id, cx);
                        }
                    }))
            }))
            .child(
                div()
                    .id("add-query-document")
                    .h(px(26.))
                    .w(px(26.))
                    .mb(px(2.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .text_color(THEME.text_muted)
                    .cursor_pointer()
                    .hover(|style| style.bg(THEME.accent_soft).text_color(THEME.accent))
                    .child(icon(Icon::Add, THEME.text_muted))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(session_id) = session_id {
                            this.add_query_tab_for(session_id, window, cx);
                        }
                    })),
            )
    }

    fn render_status(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (error, status, result, table_pagination) = self
            .active_session()
            .map(|session| {
                if let Some((error, status, result)) =
                    session.active_secondary_tab.and_then(|tab_id| {
                        session
                            .secondary_tabs
                            .iter()
                            .find(|tab| tab.id == tab_id)
                            .and_then(|tab| {
                                let SecondaryTabKind::Query(query) = &tab.kind else {
                                    return None;
                                };
                                Some((
                                    query.error.clone(),
                                    query.status.clone(),
                                    query.result.clone(),
                                ))
                            })
                    })
                {
                    return (error, status, result, None);
                }
                let table_pagination = (session.kind.is_sql()
                    && session.selected_table.is_some()
                    && session.pane == Pane::Data
                    && session.active_secondary_tab.is_none()
                    && session.result.is_some())
                .then_some((
                    session.id,
                    session.table_page,
                    session.table_has_next_page,
                    session.busy,
                ));
                (
                    session.error.clone(),
                    session.status.clone(),
                    session.result.clone(),
                    table_pagination,
                )
            })
            .unwrap_or_else(|| (self.error.clone(), self.status.clone(), None, None));
        let result_summary = result
            .as_ref()
            .map(|result| {
                if let Some((_, page, _, _)) = table_pagination {
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
            .unwrap_or_default();
        let pagination_controls =
            table_pagination.map(|(session_id, page, has_next_page, busy)| {
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(self.small_button_state(
                        "table-page-previous",
                        "Previous",
                        !busy && page > 0,
                        cx.listener(move |this, _, _, cx| {
                            this.set_table_page(session_id, page.saturating_sub(1), cx)
                        }),
                    ))
                    .child(self.small_button_state(
                        "table-page-next",
                        "Next",
                        !busy && has_next_page,
                        cx.listener(move |this, _, _, cx| {
                            this.set_table_page(session_id, page.saturating_add(1), cx)
                        }),
                    ))
            });
        div()
            .h(px(26.))
            .px(px(10.))
            .flex()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(THEME.border)
            .bg(THEME.panel)
            .text_size(px(10.))
            .text_color(THEME.text_muted)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
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

    pub(super) fn small_button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        self.small_button_state(id, label, true, listener)
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
            .with_size(Size::Small)
            .compact()
            .outline()
            .disabled(!enabled)
            .border_color(THEME.border)
            .bg(if enabled {
                THEME.panel_raised
            } else {
                THEME.panel
            })
            .text_color(if enabled {
                THEME.text
            } else {
                THEME.text_muted
            })
            .when(enabled, |view| view.cursor_pointer())
            .on_click(listener)
    }

    fn rail_button(
        &self,
        id: &'static str,
        kind: Icon,
        selected: bool,
        listener: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .size(px(32.))
            .rounded(px(6.))
            .flex()
            .items_center()
            .justify_center()
            .bg(if selected {
                THEME.accent_soft
            } else {
                THEME.rail
            })
            .cursor_pointer()
            .hover(|style| style.bg(THEME.panel_raised))
            .child(icon(
                kind,
                if selected {
                    THEME.accent
                } else {
                    THEME.text_muted
                },
            ))
            .on_click(listener)
    }
}
