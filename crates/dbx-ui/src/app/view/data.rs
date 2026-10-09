use super::super::*;
use super::{GRID_MIN_WIDTH, INSPECTOR_MIN_WIDTH, PaneResize};
use crate::diagram::display_type;
use crate::popups::DropdownMenu as _;
use gpui_component::menu::PopupMenuItem;

fn row_field_heading(
    field_id: FieldId,
    name: String,
    metadata: String,
    state_control: Option<AnyElement>,
) -> Div {
    let label_selector = SharedString::from(format!("row-field-label-{field_id}"));
    let state_selector = SharedString::from(format!("row-field-state-{field_id}"));
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .debug_selector(move || label_selector.to_string())
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(theme().text)
                        .child(name),
                )
                .child(
                    div()
                        .text_size(px(9.))
                        .text_color(theme().text_muted)
                        .child(metadata),
                ),
        )
        .when_some(state_control, |view, control| {
            view.child(
                div()
                    .debug_selector(move || state_selector.to_string())
                    .w(px(88.))
                    .h(px(20.))
                    .flex_none()
                    .child(control),
            )
        })
}

impl DbxApp {
    pub(super) fn render_data(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some(tab_id) = self
            .session(session_id)
            .and_then(|session| session.active_data_tab_id())
        else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme().text_muted)
                .child("Select a table to browse rows")
                .into_any_element();
        };
        let Some((
            kind,
            redis_filter_editor,
            can_mutate,
            filter_rows,
            inspector_open,
            column_names,
            hidden_columns,
            saved_filters,
        )) = self
            .session(session_id)
            .zip(self.data_tab(session_id, tab_id))
            .map(|(session, data)| {
                (
                    session.kind,
                    session.editors.filter_editor.clone(),
                    self.editable_table_for(session_id, tab_id).is_some(),
                    data.filters
                        .rows()
                        .iter()
                        .map(|row| {
                            (
                                row.id,
                                row.column_selector.clone(),
                                row.operator_selector.clone(),
                                row.operator,
                                row.editor.clone(),
                            )
                        })
                        .collect::<Vec<_>>(),
                    data.inspector_open,
                    data.result
                        .as_ref()
                        .map(|result| {
                            result
                                .columns
                                .iter()
                                .map(|column| column.name.clone())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default(),
                    data.layout.hidden.clone(),
                    data.layout
                        .saved_filters
                        .iter()
                        .map(|saved| saved.name.clone())
                        .collect::<Vec<_>>(),
                )
            })
        else {
            return div().into_any_element();
        };
        let has_filter_rows = !filter_rows.is_empty();
        let app = cx.entity().downgrade();
        let columns_app = app.clone();
        let saved_app = app.clone();
        let redis_filter_focus = redis_filter_editor.read(cx).focus_handle();
        div()
            .key_context("DbxDataTab")
            .on_action(cx.listener(move |this, _: &CommitChanges, window, cx| {
                this.request_commit_changes_for(session_id, tab_id, window, cx)
            }))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .key_context("DbxFilters")
                    .on_action(cx.listener(move |this, _: &ApplyFilters, _, cx| {
                        this.refresh_table_for(session_id, tab_id, cx)
                    }))
                    .px(px(8.))
                    .py(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .border_b_1()
                    .border_color(theme().border)
                    .bg(theme().panel)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div().flex().items_center().gap(px(7.)).child(
                                    div()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme().text)
                                        .child(if kind.is_sql() {
                                            "Filters"
                                        } else {
                                            if kind == DatabaseKind::Redis {
                                                "Key pattern"
                                            } else {
                                                "Records"
                                            }
                                        }),
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .when(kind.is_sql() && !has_filter_rows, |view| {
                                        view.child(
                                            button("add-filter", "Add filter", ButtonKind::Quiet)
                                                .cursor_pointer()
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.add_filter_for(
                                                            session_id, tab_id, window, cx,
                                                        )
                                                    },
                                                )),
                                        )
                                    })
                                    .when(kind.is_sql() && has_filter_rows, |view| {
                                        view.child(
                                            button("clear-filters", "Clear", ButtonKind::Quiet)
                                                .cursor_pointer()
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.clear_filters_for(session_id, tab_id, cx)
                                                })),
                                        )
                                    })
                                    .when(
                                        kind.is_sql()
                                            && (has_filter_rows || !saved_filters.is_empty()),
                                        move |view| {
                                            view.child(
                                                button(
                                                    "saved-filters",
                                                    "Saved",
                                                    ButtonKind::Quiet,
                                                )
                                                .cursor_pointer()
                                                .dropdown_menu(move |mut menu, _, _| {
                                                    for (index, name) in
                                                        saved_filters.iter().enumerate()
                                                    {
                                                        let app = saved_app.clone();
                                                        menu = menu.item(
                                                            PopupMenuItem::new(name.clone())
                                                                .on_click(move |_, window, cx| {
                                                                    let _ = app.update(cx, |this, cx| {
                                                                        this.apply_saved_filter_for(
                                                                            session_id, tab_id,
                                                                            index, window, cx,
                                                                        )
                                                                    });
                                                                }),
                                                        );
                                                    }
                                                    if !saved_filters.is_empty() {
                                                        menu = menu.separator();
                                                    }
                                                    let app = saved_app.clone();
                                                    menu = menu.item(
                                                        PopupMenuItem::new("Save current filters")
                                                            .disabled(!has_filter_rows)
                                                            .on_click(move |_, _, cx| {
                                                                let _ = app.update(cx, |this, cx| {
                                                                    this.save_current_filters_for(
                                                                        session_id, tab_id, cx,
                                                                    )
                                                                });
                                                            }),
                                                    );
                                                    for (index, name) in
                                                        saved_filters.iter().enumerate()
                                                    {
                                                        let app = saved_app.clone();
                                                        menu = menu.item(
                                                            PopupMenuItem::new(format!(
                                                                "Delete “{name}”"
                                                            ))
                                                            .on_click(move |_, _, cx| {
                                                                let _ = app.update(cx, |this, cx| {
                                                                    this.delete_saved_filter_for(
                                                                        session_id, tab_id, index,
                                                                        cx,
                                                                    )
                                                                });
                                                            }),
                                                        );
                                                    }
                                                    menu
                                                }),
                                            )
                                        },
                                    )
                                    .when(kind.is_sql() && !column_names.is_empty(), move |view| {
                                        view.child(
                                            button("table-columns", "Columns", ButtonKind::Quiet)
                                                .cursor_pointer()
                                                .dropdown_menu(move |mut menu, _, _| {
                                                    let visible = column_names.len()
                                                        - hidden_columns.len().min(column_names.len());
                                                    for name in &column_names {
                                                        let shown = !hidden_columns.contains(name);
                                                        let app = columns_app.clone();
                                                        let toggled = name.clone();
                                                        menu = menu.item(
                                                            PopupMenuItem::new(name.clone())
                                                                .checked(shown)
                                                                // Keep at least one column.
                                                                .disabled(shown && visible < 2)
                                                                .on_click(move |_, _, cx| {
                                                                    let _ = app.update(cx, |this, cx| {
                                                                        this.toggle_column_for(
                                                                            session_id,
                                                                            tab_id,
                                                                            toggled.clone(),
                                                                            cx,
                                                                        )
                                                                    });
                                                                }),
                                                        );
                                                    }
                                                    let app = columns_app.clone();
                                                    menu.separator().item(
                                                        PopupMenuItem::new("Reset layout").on_click(
                                                            move |_, _, cx| {
                                                                let _ = app.update(cx, |this, cx| {
                                                                    this.reset_table_layout_for(
                                                                        session_id, tab_id, cx,
                                                                    )
                                                                });
                                                            },
                                                        ),
                                                    )
                                                }),
                                        )
                                    })
                                    .when(kind == DatabaseKind::Redis || has_filter_rows, |view| {
                                        view.child(
                                            button(
                                                "apply-filter",
                                                if kind.is_sql() {
                                                    "Apply filters"
                                                } else {
                                                    "Apply"
                                                },
                                                ButtonKind::Primary,
                                            )
                                            .cursor_pointer()
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    this.refresh_table_for(session_id, tab_id, cx)
                                                }),
                                            ),
                                        )
                                    })
                                    .when(can_mutate, |view| {
                                        view.child(
                                            div()
                                                .mx(px(2.))
                                                .h(px(18.))
                                                .border_l_1()
                                                .border_color(theme().border),
                                        )
                                        .child(
                                            button("add-row", "New row", ButtonKind::Quiet)
                                                .cursor_pointer()
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.begin_insert_for(
                                                            session_id, tab_id, window, cx,
                                                        )
                                                    },
                                                )),
                                        )
                                    }),
                            ),
                    )
                    .when(kind == DatabaseKind::Redis, |view| {
                        view.child(div().min_w_0().child(editor::input(
                            redis_filter_editor,
                            redis_filter_focus,
                            false,
                        )))
                    })
                    .when(kind.is_sql() && has_filter_rows, |view| {
                        view.child(
                            div()
                                .id("filter-rows-scroll")
                                .w_full()
                                .min_w_0()
                                .max_h(px(132.))
                                .overflow_y_scroll()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .children(filter_rows.into_iter().map(
                                    |(
                                        row_id,
                                        column_selector,
                                        operator_selector,
                                        operator,
                                        value_editor,
                                    )| {
                                        let value_focus = value_editor.read(cx).focus_handle();
                                        div()
                                            .id(SharedString::from(format!("filter-row-{row_id}")))
                                            .w_full()
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .gap(px(7.))
                                            .child(
                                                div().flex_1().min_w_0().max_w(px(240.)).child(
                                                    Select::new(&column_selector)
                                                        .with_size(Size::Small)
                                                        .h(px(FILTER_CONTROL_HEIGHT))
                                                        .rounded(px(5.))
                                                        .w_full()
                                                        .menu_max_h(px(220.))
                                                        .placeholder("Column")
                                                        .text_size(px(11.))
                                                        .bg(theme().panel_raised)
                                                        .border_color(theme().border_strong)
                                                        .text_color(theme().text),
                                                ),
                                            )
                                            .child(
                                                div().flex_1().min_w_0().max_w(px(220.)).child(
                                                    Select::new(&operator_selector)
                                                        .with_size(Size::Small)
                                                        .h(px(FILTER_CONTROL_HEIGHT))
                                                        .rounded(px(5.))
                                                        .w_full()
                                                        .menu_max_h(px(220.))
                                                        .text_size(px(11.))
                                                        .bg(theme().panel_raised)
                                                        .border_color(theme().border_strong)
                                                        .text_color(theme().text),
                                                ),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .when(
                                                        operator_requires_value(operator),
                                                        |view| {
                                                            view.child(editor::input(
                                                                value_editor,
                                                                value_focus,
                                                                false,
                                                            ))
                                                        },
                                                    )
                                                    .when(
                                                        !operator_requires_value(operator),
                                                        |view| {
                                                            view.h(px(FILTER_CONTROL_HEIGHT))
                                                                .px(px(9.))
                                                                .flex()
                                                                .items_center()
                                                                .rounded(px(5.))
                                                                .bg(theme().panel_raised)
                                                                .text_color(theme().text_muted)
                                                                .child("No value")
                                                        },
                                                    ),
                                            )
                                            .child(
                                                Button::new(SharedString::from(format!(
                                                    "remove-filter-{row_id}"
                                                )))
                                                .flex_none()
                                                .size(px(FILTER_CONTROL_HEIGHT))
                                                .with_size(Size::XSmall)
                                                .compact()
                                                .ghost()
                                                .rounded(px(5.))
                                                .tooltip("Remove filter")
                                                .child(icon(Icon::Close, theme().text_muted))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.remove_filter_for(
                                                        session_id, tab_id, row_id, cx,
                                                    )
                                                })),
                                            )
                                    },
                                ))
                                .child(
                                    Button::new("add-filter-inline")
                                        .label("Add condition")
                                        .with_size(Size::XSmall)
                                        .compact()
                                        .ghost()
                                        .text_color(theme().accent)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.add_filter_for(session_id, tab_id, window, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.render_grid(session_id, tab_id, cx))
                    .when(!self.narrow_workspace && inspector_open, |view| {
                        view.child(self.render_inspector(session_id, tab_id, cx))
                    }),
            )
            .into_any_element()
    }

    fn render_grid(
        &self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((result_grid, has_result, busy, counts)) =
            self.data_tab(session_id, tab_id).map(|data| {
                (
                    data.data_grid.clone(),
                    data.result.is_some(),
                    data.busy,
                    data.change_counts(),
                )
            })
        else {
            return div().into_any_element();
        };

        if !has_result {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme().text_muted)
                .child(if busy {
                    "Loading rows…"
                } else {
                    "No rows loaded"
                })
                .into_any_element();
        }

        let recovered = self
            .data_tab(session_id, tab_id)
            .is_some_and(|data| data.recovered_changeset.is_some());
        let summary = [
            (counts.edited, "edited"),
            (counts.inserted, "new"),
            (counts.deleted, "deleted"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect::<Vec<_>>()
        .join(" · ");
        div()
            .id("grid")
            .key_context("DbxDataGrid")
            .flex_1()
            .min_w(px(GRID_MIN_WIDTH))
            .min_h_0()
            .flex()
            .flex_col()
            .on_action(cx.listener(move |this, _: &CommitCellEdit, window, cx| {
                this.commit_cell_edit_for(session_id, tab_id, None, window, cx)
            }))
            .on_action(
                cx.listener(move |this, _: &CommitCellEditNext, window, cx| {
                    this.commit_cell_edit_for(session_id, tab_id, Some(true), window, cx)
                }),
            )
            .on_action(
                cx.listener(move |this, _: &CommitCellEditPrevious, window, cx| {
                    this.commit_cell_edit_for(session_id, tab_id, Some(false), window, cx)
                }),
            )
            .on_action(cx.listener(move |this, _: &CancelCellEdit, window, cx| {
                this.cancel_cell_edit_for(session_id, tab_id, window, cx)
            }))
            .on_action(cx.listener(move |this, _: &SetCellNull, window, cx| {
                this.set_cell_null_for(session_id, tab_id, window, cx)
            }))
            .when_some(
                self.data_tab(session_id, tab_id)
                    .and_then(|data| data.cell_editor.as_ref())
                    .filter(|cell| cell.structured)
                    .map(|cell| cell.editor.clone()),
                |view, editor| {
                    view.child(
                        div()
                            .flex_none()
                            .h(px(260.))
                            .p(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div().flex().justify_between().child("JSON editor").child(
                                    div()
                                        .flex()
                                        .gap(px(6.))
                                        .child(
                                            button("cancel-json-edit", "Cancel", ButtonKind::Quiet)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.cancel_cell_edit_for(
                                                            session_id, tab_id, window, cx,
                                                        )
                                                    },
                                                )),
                                        )
                                        .child(
                                            button(
                                                "stage-json-edit",
                                                "Stage change",
                                                ButtonKind::Primary,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, window, cx| {
                                                    this.commit_cell_edit_for(
                                                        session_id, tab_id, None, window, cx,
                                                    )
                                                }),
                                            ),
                                        ),
                                ),
                            )
                            .child(div().flex_1().min_h_0().child(editor)),
                    )
                },
            )
            .on_action(cx.listener(move |this, _: &DeleteRows, _, cx| {
                this.delete_rows_for(session_id, tab_id, None, cx)
            }))
            .on_action(cx.listener(move |this, _: &CopyDataSelection, _, cx| {
                this.copy_data_selection_for(session_id, tab_id, cx)
            }))
            .on_action(cx.listener(move |this, _: &PasteRows, _, cx| {
                this.paste_rows_for(session_id, tab_id, cx)
            }))
            .when(counts.total() > 0, |view| {
                view.child(
                    div()
                        .flex_none()
                        .px(px(8.))
                        .py(px(5.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .border_b_1()
                        .border_color(theme().border)
                        .bg(theme().panel)
                        .child(div().text_size(px(11.)).text_color(theme().text).child(
                            if recovered {
                                format!("Recovered draft · {summary} · review required")
                            } else {
                                summary
                            },
                        ))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    button("discard-cell-edits", "Discard", ButtonKind::Quiet)
                                        .cursor_pointer()
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.discard_pending_edits_for(session_id, tab_id, cx)
                                        })),
                                )
                                .child(
                                    button("review-cell-edits", "Review SQL", ButtonKind::Quiet)
                                        .cursor_pointer()
                                        .disabled(busy)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.review_changes_for(session_id, tab_id, window, cx)
                                        })),
                                )
                                .child(
                                    button("save-cell-edits", "Commit", ButtonKind::Primary)
                                        .cursor_pointer()
                                        .disabled(busy)
                                        .tooltip(if cfg!(target_os = "macos") {
                                            "⌘S"
                                        } else {
                                            "Ctrl+S"
                                        })
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.request_commit_changes_for(
                                                session_id, tab_id, window, cx,
                                            )
                                        })),
                                ),
                        ),
                )
            })
            .child(
                div().flex_1().min_h_0().child(
                    DataTable::new(&result_grid)
                        .with_size(px(30.))
                        .stripe(false)
                        .bordered(false)
                        .scrollbar_visible(true, true),
                ),
            )
            .into_any_element()
    }

    fn render_inspector(
        &mut self,
        session_id: SessionId,
        tab_id: SecondaryTabId,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some((
            read_only_result,
            can_edit,
            has_selected_row,
            can_save,
            draft_mode,
            draft_fields,
            static_fields,
            row_deleted,
        )) = self.data_tab(session_id, tab_id).map(|data| {
            let can_mutate = self.editable_table_for(session_id, tab_id).is_some();
            let draft_fields = data
                .row_draft
                .as_ref()
                .map(|draft| {
                    draft
                        .fields()
                        .iter()
                        .map(|field| {
                            (
                                field.id,
                                field.column.name.clone(),
                                field.column.data_type.clone(),
                                field.column.nullable,
                                field.column.primary_key,
                                field.state,
                                field.editor.clone(),
                                field.sql_editor.clone(),
                                field.enum_selector.clone(),
                                field.boolean_selector.clone(),
                                field.state_selector.clone(),
                                field.value_kind() == FieldValueKind::Json,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let static_fields = data
                .selected_row
                .and_then(|row| {
                    data.result.as_ref().map(|result| {
                        result
                            .columns
                            .iter()
                            .enumerate()
                            .map(|(index, column)| {
                                (
                                    column.name.clone(),
                                    column.data_type.clone(),
                                    data.shown_value(row, index),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default();
            let row_deleted = data
                .selected_row
                .is_some_and(|row| data.pending_deletes.contains(&row));
            (
                data.result.is_some() && data.result_table.is_none(),
                can_mutate,
                data.selected_row.is_some(),
                can_mutate && data.row_draft.is_some(),
                data.draft_mode,
                draft_fields,
                static_fields,
                row_deleted,
            )
        })
        else {
            return div().into_any_element();
        };
        let has_draft = !draft_fields.is_empty();
        div()
            .relative()
            .w(px(self.inspector_width()))
            // Give way to the grid when the window narrows after a drag.
            .flex_shrink_1()
            .min_w(px(INSPECTOR_MIN_WIDTH))
            .flex()
            .flex_col()
            .min_h_0()
            .border_l_1()
            .border_color(theme().border)
            .bg(theme().panel)
            .child(
                div()
                    .px(px(14.))
                    .pt(px(14.))
                    .pb(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme().text)
                            .child(match draft_mode {
                                DraftMode::Insert => "New row",
                                DraftMode::Update if has_draft => "Edit row",
                                DraftMode::Update => "Row details",
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            // The title already says new/edit/details; only
                            // read-only adds information.
                            .when(
                                draft_mode == DraftMode::Update
                                    && !has_draft
                                    && read_only_result
                                    && has_selected_row,
                                |view| view.child(badge("Read-only", theme().text_muted)),
                            )
                            .child(
                                Button::new("close-inspector")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Close")
                                    .child(icon(Icon::Close, theme().text_muted))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.close_inspector_for(session_id, tab_id, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .id("row-fields-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(
                        !has_draft && static_fields.is_empty() && draft_mode == DraftMode::Update,
                        |view| {
                            view.child(
                                div()
                                    .px(px(5.))
                                    .py(px(12.))
                                    .text_size(px(12.))
                                    .text_color(theme().text_muted)
                                    .child("No row selected"),
                            )
                        },
                    )
                    .children(draft_fields.into_iter().map(
                        |(
                            field_id,
                            name,
                            data_type,
                            nullable,
                            primary_key,
                            state,
                            field_editor,
                            sql_editor,
                            enum_selector,
                            boolean_selector,
                            state_selector,
                            is_json,
                        )| {
                            let field_focus = field_editor.read(cx).focus_handle();
                            let sql_focus = sql_editor.read(cx).focus_handle();
                            let is_enum = enum_selector.is_some();
                            let value_control = boolean_selector
                                .or(enum_selector)
                                .map(|selector| {
                                    div()
                                        .w_full()
                                        .h(px(32.))
                                        .flex_none()
                                        .child(
                                            Select::new(&selector)
                                                .with_size(Size::Medium)
                                                .w_full()
                                                .menu_max_h(px(220.))
                                                .text_size(px(11.))
                                                .bg(theme().canvas)
                                                .border_color(theme().border_strong)
                                                .text_color(theme().text),
                                        )
                                        .into_any_element()
                                })
                                .unwrap_or_else(|| {
                                    editor::input(field_editor, field_focus, is_json)
                                        .into_any_element()
                                });
                            let sql_control =
                                editor::input(sql_editor, sql_focus, false).into_any_element();
                            let state_control = state_selector.map(|selector| {
                                Select::new(&selector)
                                    .with_size(Size::XSmall)
                                    .w_full()
                                    .menu_max_h(px(132.))
                                    .text_size(px(10.))
                                    .bg(theme().panel_raised)
                                    .border_color(theme().border)
                                    .text_color(theme().text)
                                    .into_any_element()
                            });
                            div()
                                .id(SharedString::from(format!("row-field-{field_id}")))
                                .px(px(9.))
                                .py(px(8.))
                                .border_b_1()
                                .border_color(theme().border)
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(row_field_heading(
                                    field_id,
                                    name,
                                    format!(
                                        "{} · {}{}",
                                        // MySQL spells enums with their labels,
                                        // which the selector already lists.
                                        if is_enum
                                            && data_type.to_ascii_lowercase().starts_with("enum(")
                                        {
                                            "enum".to_owned()
                                        } else if is_enum {
                                            format!("enum · {data_type}")
                                        } else {
                                            display_type(&data_type)
                                        },
                                        if nullable { "nullable" } else { "required" },
                                        if primary_key { " · primary key" } else { "" }
                                    ),
                                    state_control,
                                ))
                                .when(state == FieldValueState::Value, |view| {
                                    view.child(value_control)
                                })
                                .when(state == FieldValueState::Sql, |view| {
                                    view.child(sql_control)
                                })
                                .when(
                                    matches!(
                                        state,
                                        FieldValueState::Null | FieldValueState::Default
                                    ),
                                    |view| {
                                        view.child(
                                            div()
                                                .h(px(32.))
                                                .px(px(9.))
                                                .flex()
                                                .items_center()
                                                .rounded(px(6.))
                                                .bg(theme().panel_raised)
                                                .text_size(px(11.))
                                                .text_color(theme().text_muted)
                                                .child(if state == FieldValueState::Null {
                                                    "NULL"
                                                } else {
                                                    "Default"
                                                }),
                                        )
                                    },
                                )
                        },
                    ))
                    .when(!has_draft, |view| {
                        view.children(static_fields.into_iter().enumerate().map(
                            |(index, (name, data_type, value))| {
                                let copy_text = value
                                    .as_ref()
                                    .map(crate::app::value_view::value_clipboard_text);
                                div()
                                    .px(px(9.))
                                    .py(px(9.))
                                    .border_b_1()
                                    .border_color(theme().border)
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .child(div().text_size(px(11.)).child(name))
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .gap(px(4.))
                                                    .child(
                                                        div()
                                                            .text_size(px(9.))
                                                            .text_color(theme().text_muted)
                                                            .child(display_type(&data_type)),
                                                    )
                                                    .when_some(copy_text, |view, text| {
                                                        view.child(
                                                            Button::new(SharedString::from(
                                                                format!("inspector-copy-{index}"),
                                                            ))
                                                            .label("Copy")
                                                            .with_size(Size::XSmall)
                                                            .compact()
                                                            .ghost()
                                                            .on_click(move |_, _, cx| {
                                                                cx.write_to_clipboard(
                                                                    ClipboardItem::new_string(
                                                                        text.clone(),
                                                                    ),
                                                                );
                                                            }),
                                                        )
                                                    }),
                                            ),
                                    )
                                    .child(crate::app::value_view::value_view(value.as_ref()))
                            },
                        ))
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .p(px(12.))
                    .border_t_1()
                    .border_color(theme().border)
                    .flex()
                    .flex_col()
                    // Only show the action bar when there is something to act on.
                    .when(!(has_draft || (has_selected_row && can_edit)), |view| {
                        view.hidden()
                    })
                    .gap(px(9.))
                    .when(has_draft, |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    button("cancel-row-draft", "Cancel", ButtonKind::Quiet)
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.cancel_row_draft_for(session_id, tab_id, cx)
                                        })),
                                )
                                .child(
                                    button(
                                        "save-row",
                                        if draft_mode == DraftMode::Insert {
                                            "Stage row"
                                        } else {
                                            "Stage changes"
                                        },
                                        ButtonKind::Primary,
                                    )
                                    .disabled(!can_save)
                                    .when(can_save, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            move |this, _, window, cx| {
                                                this.save_draft_for(session_id, tab_id, window, cx)
                                            },
                                        ))
                                    }),
                                ),
                        )
                    })
                    .when(!has_draft && has_selected_row && can_edit, |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    Button::new("delete-row")
                                        .label(if row_deleted {
                                            "Restore row"
                                        } else {
                                            "Delete row"
                                        })
                                        .with_size(Size::XSmall)
                                        .compact()
                                        .ghost()
                                        .text_color(theme().danger)
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.request_delete_selected_for(
                                                session_id, tab_id, window, cx,
                                            )
                                        })),
                                )
                                .when(!row_deleted, |actions| {
                                    actions.child(
                                        button("edit-row", "Edit row", ButtonKind::Primary)
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.begin_edit_selected_for(
                                                    session_id, tab_id, window, cx,
                                                )
                                            })),
                                    )
                                }),
                        )
                    }),
            )
            .child(self.pane_resize_handle(PaneResize::Inspector, cx))
            .on_drag_move(
                cx.listener(|this, event, _, cx| {
                    this.resize_pane(PaneResize::Inspector, event, cx)
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_structure(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (indexes, checks, definition) = self
            .active_session()
            .and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Structure(structure) = &tab.kind else {
                    return None;
                };
                Some((
                    structure.indexes.clone(),
                    structure.checks.clone(),
                    structure.definition.clone(),
                ))
            })
            .unwrap_or_default();
        let (session_id, table_name, table_columns, foreign_keys, tables, busy, error) = self
            .active_session()
            .and_then(|session| {
                let tab_id = session.active_secondary_tab?;
                let tab = session.secondary_tabs.iter().find(|tab| tab.id == tab_id)?;
                let SecondaryTabKind::Structure(structure) = &tab.kind else {
                    return None;
                };
                Some((
                    session.id,
                    table_ref_label(&structure.table),
                    structure.columns.clone(),
                    structure.foreign_keys.clone(),
                    session.tables.clone(),
                    structure.busy,
                    structure.error.clone(),
                ))
            })
            .unwrap_or_else(|| {
                (
                    Uuid::nil(),
                    "Structure".into(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                    None,
                )
            });
        let object_table = self
            .session(session_id)
            .and_then(|session| {
                session
                    .secondary_tabs
                    .iter()
                    .find(|tab| Some(tab.id) == session.active_secondary_tab)
            })
            .and_then(|tab| match &tab.kind {
                SecondaryTabKind::Structure(structure) => Some(structure.table.clone()),
                _ => None,
            });
        let objects = self.render_schema_objects_for(session_id, object_table.as_ref(), cx);
        let designer = self.render_designer_for(session_id, cx);
        div()
            .id("structure-scroll")
            .flex_1()
            .overflow_y_scroll()
            .p(px(12.))
            .flex()
            .flex_col()
            .child(panel_header(
                table_name,
                if busy {
                    "Loading metadata…".into()
                } else {
                    String::new()
                },
            ))
            .child(
                button("open-table-designer", "Design table…", ButtonKind::Quiet).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.open_designer_for(session_id, window, cx)
                    }),
                ),
            )
            .when_some(designer, |view, designer| view.child(designer))
            .child(objects)
            .when(error.is_some(), |view| {
                view.child(
                    div()
                        .mt(px(8.))
                        .text_color(theme().danger)
                        .child(error.clone().unwrap_or_default()),
                )
            })
            .child(
                div()
                    .mt(px(12.))
                    .rounded(px(RADIUS_PANEL))
                    .border_1()
                    .border_color(theme().border)
                    .overflow_hidden()
                    .child(
                        structure_row()
                            .h(px(30.))
                            .follow_top_corners(RADIUS_PANEL)
                            .bg(theme().panel_raised)
                            .border_b_1()
                            .border_color(theme().border_strong)
                            .text_size(px(10.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme().text_muted)
                            .child(structure_cell(STRUCTURE_ORDINAL_WIDTH).child("#"))
                            .child(structure_cell(STRUCTURE_KEY_WIDTH).child("Key"))
                            .child(div().flex_1().min_w_0().child("Column"))
                            .child(div().flex_1().min_w_0().child("Type"))
                            .child(structure_cell(STRUCTURE_NULL_WIDTH).child("Nullable"))
                            .child(div().flex_1().min_w_0().child("Default"))
                            .child(div().flex_1().min_w_0().child("References")),
                    )
                    .children(table_columns.iter().enumerate().map(|(index, column)| {
                        let reference = foreign_keys.iter().find_map(|foreign_key| {
                            let position = foreign_key
                                .columns
                                .iter()
                                .position(|name| *name == column.name)?;
                            let target_column = foreign_key.referenced_columns.get(position)?;
                            Some(format!(
                                "{}.{target_column}",
                                foreign_key
                                    .referenced_schema
                                    .as_ref()
                                    .map(|schema| format!(
                                        "{schema}.{}",
                                        foreign_key.referenced_table
                                    ))
                                    .unwrap_or_else(|| foreign_key.referenced_table.clone())
                            ))
                        });
                        let foreign = reference.is_some();
                        structure_row()
                            .id(SharedString::from(format!("structure-column-{index}")))
                            .h(px(32.))
                            .text_size(px(12.))
                            .when(index > 0, |row| {
                                row.border_t_1().border_color(theme().border)
                            })
                            .when(index + 1 == table_columns.len(), |row| {
                                row.follow_bottom_corners(RADIUS_PANEL)
                            })
                            .when(index % 2 == 1, |row| row.bg(theme().grid_alternate))
                            .hover(|style| style.bg(theme().glass_hover))
                            .child(
                                structure_cell(STRUCTURE_ORDINAL_WIDTH)
                                    .text_size(px(10.))
                                    .text_color(theme().text_muted)
                                    .child(format!("{}", index + 1)),
                            )
                            .child(
                                structure_cell(STRUCTURE_KEY_WIDTH)
                                    .flex()
                                    .gap(px(3.))
                                    .when(column.primary_key, |cell| {
                                        cell.child(key_badge("PK", theme().warning))
                                    })
                                    .when(foreign, |cell| {
                                        cell.child(key_badge("FK", theme().focus_ring))
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme().text)
                                    .when(column.primary_key, |cell| {
                                        cell.font_weight(FontWeight::SEMIBOLD)
                                    })
                                    .child(column.name.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme().sql_type)
                                    .child(display_type(&column.data_type)),
                            )
                            .child(
                                structure_cell(STRUCTURE_NULL_WIDTH)
                                    .text_color(if column.nullable {
                                        theme().text_muted
                                    } else {
                                        theme().text
                                    })
                                    .child(if column.nullable { "NULL" } else { "NOT NULL" }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family("monospace")
                                    .text_size(px(11.))
                                    .text_color(theme().text_muted)
                                    .when_some(column.default_value.clone(), |cell, default| {
                                        cell.child(default)
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme().text_muted)
                                    .when_some(reference, |cell, reference| {
                                        cell.child(format!("→ {reference}"))
                                    }),
                            )
                    })),
            )
            .child(div().mt(px(18.)).child(panel_header("Foreign keys", "")))
            .when(
                foreign_keys.is_empty() && !busy && error.is_none(),
                |view| {
                    view.child(
                        div()
                            .mt(px(8.))
                            .px(px(10.))
                            .py(px(12.))
                            .border_1()
                            .border_color(theme().border)
                            .rounded(px(6.))
                            .text_size(px(11.))
                            .text_color(theme().text_muted)
                            .child("No foreign keys"),
                    )
                },
            )
            .children(foreign_keys.iter().enumerate().map(|(index, foreign_key)| {
                let source = foreign_key.columns.join(", ");
                let target_table = match &foreign_key.referenced_schema {
                    Some(schema) => format!("{schema}.{}", foreign_key.referenced_table),
                    None => foreign_key.referenced_table.clone(),
                };
                let target = format!(
                    "{} ({})",
                    target_table,
                    foreign_key.referenced_columns.join(", ")
                );
                let actions = foreign_key_actions(foreign_key);
                let can_navigate = foreign_key_target_table(&tables, foreign_key).is_some();
                let foreign_key = foreign_key.clone();
                div()
                    .min_h(px(44.))
                    .px(px(10.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .border_b_1()
                    .border_color(theme().border)
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        foreign_key
                                            .constraint_name
                                            .clone()
                                            .unwrap_or_else(|| "Unnamed constraint".into()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(theme().text_muted)
                                    .child(source),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "foreign-key-target-{session_id}-{index}"
                            )))
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap(px(3.))
                            .when(can_navigate, |view| {
                                view.cursor_pointer()
                                    .hover(|style| style.text_color(theme().text))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.navigate_to_foreign_key_for(
                                            session_id,
                                            foreign_key.clone(),
                                            window,
                                            cx,
                                        )
                                    }))
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(if can_navigate {
                                                theme().accent
                                            } else {
                                                theme().text_muted
                                            })
                                            .child(format!("REFERENCES {target}")),
                                    )
                                    .when(can_navigate, |view| {
                                        view.child(icon(Icon::ArrowRight, theme().accent))
                                    }),
                            )
                            .when(!actions.is_empty(), |view| {
                                view.child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(theme().text_muted)
                                        .child(actions),
                                )
                            }),
                    )
            }))
            .when(!indexes.is_empty(), |view| {
                view.child(div().mt(px(18.)).child(panel_header("Indexes", "")))
                    .children(indexes.into_iter().map(|index| {
                        let mut traits = Vec::new();
                        if index.primary {
                            traits.push("PRIMARY".to_owned());
                        } else if index.unique {
                            traits.push("UNIQUE".to_owned());
                        }
                        if let Some(method) = index.method {
                            traits.push(method.to_ascii_uppercase());
                        }
                        structure_detail_row(
                            index.name,
                            index.columns.join(", "),
                            traits.join(" · "),
                            index
                                .predicate
                                .map(|predicate| format!("WHERE {predicate}")),
                        )
                    }))
            })
            .when(!checks.is_empty(), |view| {
                view.child(
                    div()
                        .mt(px(18.))
                        .child(panel_header("Check constraints", "")),
                )
                .children(checks.into_iter().map(|check| {
                    structure_detail_row(
                        check.name.unwrap_or_else(|| "Unnamed constraint".into()),
                        format!("CHECK ({})", check.expression),
                        String::new(),
                        None,
                    )
                }))
            })
            .when_some(definition, |view, definition| {
                view.child(div().mt(px(18.)).child(panel_header("Definition", "")))
                    .child(
                        div()
                            .mt(px(8.))
                            .p(px(10.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme().border)
                            .bg(theme().panel_raised)
                            .font_family("monospace")
                            .text_size(px(11.))
                            .text_color(theme().text)
                            .whitespace_normal()
                            .child(definition),
                    )
            })
    }
}

