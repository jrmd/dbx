use super::super::*;
use crate::popups::DropdownMenu as _;
use crate::workspace::SavedQuery;
use gpui::WeakEntity;
use gpui_component::menu::{PopupMenu, PopupMenuItem};

impl DbxApp {
    fn render_plan_tree(result: &QueryResult) -> Option<AnyElement> {
        let column = |name: &str| result.columns.iter().position(|column| column.name == name);
        let (depth, operation) = (column("Depth")?, column("Operation")?);
        let rows = result.rows.iter().enumerate().map(|(index, row)| {
            let text = |name: &str| {
                column(name)
                    .and_then(|index| row.values.get(index))
                    .map(|value| match value {
                        CellValue::Text(value) => value.clone(),
                        CellValue::Integer(value) => value.to_string(),
                        _ => String::new(),
                    })
                    .unwrap_or_default()
            };
            let level = match row.values.get(depth) {
                Some(CellValue::Integer(value)) => (*value).clamp(0, 40) as f32,
                _ => 0.,
            };
            let name = match row.values.get(operation) {
                Some(CellValue::Text(value)) => value.clone(),
                _ => "Plan step".into(),
            };
            let actual = text("Actual time (ms)");
            let estimates = format!(
                "rows {} · cost {}",
                text("Estimated rows"),
                text("Estimated cost")
            );
            let metrics = if actual.is_empty() {
                estimates
            } else {
                format!(
                    "{estimates} · actual {actual} ms/loop · {} rows/loop · {} loops",
                    text("Actual rows"),
                    text("Actual loops")
                )
            };
            div()
                .id(("query-plan-node", index))
                .role(gpui::Role::TreeItem)
                .aria_label(format!(
                    "Level {}: {name}. {metrics}. {}",
                    level as usize + 1,
                    text("Detail")
                ))
                .ml(crate::theme::ui_px(level * 18.))
                .pl(crate::theme::ui_px(10.))
                .py(crate::theme::ui_px(7.))
                .border_l_1()
                .border_color(theme().border_strong)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(crate::theme::ui_px(8.))
                        .child(div().text_color(theme().accent_text).child(if level > 0. {
                            "↳"
                        } else {
                            "●"
                        }))
                        .child(div().font_weight(FontWeight::MEDIUM).child(name))
                        .child(
                            div()
                                .text_size(crate::theme::text_px(10.))
                                .text_color(theme().text_muted)
                                .child(metrics),
                        ),
                )
                .child(
                    div()
                        .pl(crate::theme::ui_px(18.))
                        .text_size(crate::theme::text_px(11.))
                        .text_color(theme().text_muted)
                        .child(text("Detail")),
                )
        });
        Some(
            div()
                .id("query-plan-tree")
                .debug_selector(|| "query-plan-tree".into())
                .role(gpui::Role::Tree)
                .aria_label("Execution plan")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(crate::theme::ui_px(12.))
                .children(rows)
                .into_any_element(),
        )
    }

    fn render_query_grid(
        result_grid: Entity<TableState<ResultTableDelegate>>,
        has_result: bool,
        has_rowset: bool,
        failed: bool,
    ) -> AnyElement {
        if failed || !has_result {
            // The status strip and error banner already say what happened.
            return div().flex_1().into_any_element();
        }

        if !has_rowset {
            return div()
                .debug_selector(|| "query-no-rows".into())
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
            .child(crate::app::result_table::accessible_grid(
                &result_grid,
                "Query results",
                false,
            ))
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
                .h(crate::theme::control_height(28.))
                .px(crate::theme::ui_px(8.))
                .rounded(crate::theme::ui_px(4.))
                .flex()
                .items_center()
                .gap(crate::theme::ui_px(8.))
                .cursor_pointer()
                .bg(if index == selected {
                    theme().accent_soft
                } else {
                    theme().panel_raised
                })
                .hover(|style| style.bg(theme().accent_soft))
                .child(
                    div()
                        .w(crate::theme::ui_px(52.))
                        .flex_none()
                        .text_size(crate::theme::text_px(9.))
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
                        .max_w(crate::theme::ui_px(190.))
                        .truncate()
                        .text_size(crate::theme::text_px(10.))
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
                .snap_to_window_with_margin(crate::theme::ui_px(8.))
                .child(
                    div()
                        .id("sql-completion-menu")
                        .debug_selector(|| "sql-completion-menu".into())
                        .w(crate::theme::ui_px(420.))
                        .max_h(crate::theme::ui_px(300.))
                        .p(crate::theme::ui_px(5.))
                        .rounded(crate::theme::ui_px(7.))
                        .border_1()
                        .border_color(theme().border_strong)
                        .bg(theme().panel_raised)
                        .text_size(crate::theme::text_px(12.))
                        .child(
                            div()
                                .id("sql-completion-items")
                                .max_h(crate::theme::ui_px(252.))
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
        let plan_available = self.active_query_tab(session_id).is_some_and(|query| {
            query.result.as_ref().is_some_and(|result| {
                result
                    .columns
                    .first()
                    .is_some_and(|column| column.name == "Depth")
                    && result
                        .columns
                        .get(1)
                        .is_some_and(|column| column.name == "Operation")
            })
        });
        let plan_tree = self
            .active_query_tab(session_id)
            .is_some_and(|query| query.plan_tree);
        let watch_interval = self
            .active_query_tab(session_id)
            .and_then(|query| query.watch_interval);
        let watch_supported = self
            .session(session_id)
            .and_then(|session| session.engine.as_ref())
            .is_some_and(|engine| QuerySession::supports_isolated_reads(engine));
        let analyze_supported = watch_supported
            && self.session(session_id).is_some_and(|session| {
                matches!(
                    session.kind,
                    DatabaseKind::PostgreSQL | DatabaseKind::MySQL | DatabaseKind::CockroachDB
                )
            });
        let plan_view = (error.is_none() && plan_tree)
            .then(|| {
                self.active_query_tab(session_id)
                    .and_then(|query| query.result.as_deref())
                    .and_then(Self::render_plan_tree)
            })
            .flatten();
        // Paint failed-query underlines only while the query revision still
        // matches the run that produced them; text edits clear the range.
        if sql_dialect {
            let diagnostics = self.query_editor_diagnostics(session_id, cx);
            query_editor.update(cx, |editor, cx| editor.set_diagnostics(diagnostics, cx));
        }
        let query_focus = query_editor.read(cx).focus_handle();
        query_editor.update(cx, |editor, cx| editor.set_vim_mode(self.vim_mode, cx));
        let vim_mode = query_editor.read(cx).vim_mode();
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
        let completion_tab_editor = query_editor.clone();
        let mut editor_panel = div()
            .relative()
            .flex_1()
            .min_h_0()
            .p(crate::theme::ui_px(10.))
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
            .on_action(cx.listener(move |this, _: &CompletionTab, window, cx| {
                this.handle_completion_action(
                    session_id,
                    CompletionAction::Tab,
                    completion_tab_editor.clone(),
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
            .child(editor::sql_input_fill(query_editor, query_focus, cx));
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
            theme().danger_text
        } else if busy || results_stale || truncated {
            theme().warning_text
        } else {
            theme().text_muted
        };
        let app = cx.entity().downgrade();
        let saved_queries = self.saved_queries_for(session_id);
        let compare_connections = self
            .sessions
            .iter()
            .filter(|session| {
                session.id != session_id
                    && session.engine.is_some()
                    && self
                        .session(session_id)
                        .is_some_and(|target| session.kind.dialect() == target.kind.dialect())
            })
            .map(|session| (session.id, session.name.clone()))
            .collect::<Vec<_>>();
        self.ensure_sql_library_loaded(cx);
        let (sql_files, sql_library_error) = match self.sql_library.cached_entries() {
            Ok(entries) => (entries, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let sql_library_path = self.sql_library.root().display().to_string();
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
        let export_prompt = self.active_query_tab(session_id).and_then(|query| {
            query.export_prompt.map(|action| {
                let export_target = query.export_target.clone();
                let focus = export_target.read(cx).focus_handle();
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(8.))
                    .px(crate::theme::ui_px(10.))
                    .py(crate::theme::ui_px(4.))
                    .child(
                        div()
                            .text_size(crate::theme::text_px(11.))
                            .child("INSERT target table:"),
                    )
                    .child(div().w(crate::theme::ui_px(220.)).child(editor::input(
                        export_target,
                        focus,
                        false,
                        cx,
                    )))
                    .child(
                        button(
                            "submit-query-insert-export",
                            match action {
                                QueryResultExportAction::Copy => "Copy",
                                QueryResultExportAction::Export => "Export…",
                            },
                            ButtonKind::Primary,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.submit_query_export_prompt_for(session_id, window, cx);
                            },
                        )),
                    )
                    .child(
                        button("cancel-query-insert-export", "Cancel", ButtonKind::Quiet).on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.dismiss_query_export_prompt_for(session_id, window, cx);
                            }),
                        ),
                    )
                    .into_any_element()
            })
        });
        let inspected_value = self
            .active_query_tab(session_id)
            .and_then(|query| query.inspected_value.clone());

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .key_context("QueryWorkbench")
            .on_action(cx.listener(move |this, _: &CancelQuery, window, cx| {
                if this.active_query_tab(session_id).is_some_and(|query| query.export_prompt.is_some()) {
                    this.dismiss_query_export_prompt_for(session_id, window, cx);
                } else {
                    this.cancel_query_for(session_id, cx);
                }
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
                    .h(crate::theme::control_height(38.))
                    .flex_none()
                    .px(crate::theme::ui_px(9.))
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
                            .gap(crate::theme::ui_px(6.))
                            .child(
                                div().w(crate::theme::ui_px(180.)).min_w(crate::theme::ui_px(72.)).flex_shrink(1.).child(
                                    editor::input_with_key_context(
                                        name_editor,
                                        name_focus,
                                        false,
                                        editor::TEXT_EDITOR_CONTEXT,
                                     cx)
                                    .h(crate::theme::control_height(28.))
                                    .py(crate::theme::ui_px(5.)),
                                ),
                            )
                            .child(
                                button("save-named-query", "Save to vault", ButtonKind::Quiet)
                                    .debug_selector(|| "save-named-query".into())
                                    .accessible_name("Save this query in the encrypted vault").tooltip("Save this query in the encrypted vault")
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
                            .gap(crate::theme::ui_px(7.))
                            .child(
                        div()
                            .id("describe-query")
                            .pressable("Describe query with assistant")
                            .h(crate::theme::control_height(28.))
                            .px(crate::theme::ui_px(10.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .gap(crate::theme::ui_px(6.))
                            .text_size(crate::theme::text_px(12.))
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
                            .gap(crate::theme::ui_px(7.))
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
                                        .accessible_name("Begin a transaction in this query tab").tooltip("Begin a transaction in this query tab")
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
                                        .accessible_name("Cancel (Esc)").tooltip("Cancel (Esc)")
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
                                    .accessible_name("Query options").tooltip("Query options")
                                    .child(icon(Icon::More, theme().text_muted))
                                    .dropdown_menu({
                                        let options = std::rc::Rc::new(QueryOptions {
                                            app: app.clone(),
                                            session_id,
                                            sql: sql_dialect,
                                            full_export: self.session(session_id).and_then(|session| session.engine.as_ref()).is_some_and(|engine| engine.capabilities().streaming_query_export),
                                            busy,
                                            has_rowset,
                                            timeout_secs,
                                            watch_interval,
                                            watch_supported,
                                            analyze_supported,
                                            saved: saved_queries.clone(),
                                            compare_connections: compare_connections.clone(),
                                            sql_files: sql_files.clone(),
                                            sql_library_error: sql_library_error.clone(),
                                            sql_library_path: sql_library_path.clone(),
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
            .when(in_transaction, |view| view.child(div().px(crate::theme::ui_px(10.)).py(crate::theme::ui_px(4.)).text_size(crate::theme::text_px(12.)).text_color(theme().warning_text)
                .child("Transaction open in this tab · Commit to save or Rollback to discard. Closing the tab rolls it back.")))
            .when_some(watch_interval, |view, interval| view.child(
                div().id("query-watch-status").debug_selector(|| "query-watch-status".into()).px(crate::theme::ui_px(10.)).py(crate::theme::ui_px(4.)).flex().items_center().gap(crate::theme::ui_px(8.))
                    .text_size(crate::theme::text_px(11.)).text_color(theme().accent_text)
                    .child(format!("Watch · every {interval}s while visible · isolated read-only transaction · edits stop watch"))
                    .child(button("stop-query-watch", "Stop watch", ButtonKind::Quiet).debug_selector(|| "stop-query-watch".into()).on_click(cx.listener(move |this, _, _, cx| this.stop_query_watch(session_id, cx))))
            ))
            .when_some(vim_mode, |view, mode| view.child(div().px(crate::theme::ui_px(10.)).py(crate::theme::ui_px(4.)).text_size(crate::theme::text_px(12.)).text_color(theme().accent_text)
                .debug_selector(|| "vim-mode-indicator".into())
                .child(format!("{} · i to insert · Esc for Normal · F6 to leave editor", mode.label()))))
            .when(statement_labels.len() > 1, |view| view.child(div().flex().gap(crate::theme::ui_px(4.)).px(crate::theme::ui_px(8.)).py(crate::theme::ui_px(4.))
                .children(statement_labels.into_iter().enumerate().map(|(index, label)| {
                    button(SharedString::from(format!("statement-result-{index}")), label,
                        if index == active_result { ButtonKind::Primary } else { ButtonKind::Quiet })
                    .debug_selector(move || format!("statement-result-{index}"))
                    .on_click(cx.listener(move |this, _, _, cx| this.select_statement_result_for(session_id, index, cx)))
                }))))
            .when_some(agent_panel, |view, panel| view.child(panel))
            .when_some(parameter_prompt, |view, prompt| view.child(prompt))
            .when_some(find_bar, |view, bar| view.child(bar))
            .when_some(export_prompt, |view, prompt| view.child(prompt))
            .when_some(inspected_value, |view, value| view.child(
                div().id("query-value-viewer").max_h(crate::theme::ui_px(300.)).overflow_y_scroll().p(crate::theme::ui_px(10.))
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
                    .child(crate::app::value_view::value_view(Some(&value), cx))))
            .child(
                gpui_component::resizable::v_resizable(SharedString::from(format!(
                    "query-workbench-split-{session_id}-{tab_id}"
                )))
                .with_state(&split_state)
                .child(
                    gpui_component::resizable::resizable_panel()
                        .size(crate::theme::ui_px(224.))
                        .size_range(crate::theme::ui_px(164.)..crate::theme::ui_px(520.))
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
                                    .h(crate::theme::control_height(30.))
                                    .flex_none()
                                    .px(crate::theme::ui_px(10.))
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
                                            .gap(crate::theme::ui_px(7.))
                                            .when(result_label.is_some(), |strip| {
                                                strip.child(
                                                    div()
                                                        .size(crate::theme::ui_px(6.))
                                                        .flex_none()
                                                        .rounded_full()
                                                        .bg(result_color),
                                                )
                                            })
                                            .when_some(result_label, |strip, label| {
                                                strip.child(
                                                    crate::accessibility::announcement("query-outcome", label, error.is_some())
                                                        .flex_none().text_size(crate::theme::text_px(11.))
                                                        .font_weight(FontWeight::MEDIUM).text_color(theme().text)
                                                        .child(label),
                                                )
                                            })
                                            .child(
                                                crate::accessibility::announcement("query-status", if error.is_some() { String::new() } else { status.clone() }, false)
                                                    .min_w_0().truncate().text_size(crate::theme::text_px(10.))
                                                    .text_color(theme().text_muted)
                                                    .child(if error.is_some() || busy {
                                                        SharedString::default()
                                                    } else {
                                                        status.into()
                                                    }),
                                            ),
                                    )
                                    .when(plan_available, |strip| strip.child(
                                        button("toggle-plan-tree", if plan_tree { "Show grid" } else { "Show plan tree" }, ButtonKind::Quiet)
                                            .debug_selector(|| "toggle-plan-tree".into())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(query) = this.active_query_tab_mut(session_id) { query.plan_tree = !query.plan_tree; }
                                                cx.notify();
                                            }))
                                    ))
                                    .when_some(executed_database, |strip, database| {
                                        strip.child(
                                            div()
                                                .ml(crate::theme::ui_px(10.))
                                                .flex_none()
                                                .text_size(crate::theme::text_px(10.))
                                                .text_color(theme().text_muted)
                                                .child(database),
                                        )
                                    }),
                            )
                            .when_some(error.clone(), |panel, error| {
                                panel.child(
                                    crate::accessibility::announcement(SharedString::from(format!(
                                            "query-error-{session_id}-{tab_id}")), error.clone(), true)
                                        .mx(crate::theme::ui_px(10.))
                                        .mt(crate::theme::ui_px(8.))
                                        .mb(crate::theme::ui_px(4.))
                                        .px(crate::theme::ui_px(9.))
                                        .py(crate::theme::ui_px(7.))
                                        .rounded(crate::theme::ui_px(5.))
                                        .border_1()
                                        .border_color(theme().danger)
                                        .bg(theme().panel_raised)
                                        .max_h(crate::theme::ui_px(180.))
                                        .overflow_y_scroll()
                                        .whitespace_normal()
                                        .text_size(crate::theme::text_px(11.))
                                        .text_color(theme().text)
                                        .child(error)
                                        .child(
                                            div()
                                                .mt(crate::theme::ui_px(6.))
                                                .flex()
                                                .gap(crate::theme::ui_px(6.))
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
                                                        button("fix-query-error", "Ask AI to fix", ButtonKind::Quiet)
                                                            .debug_selector(|| "fix-query-error".into())
                                                            .tooltip("Review an editable request containing the query and database error before sending it to your assistant")
                                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                                this.ask_agent_to_fix_query(session_id, tab_id, window, cx);
                                                            })),
                                                    ).child(
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
                            .child(plan_view.unwrap_or_else(|| Self::render_query_grid(
                                result_grid,
                                has_result,
                                has_rowset,
                                error.is_some(),
                            ))),
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
    full_export: bool,
    busy: bool,
    has_rowset: bool,
    timeout_secs: u64,
    watch_interval: Option<u64>,
    watch_supported: bool,
    analyze_supported: bool,
    saved: Vec<SavedQuery>,
    compare_connections: Vec<(SessionId, String)>,
    sql_files: Vec<crate::query_library::SqlFileEntry>,
    sql_library_error: Option<String>,
    sql_library_path: String,
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
                )
                .item(
                    self.item(
                        "EXPLAIN ANALYZE… (executes query)",
                        move |this, window, cx| {
                            this.request_analyze_query_for(session_id, window, cx)
                        },
                    )
                    .disabled(self.busy || !self.analyze_supported),
                );
            let options = self.clone();
            menu = menu.submenu(
                "Watch read query (isolated read-only)",
                window,
                cx,
                move |mut menu, _, _| {
                    for interval in [5, 10, 30, 60] {
                        menu = menu.item(
                            options
                                .item(
                                    format!("Every {interval} seconds while visible"),
                                    move |this, _, cx| {
                                        this.start_query_watch(session_id, interval, cx)
                                    },
                                )
                                .checked(options.watch_interval == Some(interval))
                                .disabled(options.busy || !options.watch_supported),
                        );
                    }
                    menu.separator().item(
                        options
                            .item("Stop watch", move |this, _, cx| {
                                this.stop_query_watch(session_id, cx)
                            })
                            .disabled(options.watch_interval.is_none()),
                    )
                },
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
            let options = self.clone();
            menu = menu.submenu(
                "Make this connection’s schema match…",
                window,
                cx,
                move |mut menu, _, _| {
                    if options.compare_connections.is_empty() {
                        return menu.label("Open another connection of the same database type");
                    }
                    for (source_id, label) in &options.compare_connections {
                        let source_id = *source_id;
                        menu = menu.item(
                            options
                                .item(label.clone(), move |this, window, cx| {
                                    this.compare_connection_schemas(
                                        session_id, source_id, window, cx,
                                    )
                                })
                                .disabled(options.busy),
                        );
                    }
                    menu
                },
            );
            menu = menu.separator();
        }

        let options = self.clone();
        menu = menu.item(
            options
                .item(
                    "Column statistics (loaded rows)",
                    move |this, window, cx| {
                        this.show_loaded_column_statistics(session_id, window, cx)
                    },
                )
                .disabled(!options.has_rowset),
        );
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
                ("As Markdown", QueryResultExportFormat::Markdown),
                ("As INSERT statements", QueryResultExportFormat::Insert),
            ] {
                menu = menu.item(
                    options
                        .item(label, move |this, window, cx| {
                            this.request_query_result_export_for(
                                session_id,
                                format,
                                QueryResultExportAction::Copy,
                                window,
                                cx,
                            )
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
                ("Markdown…", QueryResultExportFormat::Markdown),
                ("Excel XLSX…", QueryResultExportFormat::Xlsx),
                ("INSERT statements…", QueryResultExportFormat::Insert),
            ] {
                menu = menu.item(
                    options
                        .item(label, move |this, window, cx| {
                            this.request_query_result_export_for(
                                session_id,
                                format,
                                QueryResultExportAction::Export,
                                window,
                                cx,
                            )
                        })
                        .disabled(!options.has_rowset),
                );
            }
            if options.sql {
                menu = menu.separator().label("Full query · fresh read-only rerun");
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
                menu = menu.item(
                    options
                        .item("Excel XLSX…", move |this, _, cx| {
                            this.export_full_query_xlsx_for(session_id, cx)
                        })
                        .disabled(options.busy || !options.full_export),
                );
            }
            menu
        });
        menu = menu.separator();

        let files = self.clone();
        menu = menu.submenu("SQL file library", window, cx, move |mut menu, _, _| {
            menu = menu
                .label(format!("Plaintext .sql files · {}", files.sql_library_path))
                .item(files.item("Choose library folder…", |this, _, cx| {
                    this.choose_sql_library_folder(cx)
                }))
                .item(files.item("Refresh files", |this, _, cx| {
                    this.refresh_sql_library(cx);
                }))
                .item(files.item("Open .sql file…", move |this, window, cx| {
                    this.open_sql_file_for(session_id, window, cx)
                }))
                .item(
                    files.item("Save current query as .sql…", move |this, _, cx| {
                        this.save_query_sql_file_for(session_id, cx)
                    }),
                );
            if let Some(error) = &files.sql_library_error {
                menu = menu.label(error.clone());
            }
            if files.sql_files.is_empty() {
                menu = menu.separator().label("No .sql files in this folder");
            }
            menu = menu
                .separator()
                .item(files.item("Browse SQL files…", move |this, window, cx| {
                    this.browse_sql_files(false, window, cx)
                }))
                .item(
                    files.item("Browse favorite SQL files…", move |this, window, cx| {
                        this.browse_sql_files(true, window, cx)
                    }),
                );
            for entry in files
                .sql_files
                .iter()
                .filter(|entry| entry.favorite)
                .take(8)
            {
                let path = entry.relative_path.clone();
                menu = menu.item(files.item(
                    format!("★ {}", entry.label()),
                    move |this, window, cx| {
                        this.open_library_sql_for(session_id, path.clone(), window, cx)
                    },
                ));
            }
            menu
        });

        if !self.saved.is_empty() {
            let options = self.clone();
            menu = menu.submenu(
                "Vault saved queries",
                window,
                cx,
                move |mut menu, window, cx| {
                    for saved in &options.saved {
                        let saved = saved.clone();
                        menu =
                            menu.item(options.item(saved.name.clone(), move |this, window, cx| {
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
                },
            );
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

#[cfg(test)]
mod regression_tests {
    use super::*;

    #[gpui::test]
    fn query_error_hides_success_message_and_opens_repair_prompt(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.connection_picker_open = false;
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            let tab_id = Uuid::new_v4();
            let mut query = QueryTab::new(DatabaseKind::SQLite, session_id, tab_id, window, cx);
            query
                .query_editor
                .update(cx, |editor, cx| editor.set_text("SELECT missing", cx));
            query.set_result(Some(QueryResult::empty(None, 0)), cx);
            query.error = Some("no such column: missing".into());
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Query(Box::new(query)),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Query;
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("query-no-rows").is_none());
        let fix = cx
            .debug_bounds("fix-query-error")
            .expect("repair button visible");
        cx.simulate_click(fix.center(), Default::default());
        app.read_with(cx, |app, _| {
            let tab_id = app
                .session(session_id)
                .unwrap()
                .active_secondary_tab
                .unwrap();
            assert!(app.agent_panel_open(session_id, tab_id));
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.active_query_tab_mut(session_id).unwrap().error = None;
                cx.notify();
            })
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("query-no-rows").is_some());
    }

    #[gpui::test]
    fn tab_and_enter_accept_completion_and_tab_otherwise_indents(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(|cx| {
            cx.bind_keys(editor::default_key_bindings());
            cx.bind_keys([
                gpui::KeyBinding::new("tab", CompletionTab, Some(editor::SQL_EDITOR_CONTEXT)),
                gpui::KeyBinding::new("enter", CompletionEnter, Some(editor::SQL_EDITOR_CONTEXT)),
            ]);
        });
        let session_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.connection_picker_open = false;
            app.vim_mode = false;
            app.sessions = vec![ConnectionSession::new(
                session_id,
                None,
                "test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            )];
            app.active_session_id = Some(session_id);
            app.add_query_tab_for(session_id, window, cx);
            app
        });
        let editor = app.read_with(cx, |app, _| {
            app.active_query_tab(session_id)
                .unwrap()
                .query_editor
                .clone()
        });
        for key in ["tab", "enter"] {
            cx.update(|window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.set_text("SELE", cx);
                    editor.move_cursor_to(4, cx);
                    editor.focus_handle().focus(window, cx);
                })
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("sql-completion-menu").is_some());
            cx.simulate_keystrokes(key);
            assert_eq!(
                editor.read_with(cx, |editor, cx| editor.text(cx)).trim(),
                "SELECT"
            );
        }
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_text("", cx);
                editor.focus_handle().focus(window, cx);
            })
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("sql-completion-menu").is_none());
        cx.simulate_keystrokes("tab");
        assert_eq!(editor.read_with(cx, |editor, cx| editor.text(cx)), "  ");
    }
}
