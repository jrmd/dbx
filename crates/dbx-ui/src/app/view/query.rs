use super::super::*;

impl DbxApp {
    fn render_query_grid(
        result_grid: Entity<TableState<ResultTableDelegate>>,
        has_result: bool,
        busy: bool,
    ) -> AnyElement {
        if !has_result {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(THEME.text_muted)
                .child(if busy {
                    "Running query…"
                } else {
                    "Run a query to see rows"
                })
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
        menu: SqlCompletionMenu,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = menu.selected;
        let rows = menu.items.iter().enumerate().map(|(index, item)| {
            let item = item.clone();
            let context = menu.context.clone();
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
                    THEME.accent_soft
                } else {
                    THEME.panel_raised
                })
                .hover(|style| style.bg(THEME.accent_soft))
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
                        .text_color(THEME.text)
                        .child(item.label.clone()),
                )
                .child(
                    div()
                        .max_w(px(190.))
                        .truncate()
                        .text_size(px(10.))
                        .text_color(THEME.text_muted)
                        .child(item.detail.clone()),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.accept_completion_for(
                        session_id,
                        tab_id,
                        context.clone(),
                        item.clone(),
                        window,
                        cx,
                    );
                }))
        });

        deferred(
            div()
                .id("sql-completion-menu")
                .absolute()
                .left(px(10.))
                .right(px(10.))
                .top(px(214.))
                .max_h(px(300.))
                .p(px(5.))
                .rounded(px(7.))
                .border_1()
                .border_color(THEME.border_strong)
                .bg(THEME.panel_raised)
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
                        .border_color(THEME.border)
                        .text_size(px(9.))
                        .text_color(THEME.text_muted)
                        .child("↑↓ navigate · Tab/Enter insert · Esc dismiss"),
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
        let Some((tab_id, query_editor, busy, has_result, result_grid, sql_dialect)) =
            self.session(session_id).and_then(|session| {
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
                    query.result_grid.clone(),
                    session.kind.is_sql(),
                ))
            })
        else {
            return div().into_any_element();
        };
        // Paint failed-query underlines only while the text still matches the
        // run that produced them; any edit silently clears highlighting.
        if sql_dialect {
            let highlight = self
                .session(session_id)
                .and_then(|session| {
                    let tab_id = session.active_secondary_tab?;
                    let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                    let SecondaryTabKind::Query(query) = &tab.kind else {
                        return None;
                    };
                    Some((
                        query.error_highlight.clone(),
                        query.query_text.read(cx).clone(),
                    ))
                })
                .map(|(highlight, text)| match highlight {
                    Some((snapshot, range)) if snapshot == text => vec![range],
                    _ => Vec::new(),
                });
            if let Some(ranges) = highlight {
                query_editor.update(cx, |editor, cx| editor.set_diagnostics(ranges, cx));
            }
        }
        let query_focus = query_editor.read(cx).focus_handle();
        let completion = query_focus
            .is_focused(window)
            .then(|| self.query_completion_for(session_id, cx))
            .flatten();
        let completion_element =
            completion.map(|menu| self.render_sql_completion(session_id, tab_id, menu, cx));
        let completion_key_listener = cx.listener(move |this, event, window, cx| {
            this.handle_completion_key(session_id, event, window, cx)
        });
        let completion_up_editor = query_editor.clone();
        let completion_down_editor = query_editor.clone();
        let completion_enter_editor = query_editor.clone();
        let mut editor_panel = div()
            .relative()
            .h(px(224.))
            .p(px(10.))
            .border_b_1()
            .border_color(THEME.border)
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
            }))
            .on_action(cx.listener(move |this, _: &CompletionDown, window, cx| {
                this.handle_completion_action(
                    session_id,
                    CompletionAction::Down,
                    completion_down_editor.clone(),
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(move |this, _: &CompletionEnter, window, cx| {
                this.handle_completion_action(
                    session_id,
                    CompletionAction::Enter,
                    completion_enter_editor.clone(),
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(move |this, _: &FormatQuery, window, cx| {
                this.format_query_for(session_id, window, cx);
            }))
            .child(editor::sql_input(query_editor, query_focus, true));
        if let Some(completion_element) = completion_element {
            editor_panel = editor_panel.child(completion_element);
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(38.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(THEME.border)
                    .bg(THEME.panel)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(icon(Icon::Query, THEME.accent))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("SQL editor"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .when(sql_dialect, |actions| {
                                actions.child(
                                    button("format-query", "Format", ButtonKind::Quiet)
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.format_query_for(session_id, window, cx);
                                        })),
                                )
                            })
                            .child(
                                button(
                                    "run-query",
                                    if busy { "Running…" } else { "Run  ⌘↵" },
                                    ButtonKind::Primary,
                                )
                                .cursor_pointer()
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.run_query_for(session_id, cx),
                                )),
                            ),
                    ),
            )
            .child(editor_panel)
            .child(Self::render_query_grid(result_grid, has_result, busy))
            .into_any_element()
    }
}