/// One named index or constraint: its name and traits on top, the indexed
/// parts or expression below.
fn structure_detail_row(
    name: String,
    body: String,
    traits: String,
    predicate: Option<String>,
) -> Div {
    div()
        .min_h(px(44.))
        .px(px(10.))
        .py(px(7.))
        .flex()
        .flex_col()
        .gap(px(3.))
        .border_b_1()
        .border_color(theme().border)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(name),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(9.))
                        .text_color(theme().text_muted)
                        .child(traits),
                ),
        )
        .child(
            div()
                .font_family("monospace")
                .text_size(px(10.))
                .text_color(theme().text_muted)
                .child(body),
        )
        .when_some(predicate, |row, predicate| {
            row.child(
                div()
                    .font_family("monospace")
                    .text_size(px(10.))
                    .text_color(theme().text_muted)
                    .child(predicate),
            )
        })
}

/// Every control in a filter row matches the single-line editor height.
const FILTER_CONTROL_HEIGHT: f32 = 32.0;

const STRUCTURE_ORDINAL_WIDTH: f32 = 32.0;
const STRUCTURE_KEY_WIDTH: f32 = 56.0;
const STRUCTURE_NULL_WIDTH: f32 = 84.0;

fn structure_row() -> Div {
    div().px(px(12.)).flex().items_center().gap(px(12.))
}

