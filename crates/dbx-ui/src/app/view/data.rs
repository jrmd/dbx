use super::super::*;

impl DbxApp {
    pub(super) fn render_data(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(session_id) = self.active_session_id() else {
            return div().into_any_element();
        };
        let Some((kind, redis_filter_editor, can_mutate, can_delete, filter_rows, inspector_open)) =
            self.session(session_id).map(|session| {
                (
                    session.kind,
                    session.editors.filter_editor.clone(),
                    self.editable_table_for(session_id).is_some(),
                    self.editable_table_for(session_id).is_some() && session.selected_row.is_some(),
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
                    .border_color(THEME.border)
                    .bg(THEME.panel)
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
                                            .text_color(THEME.text)
                                            .child(if kind.is_sql() {
                                                "Filters"
                                            } else {
                                                "Key pattern"
                                            }),
                                    )
                                    .when(kind.is_sql(), |view| {
                                        view.child(badge(
                                            format!("{} active", filter_rows.len()),
                                            THEME.text_muted,
                                        ))
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .when(kind.is_sql(), |view| {
                                        view.child(self.small_button(
                                            "add-filter",
                                            "Add filter",
                                            cx.listener(move |this, _, window, cx| {
                                                this.add_filter_for(session_id, window, cx)
                                            }),
                                        ))
                                        .child(
                                            self.small_button(
                                                "clear-filters",
                                                "Clear",
                                                cx.listener(move |this, _, _, cx| {
                                                    this.clear_filters_for(session_id, cx)
                                                }),
                                            ),
                                        )
                                    })
                                    .child(
                                        button("apply-filter", "Apply", ButtonKind::Primary)
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.refresh_table_for(session_id, cx)
                                            })),
                                    )
                                    .when(!self.narrow_workspace, |view| {
                                        view.child(self.small_button(
                                            "toggle-inspector",
                                            if inspector_open {
                                                "Hide details"
                                            } else {
                                                "Show details"
                                            },
                                            cx.listener(move |this, _, _, cx| {
                                                this.toggle_inspector_for(session_id, cx)
                                            }),
                                        ))
                                    })
                                    .child(self.small_button_state(
                                        "add-row",
                                        "Add row",
                                        can_mutate,
                                        cx.listener(move |this, _, window, cx| {
                                            this.begin_insert_for(session_id, window, cx)
                                        }),
                                    ))
                                    .child(self.small_button_state(
                                        "delete-row",
                                        "Delete",
                                        can_delete,
                                        cx.listener(move |this, _, _, cx| {
                                            this.delete_selected_for(session_id, cx)
                                        }),
                                    )),
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
                                .text_color(THEME.text_muted)
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
                                                        .bg(THEME.panel_raised)
                                                        .border_color(THEME.border_strong)
                                                        .text_color(THEME.text),
                                                ),
                                            )
                                            .child(
                                                div().flex_1().min_w_0().max_w(px(220.)).child(
                                                    Select::new(&operator_selector)
                                                        .with_size(Size::Small)
                                                        .w_full()
                                                        .menu_max_h(px(220.))
                                                        .text_size(px(11.))
                                                        .bg(THEME.panel_raised)
                                                        .border_color(THEME.border_strong)
                                                        .text_color(THEME.text),
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
                                                                .bg(THEME.panel_raised)
                                                                .text_color(THEME.text_muted)
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
                                                .child(icon(Icon::Close, THEME.text_muted))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.remove_filter_for(session_id, row_id, cx)
                                                })),
                                            )
                                    },
                                )),
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
                .text_color(THEME.text_muted)
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
        let Some((read_only_result, can_save, draft_mode, draft_fields, static_fields)) =
            self.session(session_id).map(|session| {
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
                                    field.enum_selector.clone(),
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
                                        value
                                            .map(ToString::to_string)
                                            .unwrap_or_else(|| "—".into()),
                                        value.is_some_and(|value| matches!(value, CellValue::Null)),
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .unwrap_or_default();
                (
                    session.result.is_some() && session.result_table.is_none(),
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
            .border_color(THEME.border)
            .bg(THEME.panel)
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
                            .text_color(THEME.text)
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
                                    DraftMode::Insert => "INSERT",
                                    DraftMode::Update if has_draft => "EDITING",
                                    DraftMode::Update => "READ ONLY",
                                },
                                if draft_mode == DraftMode::Update && !has_draft {
                                    THEME.text_muted
                                } else {
                                    THEME.accent
                                },
                            ))
                            .child(
                                Button::new("close-inspector")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Close row inspector")
                                    .child(icon(Icon::Close, THEME.text_muted))
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
                                    .text_color(THEME.text_muted)
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
                            enum_selector,
                        )| {
                            let field_focus = field_editor.read(cx).focus_handle();
                            let is_enum = enum_selector.is_some();
                            let value_control = enum_selector
                                .map(|selector| {
                                    Select::new(&selector)
                                        .with_size(Size::Small)
                                        .w_full()
                                        .menu_max_h(px(220.))
                                        .text_size(px(11.))
                                        .bg(THEME.canvas)
                                        .border_color(THEME.border_strong)
                                        .text_color(THEME.text)
                                        .into_any_element()
                                })
                                .unwrap_or_else(|| {
                                    editor::input(field_editor, field_focus, false).into_any_element()
                                });
                            div()
                                .id(SharedString::from(format!("row-field-{field_id}")))
                                .px(px(9.))
                                .py(px(10.))
                                .border_b_1()
                                .border_color(THEME.border)
                                .flex()
                                .flex_col()
                                .gap(px(7.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(THEME.text)
                                                .child(name),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(9.))
                                                .text_color(THEME.text_muted)
                                                .child(format!(
                                                    "{} · {}{}",
                                                    if is_enum {
                                                        format!("enum · {data_type}")
                                                    } else {
                                                        data_type
                                                    },
                                                    if nullable { "nullable" } else { "required" },
                                                    if primary_key { " · primary key" } else { "" }
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .child(
                                            Button::new(("row-field-value", field_id))
                                                .label("Value")
                                                .with_size(Size::XSmall)
                                                .compact()
                                                .outline()
                                                .selected(state == FieldValueState::Value)
                                                .bg(if state == FieldValueState::Value {
                                                    THEME.accent_soft
                                                } else {
                                                    THEME.panel_raised
                                                })
                                                .border_color(if state == FieldValueState::Value {
                                                    THEME.accent
                                                } else {
                                                    THEME.border
                                                })
                                                .text_color(if state == FieldValueState::Value {
                                                    THEME.accent
                                                } else {
                                                    THEME.text_muted
                                                })
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.set_row_field_state_for(
                                                        session_id,
                                                        field_id,
                                                        FieldValueState::Value,
                                                        cx,
                                                    )
                                                })),
                                        )
                                        .when(draft_mode == DraftMode::Insert, |view| {
                                            view.child(
                                                Button::new(("row-field-default", field_id))
                                                    .label("Default")
                                                    .with_size(Size::XSmall)
                                                    .compact()
                                                    .outline()
                                                    .selected(state == FieldValueState::Default)
                                                    .bg(if state == FieldValueState::Default {
                                                        THEME.accent_soft
                                                    } else {
                                                        THEME.panel_raised
                                                    })
                                                    .border_color(if state == FieldValueState::Default {
                                                        THEME.accent
                                                    } else {
                                                        THEME.border
                                                    })
                                                    .text_color(if state == FieldValueState::Default {
                                                        THEME.accent
                                                    } else {
                                                        THEME.text_muted
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.set_row_field_state_for(
                                                                session_id,
                                                                field_id,
                                                                FieldValueState::Default,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                        })
                                        .when(nullable, |view| {
                                            view.child(
                                                Button::new(("row-field-null", field_id))
                                                    .label("NULL")
                                                    .with_size(Size::XSmall)
                                                    .compact()
                                                    .outline()
                                                    .selected(state == FieldValueState::Null)
                                                    .bg(if state == FieldValueState::Null {
                                                        THEME.accent_soft
                                                    } else {
                                                        THEME.panel_raised
                                                    })
                                                    .border_color(if state == FieldValueState::Null {
                                                        THEME.accent
                                                    } else {
                                                        THEME.border
                                                    })
                                                    .text_color(if state == FieldValueState::Null {
                                                        THEME.accent
                                                    } else {
                                                        THEME.text_muted
                                                    })
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.set_row_field_state_for(
                                                                session_id,
                                                                field_id,
                                                                FieldValueState::Null,
                                                                cx,
                                                            )
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                                .when(state == FieldValueState::Value, |view| {
                                    view.child(value_control)
                                })
                                .when(state != FieldValueState::Value, |view| {
                                    view.child(
                                        div()
                                            .h(px(36.))
                                            .px(px(9.))
                                            .flex()
                                            .items_center()
                                            .rounded(px(6.))
                                            .bg(THEME.panel_raised)
                                            .text_size(px(11.))
                                            .text_color(THEME.text_muted)
                                            .child(if state == FieldValueState::Null {
                                                "This field will be saved as NULL."
                                            } else {
                                                "This column is omitted; the database supplies it."
                                            }),
                                    )
                                })
                        },
                    ))
                    .when(!has_draft, |view| {
                        view.children(static_fields.into_iter().map(
                            |(name, data_type, value, is_null)| {
                                div()
                                    .px(px(9.))
                                    .py(px(9.))
                                    .border_b_1()
                                    .border_color(THEME.border)
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
                                                    .text_color(THEME.text_muted)
                                                    .child(data_type),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(if is_null {
                                                THEME.text_muted
                                            } else {
                                                THEME.text
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
                    .border_color(THEME.border)
                    .flex()
                    .flex_col()
                    .gap(px(9.))
                    .child(div().text_size(px(10.)).text_color(THEME.text_muted).child(
                        if read_only_result {
                            "Query results are read-only."
                        } else {
                            match draft_mode {
                                DraftMode::Insert => {
                                    "Choose Value or NULL for fields to send; Default omits the column."
                                }
                                DraftMode::Update => {
                                    "All changed fields save together using the original primary key."
                                }
                            }
                        },
                    ))
                    .when(has_draft, |view| {
                        view.child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(self.small_button(
                                    "cancel-row-draft",
                                    "Cancel",
                                    cx.listener(move |this, _, _, cx| {
                                        this.cancel_row_draft_for(session_id, cx)
                                    }),
                                ))
                                .child(self.small_button_state(
                                    "save-row",
                                    if draft_mode == DraftMode::Insert {
                                        "Insert row"
                                    } else {
                                        "Save row"
                                    },
                                    can_save,
                                    cx.listener(move |this, _, _, cx| {
                                        this.save_draft_for(session_id, cx)
                                    }),
                                )),
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
                        .text_color(THEME.danger)
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
                    .border_color(THEME.border_strong)
                    .bg(THEME.panel_raised)
                    .text_size(px(9.))
                    .text_color(THEME.text_muted)
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
                    .border_color(THEME.border)
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(column.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(THEME.text_muted)
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
                            .border_color(THEME.border)
                            .rounded(px(6.))
                            .text_size(px(11.))
                            .text_color(THEME.text_muted)
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
                    .border_color(THEME.border)
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
                                    .text_color(THEME.text_muted)
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
                                    .hover(|style| style.text_color(THEME.text))
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
                                                THEME.accent
                                            } else {
                                                THEME.text_muted
                                            })
                                            .child(format!("REFERENCES {target}")),
                                    )
                                    .when(can_navigate, |view| {
                                        view.child(icon(Icon::ArrowRight, THEME.accent))
                                    }),
                            )
                            .when(!actions.is_empty(), |view| {
                                view.child(
                                    div()
                                        .text_size(px(9.))
                                        .text_color(THEME.text_muted)
                                        .child(actions),
                                )
                            }),
                    )
            }))
    }
}
