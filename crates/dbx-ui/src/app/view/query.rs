use super::super::*;
use crate::popups::DropdownMenu as _;
use crate::workspace::SavedQuery;
use gpui::WeakEntity;
use gpui_component::menu::{PopupMenu, PopupMenuItem};

impl DbxApp {
    fn render_query_grid(
        result_grid: Entity<TableState<ResultTableDelegate>>,
        has_result: bool,
        has_rowset: bool,
    ) -> AnyElement {
        if !has_result {
            // The status strip and error banner already say what happened.
            return div().flex_1().into_any_element();
        }

        if !has_rowset {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme().text_muted)
                .child("Statement completed · no rows returned")
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
            // The kind column already says keyword/type/function/column.
            let detail = match item.detail.as_str() {
                "SQL keyword" | "SQL type" | "function" | "table" => "",
                detail => detail.strip_prefix("column · ").unwrap_or(detail),
            }
            .to_string();
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
                        .text_color(theme().text_muted)
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
                        .child(detail),
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
        let (in_transaction, active_result, statement_labels, transactions_supported) = self
            .session(session_id)
            .and_then(|session| {
                let query = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Query(query) = &query.kind else {
                    return None;
                };
                Some((
                    query.in_transaction,
                    query.active_result,
                    query
                        .statement_results
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            format!(
                                "{} · {}",
                                index + 1,
                                if item.error.is_some() {
                                    "Failed"
                                } else {
                                    "Result"
                                }
                            )
                        })
                        .collect::<Vec<_>>(),
                    matches!(
                        session.kind,
                        DatabaseKind::PostgreSQL
                            | DatabaseKind::CockroachDB
                            | DatabaseKind::MySQL
                            | DatabaseKind::SQLite
                    ),
                ))
            })
            .unwrap_or_default();
        // Paint failed-query underlines only while the query revision still
        // matches the run that produced them; text edits clear the range.
        if sql_dialect {
            let diagnostics = self.query_editor_diagnostics(session_id, cx);
            query_editor.update(cx, |editor, cx| editor.set_diagnostics(diagnostics, cx));
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
        // A finished or idle result needs no label; the row counts say it.
        let result_label = if error.is_some() {
            Some("Failed")
        } else if busy {
            Some("Running")
        } else if results_stale {
            Some("Stale result")
        } else if truncated {
            Some("Results limited")
        } else {
            None
        };
        let result_color = if error.is_some() {
            theme().danger
        } else if busy || results_stale || truncated {
            theme().warning
        } else {
            theme().text_muted
        };
        let app = cx.entity().downgrade();
        let saved_queries = self.saved_queries_for(session_id);
        let timeout_secs = self
            .session(session_id)
            .and_then(|session| session.secondary_tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Query(query) => Some(query.timeout_secs),
                _ => None,
            })
            .unwrap_or(60);
        let name_editor = self
            .session(session_id)
            .and_then(|session| session.secondary_tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Query(query) => Some(query.name_editor.clone()),
                _ => None,
            })
            .unwrap();
        let name_focus = name_editor.read(cx).focus_handle();
        let history = self.recent_query_history_limited(session_id, 10);
        let history_policy = self
            .session(session_id)
            .and_then(query_history_connection)
            .and_then(|connection| {
                self.workspace_documents
                    .get(&crate::workspace::connection_key(&connection))
            });
        let history_disabled = history_policy.is_some_and(|document| document.history_disabled);
        let history_retention = history_policy.map_or(100, |document| document.history_retention);
        let agent_open = self.agent_panel_open(session_id, tab_id);
        let agent_panel = self.render_agent_panel(session_id, tab_id, cx);
        let parameter_prompt = self.render_parameter_prompt(session_id, cx);
        let find_bar = self.render_find_bar(session_id, cx);
        let export_target = self
            .active_query_tab(session_id)
            .unwrap()
            .export_target
            .clone();
        let inspected_value = self
            .active_query_tab(session_id)
            .and_then(|query| query.inspected_value.clone());

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
            .on_action(cx.listener(move |this, _: &ToggleQueryAgent, window, cx| {
                this.toggle_agent_panel(session_id, tab_id, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(move |this, _: &OpenFind, window, cx| {
                this.open_find_for(session_id, false, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(move |this, _: &OpenReplace, window, cx| {
                this.open_find_for(session_id, true, window, cx);
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
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div().w(px(180.)).min_w(px(72.)).flex_shrink(1.).child(
                                    editor::input_with_key_context(
                                        name_editor,
                                        name_focus,
                                        false,
                                        editor::TEXT_EDITOR_CONTEXT,
                                    )
                                    .h(px(28.))
                                    .py(px(5.)),
                                ),
                            )
                            .child(
                                button("save-named-query", "Save", ButtonKind::Quiet)
                                    .debug_selector(|| "save-named-query".into())
                                    .tooltip("Save this query to the connection")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.save_named_query_for(session_id, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(7.))
                            .child(
                        div()
                            .id("describe-query")
                            .h(px(28.))
                            .px(px(10.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .when(agent_open, |toggle| {
                                toggle
                                    .bg(theme().glass_selected)
                                    .border_1()
                                    .border_color(theme().hairline)
                                    .text_color(theme().text)
                                    .font_weight(FontWeight::MEDIUM)
                            })
                            .when(!agent_open, |toggle| {
                                toggle.text_color(theme().text_muted).hover(|style| {
                                    style.bg(theme().glass_hover).text_color(theme().text)
                                })
                            })
                            .tooltip(tip(format!("Query assistant ({})", shortcut("K", "K"))))
                            .child(icon(
                                Icon::Sparkles,
                                if agent_open {
                                    theme().accent
                                } else {
                                    theme().text_muted
                                },
                            ))
                            .child("Ask AI")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle_agent_panel(session_id, tab_id, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .when(transactions_supported && !busy, |actions| {
                                if in_transaction {
                                    actions
                                        .child(button("commit-query", "Commit", ButtonKind::Primary)
                                            .debug_selector(|| "commit-query".into())
                                            .on_click(cx.listener(move |this, _, _, cx| this.run_console_command_for(session_id, "COMMIT", cx))))
                                        .child(button("rollback-query", "Rollback", ButtonKind::Quiet)
                                            .debug_selector(|| "rollback-query".into())
                                            .on_click(cx.listener(move |this, _, _, cx| this.run_console_command_for(session_id, "ROLLBACK", cx))))
                                } else {
                                    actions.child(button("begin-query", "Begin", ButtonKind::Quiet)
                                        .tooltip("Begin a transaction in this query tab")
                                        .on_click(cx.listener(move |this, _, _, cx| this.run_console_command_for(session_id, "BEGIN", cx))))
                                }
                            })
                            .when(!busy, |actions| {
                                actions.child(
                                    button("run-query", "Run", ButtonKind::Primary)
                                        .tooltip(format!(
                                            "Run statement ({})",
                                            shortcut("↵", "Enter")
                                        ))
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
                                        .tooltip("Cancel (Esc)")
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.cancel_query_for(session_id, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("query-workbench-more")
                                    .debug_selector(|| "query-workbench-more".into())
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Query options")
                                    .child(icon(Icon::More, theme().text_muted))
                                    .dropdown_menu({
                                        let options = std::rc::Rc::new(QueryOptions {
                                            app: app.clone(),
                                            session_id,
                                            sql: sql_dialect,
                                            busy,
                                            has_rowset,
                                            timeout_secs,
                                            saved: saved_queries.clone(),
                                            history: history.clone(),
                                            history_disabled,
                                            history_retention,
                                        });
                                        move |menu, window, cx| options.clone().menu(menu, window, cx)
                                    }),
                            ),
                    ),
                    ),
            )
            .when(in_transaction, |view| view.child(div().px(px(10.)).py(px(4.)).text_xs().text_color(theme().warning)
                .child("Transaction open in this tab · Commit to save or Rollback to discard. Closing the tab rolls it back.")))
            .when(statement_labels.len() > 1, |view| view.child(div().flex().gap(px(4.)).px(px(8.)).py(px(4.))
                .children(statement_labels.into_iter().enumerate().map(|(index, label)| {
                    button(SharedString::from(format!("statement-result-{index}")), label,
                        if index == active_result { ButtonKind::Primary } else { ButtonKind::Quiet })
                    .debug_selector(move || format!("statement-result-{index}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_statement_result_for(session_id, index, cx)))
                }))))
            .when_some(agent_panel, |view, panel| view.child(panel))
            .when_some(parameter_prompt, |view, prompt| view.child(prompt))
            .when_some(find_bar, |view, bar| view.child(bar))
            .when(has_rowset, |view| view.child(div().flex().items_center().gap(px(8.)).px(px(10.)).py(px(4.))
                .child(div().text_size(px(11.)).child("INSERT export target table:"))
                .child(div().w(px(220.)).child(export_target))))
            .when_some(inspected_value, |view, value| view.child(
                div().id("query-value-viewer").max_h(px(300.)).overflow_y_scroll().p(px(10.))
                    .child(div().flex().justify_between().child("Value")
                        .child(button("copy-query-value", "Copy full value", ButtonKind::Quiet).on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(value) = this.active_query_tab(session_id).and_then(|query| query.inspected_value.as_ref()) {
                                cx.write_to_clipboard(ClipboardItem::new_string(value_view::value_clipboard_text(value)));
                            }
                        })))
                        .child(button("close-query-value", "Close", ButtonKind::Quiet).on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(query) = this.active_query_tab_mut(session_id) { query.inspected_value = None; }
                            cx.notify();
                        }))))
                    .child(crate::app::value_view::value_view(Some(&value)))))
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
                                            .when(result_label.is_some(), |strip| {
                                                strip.child(
                                                    div()
                                                        .size(px(6.))
                                                        .flex_none()
                                                        .rounded_full()
                                                        .bg(result_color),
                                                )
                                            })
                                            .when_some(result_label, |strip, label| {
                                                strip.child(
                                                    div()
                                                        .flex_none()
                                                        .text_size(px(11.))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(theme().text)
                                                        .child(label),
                                                )
                                            })
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(10.))
                                                    .text_color(theme().text_muted)
                                                    .child(if error.is_some() || busy {
                                                        SharedString::default()
                                                    } else {
                                                        status.into()
                                                    }),
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
                            )),
                    ),
                ),
            )
            .into_any_element()
    }
}

/// What the query tab's options menu acts on, shared by its submenus.
struct QueryOptions {
    app: WeakEntity<DbxApp>,
    session_id: SessionId,
    sql: bool,
    busy: bool,
    has_rowset: bool,
    timeout_secs: u64,
    saved: Vec<SavedQuery>,
    history: Vec<QueryHistoryEntry>,
    history_disabled: bool,
    history_retention: usize,
}

impl QueryOptions {
    /// A menu item that runs `action` on the app.
    fn item(
        &self,
        label: impl Into<SharedString>,
        action: impl Fn(&mut DbxApp, &mut Window, &mut Context<DbxApp>) + 'static,
    ) -> PopupMenuItem {
        let app = self.app.clone();
        PopupMenuItem::new(label).on_click(move |_, window, cx| {
            let _ = app.update(cx, |this, cx| action(this, window, cx));
        })
    }

    fn menu(
        self: std::rc::Rc<Self>,
        mut menu: PopupMenu,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let session_id = self.session_id;
        if self.sql {
            menu = menu
                .item(
                    self.item("Run all", move |this, window, cx| {
                        this.request_run_query_for(session_id, true, window, cx)
                    })
                    .disabled(self.busy),
                )
                .item(self.item("Format query", move |this, window, cx| {
                    this.format_query_for(session_id, window, cx)
                }))
                .separator()
                .item(
                    self.item("Explain query", move |this, _, cx| {
                        this.explain_query_for(session_id, cx)
                    })
                    .disabled(self.busy),
                );
            let options = self.clone();
            menu = menu.submenu("Diagnostics", window, cx, move |menu, _, _| {
                let busy = options.busy;
                menu.item(
                    options
                        .item("Capture plan", move |this, _, cx| {
                            this.pin_plan_for(session_id, cx)
                        })
                        .disabled(busy),
                )
                .item(
                    options
                        .item("Compare plans", move |this, _, cx| {
                            this.compare_plan_for(session_id, cx)
                        })
                        .disabled(busy),
                )
                .separator()
                .item(options.item("Server sessions", move |this, window, cx| {
                    this.open_monitor_for(session_id, dbx_core::Monitor::Sessions, window, cx)
                }))
                .item(options.item("Lock waits", move |this, window, cx| {
                    this.open_monitor_for(session_id, dbx_core::Monitor::Locks, window, cx)
                }))
                .separator()
                .item(
                    options
                        .item("Capture schema baseline", move |this, window, cx| {
                            this.inspect_schema_for(session_id, true, window, cx)
                        })
                        .disabled(busy),
                )
                .item(
                    options
                        .item(
                            "Compare schema / draft migration",
                            move |this, window, cx| {
                                this.inspect_schema_for(session_id, false, window, cx)
                            },
                        )
                        .disabled(busy),
                )
            });
            menu = menu.separator();
        }

        let options = self.clone();
        menu = menu.submenu("Copy result", window, cx, move |mut menu, _, _| {
            let rows = options.has_rowset;
            menu = menu.item(
                options
                    .item("Selection", move |this, _, cx| {
                        this.copy_query_selection_for(session_id, cx)
                    })
                    .disabled(!rows),
            );
            for (label, format) in [
                ("As TSV", QueryResultExportFormat::Tsv),
                ("As CSV", QueryResultExportFormat::Csv),
                ("As JSON", QueryResultExportFormat::Json),
                ("As INSERT statements", QueryResultExportFormat::Insert),
            ] {
                menu = menu.item(
                    options
                        .item(label, move |this, _, cx| {
                            this.copy_query_result_for(session_id, format, cx)
                        })
                        .disabled(!rows),
                );
            }
            menu
        });
        let options = self.clone();
        menu = menu.submenu("Export", window, cx, move |mut menu, _, _| {
            menu = menu.label("Loaded rows");
            for (label, format) in [
                ("CSV…", QueryResultExportFormat::Csv),
                ("TSV…", QueryResultExportFormat::Tsv),
                ("JSON…", QueryResultExportFormat::Json),
                ("INSERT statements…", QueryResultExportFormat::Insert),
            ] {
                menu = menu.item(
                    options
                        .item(label, move |this, _, cx| {
                            this.export_query_result_for(session_id, format, cx)
                        })
                        .disabled(!options.has_rowset),
                );
            }
            if options.sql {
                menu = menu.separator().label("Full query");
                for (label, format) in [
                    ("CSV…", dbx_core::QueryExportFormat::Csv),
                    ("TSV…", dbx_core::QueryExportFormat::Tsv),
                    ("Typed JSONL…", dbx_core::QueryExportFormat::JsonLines),
                ] {
                    menu = menu.item(
                        options
                            .item(label, move |this, _, cx| {
                                this.export_full_query_for(session_id, format, cx)
                            })
                            .disabled(options.busy),
                    );
                }
            }
            menu
        });
        menu = menu.separator();

        if !self.saved.is_empty() {
            let options = self.clone();
            menu = menu.submenu("Saved queries", window, cx, move |mut menu, window, cx| {
                for saved in &options.saved {
                    let saved = saved.clone();
                    menu = menu.item(options.item(saved.name.clone(), move |this, window, cx| {
                        this.open_saved_query_for(session_id, saved.clone(), window, cx)
                    }));
                }
                let delete = options.clone();
                menu.separator()
                    .submenu("Delete", window, cx, move |mut menu, _, _| {
                        for saved in &delete.saved {
                            let name = saved.name.clone();
                            menu = menu.item(delete.item(
                                saved.name.clone(),
                                move |this, window, cx| {
                                    this.request_delete_saved_query_for(
                                        session_id,
                                        name.clone(),
                                        window,
                                        cx,
                                    )
                                },
                            ));
                        }
                        menu
                    })
            });
        }
        let options = self.clone();
        menu = menu.submenu("History", window, cx, move |mut menu, window, cx| {
            menu = menu.item(options.item("Search…", |this, window, cx| {
                this.search_history(window, cx)
            }));
            if !options.history.is_empty() {
                menu = menu.separator().label("Recent");
                for entry in &options.history {
                    let entry = entry.clone();
                    let compact = entry.sql.split_whitespace().collect::<Vec<_>>().join(" ");
                    let label = if compact.chars().count() > 56 {
                        format!("{}…", compact.chars().take(55).collect::<String>())
                    } else {
                        compact
                    };
                    menu = menu.item(options.item(label, move |this, window, cx| {
                        this.load_query_history_entry_for(session_id, &entry, window, cx)
                    }));
                }
            }
            let recording = !options.history_disabled;
            let retention = options.clone();
            menu.separator()
                .item(
                    options
                        .item("Record history", move |this, _, cx| {
                            this.set_history_policy(session_id, Some(recording), None, cx)
                        })
                        .checked(recording),
                )
                .submenu("Keep", window, cx, move |mut menu, _, _| {
                    for limit in [10, 30, 100] {
                        menu = menu.item(
                            retention
                                .item(format!("{limit} entries"), move |this, _, cx| {
                                    this.set_history_policy(session_id, None, Some(limit), cx)
                                })
                                .checked(retention.history_retention == limit),
                        );
                    }
                    menu
                })
                .separator()
                .item(
                    options
                        .item("Clear history…", move |this, window, cx| {
                            this.request_clear_query_history_for(session_id, window, cx)
                        })
                        .disabled(options.history.is_empty()),
                )
        });
        let options = self.clone();
        menu = menu.submenu("Timeout", window, cx, move |mut menu, _, _| {
            for seconds in [5, 30, 60, 300] {
                let label = if seconds < 60 {
                    format!("{seconds} seconds")
                } else {
                    format!(
                        "{} minute{}",
                        seconds / 60,
                        if seconds == 60 { "" } else { "s" }
                    )
                };
                menu = menu.item(
                    options
                        .item(label, move |this, _, cx| {
                            this.set_query_timeout_for(session_id, seconds, cx)
                        })
                        .checked(options.timeout_secs == seconds),
                );
            }
            menu
        });
        menu.separator()
            .item(self.item("Reopen closed query", move |this, window, cx| {
                this.reopen_last_closed_query_for(session_id, window, cx)
            }))
    }
}
