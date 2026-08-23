use super::super::*;

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
        let Some((kind, redis_filter_editor, can_mutate, filter_rows, inspector_open)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.editors.filter_editor.clone(),
                    self.editable_table_for(session_id).is_some(),
                    session
                        .filters
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
                    session.inspector_open,
                )
            })
        else {
            return div().into_any_element();
        };
        let has_filter_rows = !filter_rows.is_empty();
        let redis_filter_focus = redis_filter_editor.read(cx).focus_handle();
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
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
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme().text)
                                            .child(if kind.is_sql() {
                                                "Filters"
                                            } else {
                                                "Key pattern"
                                            }),
                                    )
                                    .when(kind.is_sql() && has_filter_rows, |view| {
                                        view.child(badge(
                                            format!("{} active", filter_rows.len()),
                                            theme().text_muted,
                                        ))
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .when(kind.is_sql() && !has_filter_rows, |view| {
                                        view.child(self.small_button(
                                            "add-filter",
                                            "Add filter",
                                            cx.listener(move |this, _, window, cx| {
                                                this.add_filter_for(session_id, window, cx)
                                            }),
                                        ))
                                    })
                                    .when(kind.is_sql() && has_filter_rows, |view| {
                                        view.child(self.small_button(
                                            "clear-filters",
                                            "Clear",
                                            cx.listener(move |this, _, _, cx| {
                                                this.clear_filters_for(session_id, cx)
                                            }),
                                        ))
                                    })
                                    .when(!kind.is_sql() || has_filter_rows, |view| {
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
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.refresh_table_for(session_id, cx)
                                            })),
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
                                                            session_id, window, cx,
                                                        )
                                                    },
                                                )),
                                        )
                                    }),
                            ),
                    )
                    .when(!kind.is_sql(), |view| {
                        view.child(div().min_w_0().child(editor::input(
                            redis_filter_editor,
                            redis_filter_focus,
                            false,
                        )))
                    })
                    .when(kind.is_sql() && !has_filter_rows, |view| {
                        view.child(
                            div()
                                .px(px(8.))
                                .py(px(6.))
                                .text_size(px(11.))
                                .text_color(theme().text_muted)
                                .child(format!(
                                    "Showing up to {TABLE_BROWSE_PAGE_SIZE} rows. Add a filter to narrow this table."
                                )),
                        )
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
                                                            view.h(px(36.))
                                                                .px(px(9.))
                                                                .flex()
                                                                .items_center()
                                                                .rounded(px(6.))
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
                                                .w(px(28.))
                                                .with_size(Size::XSmall)
                                                .compact()
                                                .ghost()
                                                .tooltip("Remove filter")
                                                .child(icon(Icon::Close, theme().text_muted))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.remove_filter_for(session_id, row_id, cx)
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
                                            this.add_filter_for(session_id, window, cx)
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
                    .child(self.render_grid(cx))
                    .when(!self.narrow_workspace && inspector_open, |view| {
                        view.child(self.render_inspector(cx))
                    }),
            )
            .into_any_element()
    }

    fn render_grid(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((result_grid, has_result, busy)) = self.session(session_id).map(|session| {
            (
                session.data_grid.clone(),
                session.result.is_some(),
                session.busy,
            )
        }) else {
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
                    "Select a table to browse rows"
                })
                .into_any_element();
        }

        div()
            .id("grid")
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

    fn render_inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((
            read_only_result,
            can_edit,
            has_selected_row,
            can_save,
            draft_mode,
            draft_fields,
            static_fields,
        )) = self.session(session_id).map(|session| {
            let can_mutate = self.editable_table_for(session_id).is_some();
            let draft_fields = session
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
            let static_fields = session
                .selected_row
                .and_then(|row_index| session.result.as_ref()?.rows.get(row_index))
                .and_then(|row| {
                    session.result.as_ref().map(|result| {
                        result
                            .columns
                            .iter()
                            .enumerate()
                            .map(|(index, column)| {
                                let value = row.values.get(index);
                                (
                                    column.name.clone(),
                                    column.data_type.clone(),
                                    value.map(ToString::to_string).unwrap_or_else(|| "—".into()),
                                    value.is_some_and(|value| matches!(value, CellValue::Null)),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .unwrap_or_default();
            (
                session.result.is_some() && session.result_table.is_none(),
                can_mutate,
                session.selected_row.is_some(),
                can_mutate && session.row_draft.is_some(),
                session.draft_mode,
                draft_fields,
                static_fields,
            )
        })
        else {
            return div().into_any_element();
        };
        let has_draft = !draft_fields.is_empty();
        div()
            .w(px(330.))
            .flex_none()
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
                            .child(badge(
                                match draft_mode {
                                    DraftMode::Insert => "NEW",
                                    DraftMode::Update if has_draft => "EDITING",
                                    DraftMode::Update if can_edit && has_selected_row => "SELECTED",
                                    DraftMode::Update if read_only_result && has_selected_row => {
                                        "READ ONLY"
                                    }
                                    DraftMode::Update => "DETAILS",
                                },
                                if draft_mode == DraftMode::Update && !has_draft {
                                    theme().text_muted
                                } else {
                                    theme().accent
                                },
                            ))
                            .child(
                                Button::new("close-inspector")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Close row inspector")
                                    .child(icon(Icon::Close, theme().text_muted))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.close_inspector_for(session_id, cx)
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
                                    .child("Select a row to inspect all of its fields."),
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
                                        if is_enum {
                                            format!("enum · {data_type}")
                                        } else {
                                            data_type
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
                                    view.child(sql_control).child(
                                        div()
                                            .text_size(px(9.))
                                            .text_color(theme().text_muted)
                                            .child(
                                                "Runs as one database expression, for example NOW().",
                                            ),
                                    )
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
                                                    "Stores SQL NULL"
                                                } else {
                                                    "Database supplies the value"
                                                }),
                                        )
                                    },
                                )
                        },
                    ))
                    .when(!has_draft, |view| {
                        view.children(static_fields.into_iter().map(
                            |(name, data_type, value, is_null)| {
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
                                                    .text_size(px(9.))
                                                    .text_color(theme().text_muted)
                                                    .child(data_type),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(if is_null {
                                                theme().text_muted
                                            } else {
                                                theme().text
                                            })
                                            .child(value),
                                    )
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
                    .gap(px(9.))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(theme().text_muted)
                            .child(if has_draft && draft_mode == DraftMode::Insert {
                                "Default omits the column; SQL runs an expression."
                            } else if has_draft {
                                "Only changed values are written; SQL runs an expression."
                            } else if read_only_result && has_selected_row {
                                "Query results are read-only."
                            } else if has_selected_row {
                                "Review this row before choosing Edit."
                            } else {
                                "Select a row to inspect its values."
                            }),
                    )
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
                                            this.cancel_row_draft_for(session_id, cx)
                                        })),
                                )
                                .child(
                                    button(
                                        "save-row",
                                        if draft_mode == DraftMode::Insert {
                                            "Insert row"
                                        } else {
                                            "Save changes"
                                        },
                                        ButtonKind::Primary,
                                    )
                                    .disabled(!can_save)
                                    .when(can_save, |button| {
                                        button.cursor_pointer().on_click(cx.listener(
                                            move |this, _, window, cx| {
                                                this.save_draft_for(session_id, window, cx)
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
                                        .label("Delete row")
                                        .with_size(Size::Small)
                                        .compact()
                                        .ghost()
                                        .text_color(theme().danger)
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.request_delete_selected_for(session_id, window, cx)
                                        })),
                                )
                                .child(
                                    button("edit-row", "Edit row", ButtonKind::Primary)
                                        .cursor_pointer()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.begin_edit_selected_for(session_id, window, cx)
                                        })),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_structure(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    format!(
                        "{} columns · {} foreign keys",
                        table_columns.len(),
                        foreign_keys.len()
                    )
                },
            ))
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
                    .h(px(34.))
                    .mt(px(10.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme().border_strong)
                    .bg(theme().panel_raised)
                    .text_size(px(9.))
                    .text_color(theme().text_muted)
                    .child("COLUMN")
                    .child("TYPE / CONSTRAINTS"),
            )
            .children(table_columns.iter().map(|column| {
                div()
                    .h(px(34.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme().border)
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(column.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme().text_muted)
                            .child(format!(
                                "{}{}{}",
                                column.data_type,
                                if column.nullable {
                                    " · nullable"
                                } else {
                                    " · required"
                                },
                                if column.primary_key {
                                    " · primary key"
                                } else {
                                    ""
                                }
                            )),
                    )
            }))
            .child(div().mt(px(18.)).child(panel_header(
                "Foreign keys",
                format!("{} constraints", foreign_keys.len()),
            )))
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
                            .child("No foreign-key constraints on this table."),
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
    }
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