fn structure_cell(width: f32) -> Div {
    div().w(px(width)).flex_none().flex().items_center()
}

/// A compact PK/FK marker shared by the structure grid and diagram cards.
fn key_badge(label: &'static str, color: Rgba) -> Div {
    div()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .bg(color.alpha(0.14))
        .text_size(px(9.))
        .font_weight(FontWeight::BOLD)
        .text_color(color)
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};
    use gpui_component::{
        IndexPath,
        select::{SearchableVec, SelectState},
    };

    struct RowFieldHeadingHarness {
        selector: Entity<SelectState<SearchableVec<SharedString>>>,
    }

    impl Render for RowFieldHeadingHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(312.)).child(row_field_heading(
                1,
                "id".into(),
                "bigint · required · primary key".into(),
                Some(
                    Select::new(&self.selector)
                        .with_size(Size::XSmall)
                        .w_full()
                        .into_any_element(),
                ),
            ))
        }
    }

    #[gpui::test]
    fn row_field_heading_keeps_label_readable_beside_state_select(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let items = SearchableVec::new(vec![
                SharedString::from("Value"),
                SharedString::from("NULL"),
                SharedString::from("Default"),
            ]);
            let selector = cx.new(|select_cx| {
                SelectState::new(items, Some(IndexPath::new(2)), window, select_cx)
            });
            RowFieldHeadingHarness { selector }
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));

        let label = cx
            .debug_bounds("row-field-label-1")
            .expect("field label should be rendered");
        let state = cx
            .debug_bounds("row-field-state-1")
            .expect("field state select should be rendered");
        assert!(
            label.size.width >= px(180.),
            "field label collapsed to {:?}",
            label.size.width
        );
        assert_eq!(state.size.width, px(88.));
        assert_eq!(state.size.height, px(20.));
    }
}
