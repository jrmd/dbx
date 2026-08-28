use super::super::*;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

impl DbxApp {
    fn render_query_grid(
        result_grid: Entity<TableState<ResultTableDelegate>>,
        has_result: bool,
        has_rowset: bool,
        busy: bool,
        failed: bool,
    ) -> AnyElement {
        if !has_result {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme().text_muted)
                .child(if busy {
                    "Running query…"
                } else if failed {
                    "The query did not complete. Review the error above and try again."
                } else {
                    "Run a query to see rows"
                })
                .into_any_element();
        }

        if !has_rowset {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme().text_muted)
                .child("The statement completed without a row result.")
                .into_any_element();
        }

        div()
            .id("query-grid")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(
                DataTable::new(&result_grid)
                    .with_size(px(30.))
                    .stripe(false)
                    .bordered(false)
                    .scrollbar_visible(true, true),
            )
            .into_any_element()
    }

    fn render_sql_completion(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        anchor: Point<Pixels>,
        menu: SqlCompletionMenu,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = menu.selected;
        let rows = menu.items.iter().enumerate().map(|(index, item)| {
            let item = item.clone();
            let replacement_range = menu.replacement_range.clone();
            let item_kind = item.kind;
            div()
                .id(SharedString::from(format!(
                    "sql-completion-{session_id}-{tab_id}-{index}"
                )))
                .h(px(28.))
                .px(px(8.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .bg(if index == selected {
                    theme().accent_soft
                } else {
                    theme().panel_raised
                })
                .hover(|style| style.bg(theme().accent_soft))
                .child(
                    div()
                        .w(px(52.))
                        .flex_none()
                        .text_size(px(9.))
                        .text_color(item_kind.color())
                        .child(item_kind.label()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme().text)
                        .child(item.label.clone()),
                )
                .child(
                    div()
                        .max_w(px(190.))
                        .truncate()
                        .text_size(px(10.))
                        .text_color(theme().text_muted)
                        .child(item.detail.clone()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.accept_completion_for(
                        session_id,
                        tab_id,
                        replacement_range.clone(),
                        item.clone(),
                        window,
                        cx,
                    );
                }))
        });

        deferred(
            anchored()
                .position(anchor)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("sql-completion-menu")
                        .debug_selector(|| "sql-completion-menu".into())
                        .w(px(420.))
                        .max_h(px(300.))
                        .p(px(5.))
                        .rounded(px(7.))
                        .border_1()
                        .border_color(theme().border_strong)
                        .bg(theme().panel_raised)
                        .text_size(px(12.))
                        .child(
                            div()
                                .id("sql-completion-items")
                                .max_h(px(252.))
                                .overflow_y_scroll()
                                .children(rows),
                        )
                        .child(
                            div()
                                .mt(px(4.))
                                .pt(px(5.))
                                .px(px(8.))
                                .border_t_1()
                                .border_color(theme().border)
                                .text_size(px(9.))
                                .text_color(theme().text_muted)
                                .child("↑↓ navigate · Tab/Enter insert · Esc dismiss"),
                        ),
                ),
        )
        .with_priority(20)
        .into_any_element()
    }

    pub(super) fn render_query(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((
            tab_id,
            query_editor,
            busy,
            has_result,
            has_rowset,
            result_grid,
            sql_dialect,
            split_state,
            status,
            error,
            results_stale,
            truncated,
            executed_database,
        )) = self.session(session_id).and_then(|session| {
            let tab_id = session.active_secondary_tab?;
            let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
            let SecondaryTabKind::Query(query) = &tab.kind else {
                return None;
            };
            Some((
                tab_id,
                query.query_editor.clone(),
                query.busy,
                query.result.is_some(),
                query
                    .result
                    .as_ref()
                    .is_some_and(|result| !result.columns.is_empty()),
                query.result_grid.clone(),
                session.kind.is_sql(),
                query.split_state.clone(),
                query.status.clone(),
                query.error.clone(),
                query.results_stale,
                query.result.as_ref().is_some_and(|result| result.truncated),
                query.executed_database.clone(),
            ))
        })
        else {
            return div().into_any_element();
        };
        // Paint failed-query underlines only while the query revision still
        // matches the run that produced them; text edits clear the range.
        if sql_dialect {
            let highlight = self
                .session(session_id)
                .and_then(|session| {
                    let tab_id = session.active_secondary_tab?;
                    let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                    let SecondaryTabKind::Query(query) = &tab.kind else {
                        return None;
                    };
                    query.error_highlight.clone()
                })
                .map_or_else(Vec::new, |range| vec![range]);
            query_editor.update(cx, |editor, cx| editor.set_diagnostics(highlight, cx));
        }
        let query_focus = query_editor.read(cx).focus_handle();
        let completion = query_focus
            .is_focused(window)
            .then(|| self.query_completion_for(session_id, cx))
            .flatten();
        let completion_element = completion.map(|menu| {
            self.render_sql_completion(
                session_id,
                tab_id,
                query_editor.read(cx).completion_anchor(cx),
                menu,
                cx,
            )
        });
        let completion_key_listener = cx.listener(move |this, event, window, cx| {
            this.handle_completion_key(session_id, event, window, cx)
        });
        let completion_up_editor = query_editor.clone();
        let completion_down_editor = query_editor.clone();
        let completion_enter_editor = query_editor.clone();
        let mut editor_panel = div()
            .relative()
            .flex_1()
            .min_h_0()
            .p(px(10.))
            .key_context(editor::SQL_EDITOR_CONTEXT)
            .capture_key_down(completion_key_listener)
            .on_action(cx.listener(move |this, _: &CompletionUp, window, cx| {
                this.handle_completion_action(
                    session_id,
                    CompletionAction::Up,
                    completion_up_editor.clone(),
                    window,
                    cx,
                );
                cx.stop_propagation();
            }))
            .on_action(cx.listener(move |this, _: &CompletionDown, window, cx| {
                this.handle_completion_action(
                    session_id,
                    CompletionAction::Down,
                    completion_down_editor.clone(),
                    window,
                    cx,
                );
                cx.stop_propagation();
            }))
            .on_action(cx.listener(move |this, _: &CompletionEnter, window, cx| {
                this.handle_completion_action(
                    session_id,
                    CompletionAction::Enter,
                    completion_enter_editor.clone(),
                    window,
                    cx,
                );
                cx.stop_propagation();
            }))
            .when(sql_dialect, |panel| {
                panel.on_action(cx.listener(move |this, _: &FormatQuery, window, cx| {
                    this.format_query_for(session_id, window, cx);
                }))
            })
            .on_action(cx.listener(move |this, _: &RunQuery, window, cx| {
                this.request_run_query_for(session_id, false, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(move |this, _: &RunQueryAll, window, cx| {
                this.request_run_query_for(session_id, true, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(move |this, _: &CancelQuery, _, cx| {
                this.cancel_query_for(session_id, cx);
                cx.stop_propagation();
            }))
            .child(editor::sql_input_fill(query_editor, query_focus));
        if let Some(completion_element) = completion_element {
            editor_panel = editor_panel.child(completion_element);
        }
        let result_label = if error.is_some() {
            "Failed"
        } else if busy {
            "Running"
        } else if results_stale {
            "Stale result"
        } else if truncated {
            "Results limited"
        } else if has_result {
            "Complete"
        } else {
            "Ready"
        };
        let result_color = if error.is_some() {
            theme().danger
        } else if busy || results_stale || truncated {
            theme().warning
        } else if has_result {
            theme().success
        } else {
            theme().text_muted
        };
        let editor_label = if sql_dialect {
            "SQL editor"
        } else {
            "Command editor"
        };
        let app = cx.entity().downgrade();
        let history = self
            .recent_query_history_for(session_id)
            .into_iter()
            .take(10)
            .collect::<Vec<_>>();

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .key_context("QueryWorkbench")
            .on_action(cx.listener(move |this, _: &CancelQuery, _, cx| {
                this.cancel_query_for(session_id, cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .h(px(38.))
                    .flex_none()
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme().border)
                    .bg(theme().panel)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(icon(Icon::Query, theme().accent))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(editor_label),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .when(!busy, |actions| {
                                actions.child(
                                    button("run-query", "Run", ButtonKind::Primary)
                                        .tooltip(
                                            "Run selection or current statement (Cmd/Ctrl+Enter)",
                                        )
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.request_run_query_for(
                                                session_id, false, window, cx,
                                            );
                                        })),
                                )
                            })
                            .when(busy, |actions| {
                                actions.child(
                                    button("cancel-query", "Cancel", ButtonKind::Quiet)
                                        .tooltip("Cancel the active query (Escape)")
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.cancel_query_for(session_id, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("query-workbench-more")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Query options")
                                    .child(icon(Icon::More, theme().text_muted))
                                    .dropdown_menu(move |menu, _, _| {
                                        let run_all = app.clone();
                                        let format_query = app.clone();
                                        let copy_selection = app.clone();
                                        let copy_tsv = app.clone();
                                        let copy_csv = app.clone();
                                        let copy_json = app.clone();
                                        let export_tsv = app.clone();
                                        let export_csv = app.clone();
                                        let export_json = app.clone();
                                        let reopen_last = app.clone();
                                        let clear_history = app.clone();
                                        let mut menu = menu;
                                        if sql_dialect {
                                            menu = menu
                                                .item(
                                                    PopupMenuItem::new("Run all")
                                                        .disabled(busy)
                                                        .on_click(move |_, window, cx| {
                                                            let _ = run_all.update(
                                                                cx,
                                                                |this, cx| {
                                                                    this.request_run_query_for(
                                                                        session_id, true, window, cx,
                                                                    );
                                                                },
                                                            );
                                                        }),
                                                )
                                                .item(PopupMenuItem::new("Format query").on_click(
                                                    move |_, window, cx| {
                                                        let _ =
                                                            format_query.update(cx, |this, cx| {
                                                                this.format_query_for(
                                                                    session_id, window, cx,
                                                                );
                                                            });
                                                    },
                                                ));
                                        }
                                        let mut menu = menu
                                            .separator()
                                            .item(
                                                PopupMenuItem::new("Copy selection")
                                                    .disabled(!has_rowset)
                                                    .on_click(move |_, _, cx| {
                                                        let _ = copy_selection.update(
                                                            cx,
                                                            |this, cx| {
                                                                this.copy_query_selection_for(
                                                                    session_id, cx,
                                                                );
                                                            },
                                                        );
                                                    }),
                                            )
                                            .item(
                                                PopupMenuItem::new("Copy result as TSV")
                                                    .disabled(!has_rowset)
                                                    .on_click(move |_, _, cx| {
                                                        let _ = copy_tsv.update(cx, |this, cx| {
                                                            this.copy_query_result_for(
                                                                session_id,
                                                                QueryResultExportFormat::Tsv,
                                                                cx,
                                                            );
                                                        });
                                                    }),
                                            )
                                            .item(
                                                PopupMenuItem::new("Copy result as CSV")
                                                    .disabled(!has_rowset)
                                                    .on_click(move |_, _, cx| {
                                                        let _ = copy_csv.update(cx, |this, cx| {
                                                            this.copy_query_result_for(
                                                                session_id,
                                                                QueryResultExportFormat::Csv,
                                                                cx,
                                                            );
                                                        });
                                                    }),
                                            )
                                            .item(
                                                PopupMenuItem::new("Copy result as JSON")
                                                    .disabled(!has_rowset)
                                                    .on_click(move |_, _, cx| {
                                                        let _ = copy_json.update(cx, |this, cx| {
                                                            this.copy_query_result_for(
                                                                session_id,
                                                                QueryResultExportFormat::Json,
                                                                cx,
                                                            );
                                                        });
                                                    }),
                                            )
                                            .separator()
                                            .item(
                                                PopupMenuItem::new("Export TSV…")
                                                    .disabled(!has_rowset)
                                                    .on_click(move |_, _, cx| {
                                                        let _ =
                                                            export_tsv.update(cx, |this, cx| {
                                                                this.export_query_result_for(
                                                                    session_id,
                                                                    QueryResultExportFormat::Tsv,
                                                                    cx,
                                                                );
                                                            });
                                                    }),
                                            )
                                            .item(
                                                PopupMenuItem::new("Export CSV…")
                                                    .disabled(!has_rowset)
                                                    .on_click(move |_, _, cx| {
                                                        let _ =
                                                            export_csv.update(cx, |this, cx| {
                                                                this.export_query_result_for(
                                                                    session_id,
                                                                    QueryResultExportFormat::Csv,
                                                                    cx,
                                                                );
                                                            });
                                                    }),
                                            )
                                            .item(
                                                PopupMenuItem::new("Export JSON…")
                                                    .disabled(!has_rowset)
                                                    .on_click(move |_, _, cx| {
                                                        let _ =
                                                            export_json.update(cx, |this, cx| {
                                                                this.export_query_result_for(
                                                                    session_id,
                                                                    QueryResultExportFormat::Json,
                                                                    cx,
                                                                );
                                                            });
                                                    }),
                                            )
                                            .separator()
                                            .item(
                                                PopupMenuItem::new("Reopen closed query").on_click(
                                                    move |_, window, cx| {
                                                        let _ =
                                                            reopen_last.update(cx, |this, cx| {
                                                                this.reopen_last_closed_query_for(
                                                                    session_id, window, cx,
                                                                );
                                                            });
                                                    },
                                                ),
                                            )
                                            .item(
                                                PopupMenuItem::new("Clear query history")
                                                    .disabled(history.is_empty())
                                                    .on_click(move |_, window, cx| {
                                                        let _ =
                                                            clear_history.update(cx, |this, cx| {
                                                                this.request_clear_query_history_for(
                                                                    session_id, window, cx,
                                                                );
                                                            });
                                                    }),
                                            );
                                        if !history.is_empty() {
                                            menu = menu.separator();
                                            for (index, entry) in history.iter().enumerate() {
                                                let entry = entry.clone();
                                                let load_history = app.clone();
                                                let compact_sql = entry
                                                    .sql
                                                    .split_whitespace()
                                                    .collect::<Vec<_>>()
                                                    .join(" ");
                                                let compact_sql =
                                                    if compact_sql.chars().count() > 56 {
                                                        format!(
                                                            "{}…",
                                                            compact_sql
                                                                .chars()
                                                                .take(55)
                                                                .collect::<String>()
                                                        )
                                                    } else {
                                                        compact_sql
                                                    };
                                                menu = menu.item(
                                                    PopupMenuItem::new(SharedString::from(
                                                        format!(
                                                            "Recent {}: {compact_sql}",
                                                            index + 1
                                                        ),
                                                    ))
                                                    .on_click(move |_, window, cx| {
                                                        let _ =
                                                            load_history.update(cx, |this, cx| {
                                                                this.load_query_history_entry_for(
                                                                    session_id, &entry, window, cx,
                                                                );
                                                            });
                                                    }),
                                                );
                                            }
                                        }
                                        menu.scrollable(true)
                                    }),
                            ),
                    ),
            )
            .child(
                gpui_component::resizable::v_resizable(SharedString::from(format!(
                    "query-workbench-split-{session_id}-{tab_id}"
                )))
                .with_state(&split_state)
                .child(
                    gpui_component::resizable::resizable_panel()
                        .size(px(224.))
                        .size_range(px(164.)..px(520.))
                        .child(editor_panel),
                )
                .child(
                    gpui_component::resizable::resizable_panel().child(
                        div()
                            .size_full()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .key_context("QueryResult")
                            .on_action(cx.listener(Self::copy_query_selection_action))
                            .child(
                                div()
                                    .h(px(30.))
                                    .flex_none()
                                    .px(px(10.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .border_b_1()
                                    .border_color(theme().border)
                                    .bg(theme().panel)
                                    .child(
                                        div()
                                            .flex()
                                            .min_w_0()
                                            .items_center()
                                            .gap(px(7.))
                                            .child(
                                                div()
                                                    .size(px(6.))
                                                    .flex_none()
                                                    .rounded_full()
                                                    .bg(result_color),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .text_size(px(11.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(theme().text)
                                                    .child(result_label),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(10.))
                                                    .text_color(theme().text_muted)
                                                    .child(status),
                                            ),
                                    )
                                    .when_some(executed_database, |strip, database| {
                                        strip.child(
                                            div()
                                                .ml(px(10.))
                                                .flex_none()
                                                .text_size(px(10.))
                                                .text_color(theme().text_muted)
                                                .child(database),
                                        )
                                    }),
                            )
                            .when_some(error.clone(), |panel, error| {
                                panel.child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "query-error-{session_id}-{tab_id}"
                                        )))
                                        .mx(px(10.))
                                        .mt(px(8.))
                                        .mb(px(4.))
                                        .px(px(9.))
                                        .py(px(7.))
                                        .rounded(px(5.))
                                        .border_1()
                                        .border_color(theme().danger)
                                        .bg(theme().panel_raised)
                                        .max_h(px(180.))
                                        .overflow_y_scroll()
                                        .whitespace_normal()
                                        .text_size(px(11.))
                                        .text_color(theme().text)
                                        .child(error)
                                        .child(
                                            div()
                                                .mt(px(6.))
                                                .flex()
                                                .gap(px(6.))
                                                .child(
                                                    button(
                                                        "copy-query-error",
                                                        "Copy error",
                                                        ButtonKind::Quiet,
                                                    )
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.copy_query_error_for(session_id, cx);
                                                    })),
                                                )
                                                .when(sql_dialect, |actions| {
                                                    actions.child(
                                                        button(
                                                            "locate-query-error",
                                                            "Locate in editor",
                                                            ButtonKind::Quiet,
                                                        )
                                                        .cursor_pointer()
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.focus_query_error_for(
                                                                    session_id, window, cx,
                                                                );
                                                            },
                                                        )),
                                                    )
                                                }),
                                        ),
                                )
                            })
                            .child(Self::render_query_grid(
                                result_grid,
                                has_result,
                                has_rowset,
                                busy,
                                error.is_some(),
                            )),
                    ),
                ),
            )
            .into_any_element()
    }
}
