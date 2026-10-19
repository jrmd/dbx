use super::*;
use crate::theme::ui_px;
use crate::workspace::SavedQuery;
use gpui_component::checkbox::Checkbox;

fn designer_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
) -> Button {
    let id = id.into();
    // The shared button sets rounded_full on its style refinement. Set that
    // refinement explicitly; Button::rounded alone cannot override it.
    gpui::Styled::rounded(
        button(id.clone(), label, kind).debug_selector(move || id.to_string()),
        ui_px(5.),
    )
    .px(ui_px(10.))
}

fn designer_action_button(id: SharedString, label: &'static str, selected: bool) -> Button {
    Button::new(id.clone())
        .debug_selector(move || id.to_string())
        .ghost()
        .with_size(Size::XSmall)
        .h(ui_px(28.))
        .px(ui_px(8.))
        .rounded(ui_px(5.))
        .border_0()
        .bg(if selected {
            theme().accent_soft
        } else {
            gpui::transparent_black().into()
        })
        .text_color(if selected {
            theme().accent_text
        } else {
            theme().text_muted
        })
        .font_weight(FontWeight::MEDIUM)
        // Keep action labels compact so the navigation wraps in small windows.
        .child(div().line_height(relative(1.)).child(label))
        .accessible_name(label)
}

fn split_columns(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn designer_index_droppable(kind: DatabaseKind, index: &dbx_core::IndexInfo) -> bool {
    !index.primary
        && index.constraint_name.is_none()
        && !(kind.dialect() == DatabaseKind::SQLite && index.definition.is_none())
}

fn designer_constraints(
    kind: DatabaseKind,
    structure: &StructureTab,
) -> Vec<(String, &'static str)> {
    let mut constraints = Vec::new();
    for index in &structure.indexes {
        let name = index.constraint_name.clone().or_else(|| {
            (kind == DatabaseKind::MySQL && (index.primary || index.unique))
                .then(|| index.name.clone())
        });
        if let Some(name) = name {
            constraints.push((
                name,
                if index.primary {
                    "Primary key"
                } else if index.unique {
                    "Unique"
                } else {
                    "Exclusion"
                },
            ));
        }
    }
    for key in &structure.foreign_keys {
        if let Some(name) = &key.constraint_name {
            constraints.push((name.clone(), "Foreign key"));
        }
    }
    for check in &structure.checks {
        if let Some(name) = &check.name {
            constraints.push((name.clone(), "Check"));
        }
    }
    constraints.sort_by(|left, right| left.0.cmp(&right.0));
    constraints.dedup_by(|left, right| left.0 == right.0);
    constraints
}

pub(super) struct Designer {
    fields: Vec<Entity<TextEditor>>,
    action: usize,
    nullable: bool,
    unique: bool,
    drafts: Vec<String>,
    pending: bool,
    visible: bool,
}

impl DbxApp {
    pub(super) fn open_designer_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(designer) = self.designer_mut(session_id) {
            designer.visible = true;
            cx.notify();
            return;
        }
        let fields = (0..6)
            .map(|_| {
                let value = cx.new(|_| String::new());
                cx.new(|cx| TextEditor::new(value, false, window, cx))
            })
            .collect();
        if let Some(session) = self.session_mut(session_id)
            && let Some(tab) = session
                .secondary_tabs
                .iter_mut()
                .find(|tab| Some(tab.id) == session.active_secondary_tab)
            && let SecondaryTabKind::Structure(structure) = &mut tab.kind
        {
            structure.designer = Some(Designer {
                fields,
                action: 0,
                nullable: true,
                unique: false,
                drafts: Vec::new(),
                pending: false,
                visible: true,
            });
        }
        cx.notify();
    }

    pub(super) fn toggle_designer_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(designer) = self.designer_mut(session_id) {
            designer.visible = !designer.visible;
            cx.notify();
        } else {
            self.open_designer_for(session_id, window, cx);
        }
    }

    fn draft_designer_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let queued = self.session(session_id).and_then(|session| {
            let tab = session
                .secondary_tabs
                .iter()
                .find(|tab| Some(tab.id) == session.active_secondary_tab)?;
            let SecondaryTabKind::Structure(structure) = &tab.kind else {
                return None;
            };
            let designer = structure.designer.as_ref()?;
            (!designer.pending
                && !designer.drafts.is_empty()
                && designer
                    .fields
                    .iter()
                    .all(|field| field.read(cx).text(cx).trim().is_empty()))
            .then(|| SavedQuery {
                name: format!("Alter {}", structure.table.name),
                sql: designer.drafts.join("\n"),
            })
        });
        if let Some(query) = queued {
            self.open_saved_query_for(session_id, query, window, cx);
        } else {
            self.build_designer_draft(session_id, false, window, cx);
        }
    }

    fn build_designer_draft(
        &mut self,
        session_id: SessionId,
        queue: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((kind, table, change, previous)) = self.session(session_id).and_then(|session| {
            let tab = session
                .secondary_tabs
                .iter()
                .find(|tab| Some(tab.id) == session.active_secondary_tab)?;
            let SecondaryTabKind::Structure(structure) = &tab.kind else {
                return None;
            };
            let designer = structure.designer.as_ref()?;
            if designer.pending {
                return None;
            }
            let values = designer
                .fields
                .iter()
                .map(|editor| editor.read(cx).text(cx))
                .collect::<Vec<_>>();
            if designer.action == 4
                && structure.indexes.iter().any(|index| {
                    index.name == values[0].trim() && !designer_index_droppable(session.kind, index)
                })
            {
                return Some((
                    session.kind,
                    structure.table.clone(),
                    None,
                    designer.drafts.clone(),
                ));
            }
            let change = match designer.action {
                0 => dbx_core::TableAlteration::AddColumn {
                    name: values[0].trim().into(),
                    data_type: values[1].trim().into(),
                    nullable: designer.nullable,
                    default: (!values[2].trim().is_empty()).then(|| values[2].trim().into()),
                },
                1 => dbx_core::TableAlteration::RenameColumn {
                    name: values[0].trim().into(),
                    new_name: values[1].trim().into(),
                },
                2 => dbx_core::TableAlteration::DropColumn {
                    name: values[0].trim().into(),
                },
                3 => dbx_core::TableAlteration::AddIndex {
                    name: values[0].trim().into(),
                    columns: values[1]
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    unique: designer.unique,
                },
                4 => dbx_core::TableAlteration::DropIndex {
                    name: values[0].trim().into(),
                },
                5 => dbx_core::TableAlteration::AlterColumn {
                    name: values[0].trim().into(),
                    data_type: values[1].trim().into(),
                    nullable: designer.nullable,
                    default: (!values[2].trim().is_empty()).then(|| values[2].trim().into()),
                },
                6 => dbx_core::TableAlteration::AddPrimaryKey {
                    name: values[0].trim().into(),
                    columns: split_columns(&values[1]),
                },
                7 => dbx_core::TableAlteration::AddForeignKey {
                    name: values[0].trim().into(),
                    columns: split_columns(&values[1]),
                    referenced_table: TableRef {
                        name: values[2].trim().into(),
                        schema: (!values[4].trim().is_empty()).then(|| values[4].trim().into()),
                    },
                    referenced_columns: split_columns(&values[3]),
                },
                8 => dbx_core::TableAlteration::AddCheck {
                    name: values[0].trim().into(),
                    expression: values[1].trim().into(),
                },
                _ => dbx_core::TableAlteration::DropConstraint {
                    name: values[0].trim().into(),
                },
            };
            Some((
                session.kind,
                structure.table.clone(),
                Some(change),
                designer.drafts.clone(),
            ))
        }) else {
            return;
        };
        let Some(change) = change else {
            self.show_toast(ToastKind::Error, "This index belongs to a constraint. Use Drop constraint, or review the engine’s table-rebuild SQL.", cx);
            return;
        };
        if kind == DatabaseKind::MySQL
            && matches!(change, dbx_core::TableAlteration::AlterColumn { .. })
        {
            let Some(session) = self.session(session_id) else {
                return;
            };
            let Some(engine) = session.engine.clone() else {
                return;
            };
            let expected_engine = engine.clone();
            let database = session.current_database.clone();
            let tab_id = session.active_secondary_tab;
            if let Some(designer) = self.designer_mut(session_id) {
                designer.pending = true;
            }
            let source = table.clone();
            let task = self.runtime.spawn(async move {
                dbx_core::draft_table_alteration_for(&engine, &source, &change).await
            });
            if let Some(session) = self.session_mut(session_id) {
                session.track_background_task(&task);
            }
            cx.spawn_in(window, async move |this, cx| {
                let result = task.await?;
                this.update_in(cx, |this, window, cx| {
                    let Some(session) = this.session_mut(session_id) else {
                        return;
                    };
                    if session.current_database != database
                        || !session
                            .engine
                            .as_ref()
                            .is_some_and(|engine| Arc::ptr_eq(engine, &expected_engine))
                    {
                        return;
                    }
                    if let Some(tab) = session
                        .secondary_tabs
                        .iter_mut()
                        .find(|tab| Some(tab.id) == tab_id)
                        && let SecondaryTabKind::Structure(structure) = &mut tab.kind
                        && let Some(designer) = &mut structure.designer
                    {
                        designer.pending = false;
                    }
                    if session.active_secondary_tab != tab_id {
                        cx.notify();
                        return;
                    }
                    this.finish_designer_draft(
                        session_id, queue, table, previous, result, window, cx,
                    );
                })?;
                Ok::<(), anyhow::Error>(())
            })
            .detach();
            cx.notify();
            return;
        }
        self.finish_designer_draft(
            session_id,
            queue,
            table.clone(),
            previous,
            dbx_core::draft_table_alteration(kind, &table, &change),
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_designer_draft(
        &mut self,
        session_id: SessionId,
        queue: bool,
        table: TableRef,
        previous: Vec<String>,
        result: dbx_core::Result<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(sql) if queue => {
                if let Some(session) = self.session_mut(session_id)
                    && let Some(tab) = session
                        .secondary_tabs
                        .iter_mut()
                        .find(|tab| Some(tab.id) == session.active_secondary_tab)
                    && let SecondaryTabKind::Structure(structure) = &mut tab.kind
                    && let Some(designer) = &mut structure.designer
                {
                    designer.drafts.push(sql);
                }
                cx.notify();
            }
            Ok(sql) => {
                let mut statements = previous;
                if statements.last() != Some(&sql) {
                    statements.push(sql);
                }
                self.open_saved_query_for(
                    session_id,
                    SavedQuery {
                        name: format!("Alter {}", table.name),
                        sql: statements.join("\n"),
                    },
                    window,
                    cx,
                );
            }
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
    }

    fn designer_mut(&mut self, session_id: SessionId) -> Option<&mut Designer> {
        let session = self.session_mut(session_id)?;
        let tab = session
            .secondary_tabs
            .iter_mut()
            .find(|tab| Some(tab.id) == session.active_secondary_tab)?;
        let SecondaryTabKind::Structure(structure) = &mut tab.kind else {
            return None;
        };
        structure.designer.as_mut()
    }
    fn set_designer_field(
        &mut self,
        session_id: SessionId,
        index: usize,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(designer) = self.designer_mut(session_id) {
            designer.fields[index].update(cx, |editor, cx| editor.set_text(value, cx));
        }
        cx.notify();
    }
    fn append_designer_column(
        &mut self,
        session_id: SessionId,
        index: usize,
        name: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(designer) = self.designer_mut(session_id) {
            let mut names = split_columns(&designer.fields[index].read(cx).text(cx));
            if let Some(position) = names.iter().position(|value| value == &name) {
                names.remove(position);
            } else {
                names.push(name);
            }
            designer.fields[index].update(cx, |editor, cx| editor.set_text(names.join(", "), cx));
        }
        cx.notify();
    }

    pub(super) fn render_designer_for(
        &self,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.session(session_id)?;
        let tab = session
            .secondary_tabs
            .iter()
            .find(|tab| Some(tab.id) == session.active_secondary_tab)?;
        let SecondaryTabKind::Structure(structure) = &tab.kind else {
            return None;
        };
        let designer = structure.designer.as_ref()?;
        if !designer.visible {
            return None;
        }
        let action = designer.action;
        let fields = designer.fields.clone();
        let actions = [
            "Add column",
            "Rename column",
            "Drop column",
            "Add index",
            "Drop index",
            "Alter column",
            "Primary key",
            "Foreign key",
            "Check",
            "Drop constraint",
        ];
        let description = match action {
            0 => "Define a new column and its default value.",
            1 => "Choose an existing column and give it a new name.",
            2 => "Choose the column to remove from this table.",
            3 => "Index one or more columns to speed up lookups.",
            4 => "Choose the index to remove from this table.",
            5 => "Update a column’s type, default value and nullability.",
            6 => "Choose the columns that uniquely identify each row.",
            7 => "Link columns in this table to another table.",
            8 => "Define a condition that every row must satisfy.",
            _ => "Choose the constraint to remove from this table.",
        };
        // Keep labels short; guidance belongs below the relevant field.
        let labels: &[(&str, &str)] = match action {
            0 => &[
                ("Column name", ""),
                ("Data type", ""),
                ("Default expression", "Optional · SQL expression or literal"),
            ],
            1 => &[("Existing column", ""), ("New name", "")],
            2 => &[("Column to drop", "")],
            3 => &[
                ("Index name", ""),
                ("Columns", "Separate column names with commas"),
            ],
            4 => &[("Index to drop", "")],
            5 => &[
                ("Existing column", ""),
                ("Data type", "Enter the complete type definition"),
                ("Default expression", "Leave empty to remove the default"),
            ],
            6 => &[
                ("Constraint name", ""),
                ("Primary key columns", "Separate column names with commas"),
            ],
            7 => &[
                ("Constraint name", ""),
                ("Local columns", "Separate column names with commas"),
                ("Referenced table", ""),
                ("Referenced columns", "Separate column names with commas"),
                ("Referenced schema", "Optional"),
            ],
            8 => &[
                ("Constraint name", ""),
                ("Check expression", "SQL condition, e.g. budget >= 0"),
            ],
            _ => &[("Constraint to drop", "")],
        };
        let type_value = fields[1].read(cx).text(cx);
        let selected_column = fields[0].read(cx).text(cx);
        let local_columns = split_columns(&fields[1].read(cx).text(cx));
        let reference_table = TableRef {
            name: fields[2].read(cx).text(cx).trim().into(),
            schema: {
                let schema = fields[4].read(cx).text(cx);
                (!schema.trim().is_empty()).then(|| schema.trim().into())
            },
        };
        let referenced_columns = split_columns(&fields[3].read(cx).text(cx));
        let chip = |id: SharedString, label: String, selected: bool| {
            designer_button(id, label, ButtonKind::Quiet)
                .h(ui_px(24.))
                .px(ui_px(7.))
                .bg(if selected {
                    theme().accent_soft
                } else {
                    theme().panel_raised
                })
                .text_color(if selected {
                    theme().accent_text
                } else {
                    theme().text_muted
                })
                .border_color(if selected {
                    theme().accent
                } else {
                    theme().border
                })
        };
        let mut form = div()
            .w_full()
            .max_w(ui_px(760.))
            .flex()
            .flex_col()
            .gap(ui_px(12.));
        for (index, &(label, hint)) in labels.iter().enumerate() {
            let editor = fields[index].clone();
            editor.update(cx, |editor, cx| editor.set_accessibility_label(label, cx));
            let focus = editor.read(cx).focus_handle();
            let mut field = div()
                .id(format!("designer-field-{index}"))
                .debug_selector(move || format!("designer-field-{index}"))
                .min_w_0()
                .flex()
                .flex_col()
                .gap(ui_px(5.))
                .child(
                    div()
                        .text_size(crate::theme::text_px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
                .child(
                    editor::input_with_height(editor, focus, false, ui_px(32.), cx)
                        .debug_selector(move || format!("designer-input-{index}"))
                        .flex_none(),
                )
                .when(!hint.is_empty(), |view| {
                    view.child(
                        div()
                            .text_size(crate::theme::text_px(10.))
                            .text_color(theme().text_muted)
                            .child(hint),
                    )
                });
            if index == 0 && action == 4 {
                field = field
                    .child(div().flex().flex_col().gap(ui_px(5.)).children(
                        structure.indexes.iter().map(|index| {
                            let name = index.name.clone();
                            let droppable = designer_index_droppable(session.kind, index);
                            let label = format!(
                                "{} ({}){}",
                                name,
                                index.columns.join(", "),
                                if droppable {
                                    ""
                                } else {
                                    " · constraint index"
                                }
                            );
                            chip(
                                format!("designer-index-{name}").into(),
                                label,
                                selected_column.trim() == name,
                            )
                            .disabled(!droppable)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.set_designer_field(session_id, 0, name.clone(), cx);
                                },
                            ))
                        }),
                    ))
                    .child(
                        div()
                            .text_size(crate::theme::text_px(10.))
                            .text_color(theme().text_muted)
                            .child(if structure.indexes.is_empty() {
                                "No indexes on this table."
                            } else {
                                "Constraint indexes are removed through Drop constraint."
                            }),
                    );
            }
            if index == 0 && action == 9 {
                let constraints = designer_constraints(session.kind, structure);
                field = field.child(div().flex().flex_wrap().gap(ui_px(4.)).children(
                    constraints.iter().map(|(name, kind)| {
                        let name = name.clone();
                        chip(format!("designer-constraint-{name}").into(), format!("{name} · {kind}"), selected_column.trim() == name)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_designer_field(session_id, 0, name.clone(), cx);
                            }))
                    })
                )).when(constraints.is_empty(), |view| view.child(
                    div().text_size(crate::theme::text_px(10.)).text_color(theme().text_muted)
                        .child("No named constraints available. Enter a name if you created one in the SQL plan.")
                ));
            }
            if index == 0 && !structure.columns.is_empty() && matches!(action, 1 | 2 | 5) {
                field = field.child(div().flex().flex_wrap().gap(ui_px(4.)).children(
                    structure.columns.iter().map(|column| {
                        let name = column.name.clone();
                        let data_type = column.data_type.clone();
                        let default = column.default_value.clone().unwrap_or_default();
                        let nullable = column.nullable;
                        chip(
                            format!("designer-column-{name}").into(),
                            name.clone(),
                            selected_column.trim() == name,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(designer) = this.designer_mut(session_id) {
                                designer.fields[0]
                                    .update(cx, |editor, cx| editor.set_text(name.clone(), cx));
                                if action == 5 {
                                    designer.fields[1].update(cx, |editor, cx| {
                                        editor.set_text(data_type.clone(), cx)
                                    });
                                    designer.fields[2].update(cx, |editor, cx| {
                                        editor.set_text(default.clone(), cx)
                                    });
                                    designer.nullable = nullable;
                                }
                            }
                            cx.notify();
                        }))
                    }),
                ));
            }
            if index == 1 && matches!(action, 0 | 5) {
                field = field.child(
                    div().flex().flex_wrap().gap(ui_px(4.)).children(
                        [
                            "integer",
                            "bigint",
                            "text",
                            "varchar(255)",
                            "boolean",
                            "numeric(12,2)",
                            "timestamp",
                        ]
                        .into_iter()
                        .map(|value| {
                            chip(
                                format!("designer-type-{value}").into(),
                                value.into(),
                                type_value.trim().eq_ignore_ascii_case(value),
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.set_designer_field(session_id, 1, value.into(), cx)
                                },
                            ))
                        }),
                    ),
                );
            }
            if index == 1 && matches!(action, 0 | 5) {
                field = field.child(
                    div().flex().flex_wrap().gap(ui_px(4.)).children(
                        session
                            .schema_objects
                            .iter()
                            .filter(|object| object.kind == dbx_core::SchemaObjectKind::Enum)
                            .filter_map(|object| {
                                dbx_core::quote_identifier(session.kind, &object.name)
                                    .ok()
                                    .and_then(|name| match &object.schema {
                                        Some(schema) => {
                                            dbx_core::quote_identifier(session.kind, schema)
                                                .ok()
                                                .map(|schema| format!("{schema}.{name}"))
                                        }
                                        None => Some(name),
                                    })
                            })
                            .map(|name| {
                                chip(
                                    format!("designer-enum-{name}").into(),
                                    name.clone(),
                                    type_value.trim() == name,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.set_designer_field(session_id, 1, name.clone(), cx);
                                    },
                                ))
                            }),
                    ),
                );
            }
            if index == 1 && matches!(action, 3 | 6 | 7) && !structure.columns.is_empty() {
                field = field.child(div().flex().flex_wrap().gap(ui_px(4.)).children(
                    structure.columns.iter().map(|column| {
                        let name = column.name.clone();
                        chip(
                            format!("designer-local-{name}").into(),
                            name.clone(),
                            local_columns.contains(&name),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.append_designer_column(session_id, 1, name.clone(), cx)
                        }))
                    }),
                ));
            }
            if action == 7 && index == 2 {
                field = field.child(
                    div()
                        .max_h(ui_px(96.))
                        .id("designer-reference-tables")
                        .overflow_y_scroll()
                        .flex()
                        .flex_wrap()
                        .gap(ui_px(4.))
                        .children(
                            session
                                .tables
                                .iter()
                                .filter(|table| table.kind == EntityKind::Table)
                                .map(|table| {
                                    let table = table.clone();
                                    let label = match &table.schema {
                                        Some(schema) => format!("{schema}.{}", table.name),
                                        None => table.name.clone(),
                                    };
                                    chip(
                                        format!("designer-ref-{:?}-{}", table.schema, table.name)
                                            .into(),
                                        label,
                                        reference_table.name == table.name
                                            && reference_table.schema == table.schema,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            this.set_designer_field(
                                                session_id,
                                                2,
                                                table.name.clone(),
                                                cx,
                                            );
                                            this.set_designer_field(
                                                session_id,
                                                4,
                                                table.schema.clone().unwrap_or_default(),
                                                cx,
                                            );
                                            this.set_designer_field(
                                                session_id,
                                                3,
                                                String::new(),
                                                cx,
                                            );
                                        },
                                    ))
                                }),
                        ),
                );
            }
            if action == 7 && index == 3 {
                field = field.child(
                    div().flex().flex_wrap().gap(ui_px(4.)).children(
                        session
                            .completion_columns
                            .get(&completion_table_key(&reference_table))
                            .into_iter()
                            .flatten()
                            .map(|column| {
                                let name = column.name.clone();
                                chip(
                                    format!("designer-ref-column-{name}").into(),
                                    name.clone(),
                                    referenced_columns.contains(&name),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.append_designer_column(session_id, 3, name.clone(), cx)
                                    },
                                ))
                            }),
                    ),
                );
            }
            form = form.child(field);
        }
        form = form
            .when(matches!(action, 0 | 3 | 5), |view| view.child(
                Checkbox::new("designer-option")
                    .label(if action == 3 { "Unique index" } else { "Allow NULL values" })
                    .checked(if action == 3 { designer.unique } else { designer.nullable })
                    .with_size(Size::Small)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(designer) = this.designer_mut(session_id) {
                            if action == 3 { designer.unique = !designer.unique; }
                            else { designer.nullable = !designer.nullable; }
                        }
                        cx.notify();
                    }))
            ))
            .when(session.kind == DatabaseKind::MySQL && action == 5, |view| view.child(
                div().text_size(crate::theme::text_px(11.)).text_color(theme().warning_text)
                    .child("MODIFY replaces the complete column definition. Existing AUTO_INCREMENT, COLLATE, COMMENT and ON UPDATE attributes are preserved from metadata. Generated columns require manual SQL.")
            ));
        Some(
            div()
                .id("table-designer")
                .debug_selector(|| "table-designer".into())
                .w_full()
                .flex_none()
                .mb(ui_px(12.))
                .border_1()
                .border_color(theme().border_strong)
                .rounded(ui_px(6.))
                .bg(theme().panel)
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .w_full()
                        .flex_none()
                        .p(ui_px(8.))
                        .border_b_1()
                        .border_color(theme().border)
                        .bg(theme().panel_raised)
                        .flex()
                        .flex_wrap()
                        .gap(ui_px(16.))
                        .children(
                            [
                                ("Columns", vec![0, 1, 5, 2]),
                                ("Indexes", vec![3, 4]),
                                ("Constraints", vec![6, 7, 8, 9]),
                            ]
                            .into_iter()
                            .map(|(label, indices)| {
                                div()
                                    .min_w_0()
                                    .max_w_full()
                                    .flex()
                                    .flex_col()
                                    .gap(ui_px(2.))
                                    .child(
                                        div()
                                            .px(ui_px(8.))
                                            .py(ui_px(4.))
                                            .text_size(crate::theme::text_px(10.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme().text_muted)
                                            .child(label),
                                    )
                                    .child(div().flex().flex_wrap().gap(ui_px(2.)).children(
                                        indices.into_iter().map(|index| {
                                            designer_action_button(
                                                format!("designer-action-{index}").into(),
                                                actions[index],
                                                index == action,
                                            )
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    if let Some(designer) =
                                                        this.designer_mut(session_id)
                                                        && designer.action != index
                                                    {
                                                        designer.action = index;
                                                        for field in &designer.fields {
                                                            field.update(cx, |editor, cx| {
                                                                editor.set_text(String::new(), cx)
                                                            });
                                                        }
                                                        designer.nullable = true;
                                                        designer.unique = false;
                                                    }
                                                    cx.notify();
                                                }),
                                            )
                                        }),
                                    ))
                            }),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .p(ui_px(16.))
                                .flex_1()
                                .flex()
                                .flex_col()
                                .gap(ui_px(16.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(ui_px(4.))
                                        .child(
                                            div()
                                                .text_size(crate::theme::text_px(13.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(actions[action]),
                                        )
                                        .child(
                                            div()
                                                .text_size(crate::theme::text_px(11.))
                                                .text_color(theme().text_muted)
                                                .child(description),
                                        ),
                                )
                                .child(form),
                        )
                        .child(
                            div()
                                .id("designer-footer")
                                .border_t_1()
                                .border_color(theme().border)
                                .px(ui_px(16.))
                                .py(ui_px(12.))
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .justify_between()
                                .gap(ui_px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(ui_px(8.))
                                        .child(
                                            div()
                                                .text_size(crate::theme::text_px(11.))
                                                .text_color(theme().text_muted)
                                                .child(format!("{} queued", designer.drafts.len())),
                                        )
                                        .when(!designer.drafts.is_empty(), |view| {
                                            view.child(
                                                designer_button(
                                                    "designer-clear-plan",
                                                    "Clear",
                                                    ButtonKind::Quiet,
                                                )
                                                .h(ui_px(24.))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if let Some(designer) =
                                                        this.designer_mut(session_id)
                                                    {
                                                        designer.drafts.clear();
                                                    }
                                                    cx.notify();
                                                })),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap(ui_px(6.))
                                        .child(
                                            designer_button(
                                                "designer-queue",
                                                "Add to plan",
                                                ButtonKind::Quiet,
                                            )
                                            .disabled(designer.pending)
                                            .on_click(
                                                cx.listener(move |this, _, window, cx| {
                                                    this.build_designer_draft(
                                                        session_id, true, window, cx,
                                                    )
                                                }),
                                            ),
                                        )
                                        .child(
                                            designer_button(
                                                "designer-draft",
                                                "Review SQL plan",
                                                ButtonKind::Primary,
                                            )
                                            .disabled(designer.pending)
                                            .on_click(
                                                cx.listener(move |this, _, window, cx| {
                                                    this.draft_designer_for(session_id, window, cx)
                                                }),
                                            ),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn index_fixture(name: &str, constraint: Option<&str>, primary: bool) -> dbx_core::IndexInfo {
        dbx_core::IndexInfo {
            name: name.into(),
            constraint_name: constraint.map(str::to_owned),
            columns: vec!["id".into()],
            unique: true,
            primary,
            method: Some("btree".into()),
            predicate: None,
            definition: Some(format!("CREATE UNIQUE INDEX {name} ON items (id)")),
        }
    }

    #[test]
    fn constraint_indexes_cannot_be_dropped_as_standalone_indexes() {
        let regular = index_fixture("lookup", None, false);
        let key = index_fixture("key", Some("key"), false);
        let primary = index_fixture("primary", Some("primary"), true);
        assert!(designer_index_droppable(DatabaseKind::PostgreSQL, &regular));
        assert!(!designer_index_droppable(DatabaseKind::PostgreSQL, &key));
        assert!(!designer_index_droppable(
            DatabaseKind::PostgreSQL,
            &primary
        ));
        let mut implicit = regular.clone();
        implicit.definition = None;
        assert!(!designer_index_droppable(DatabaseKind::SQLite, &implicit));
    }

    #[gpui::test]
    fn designer_layout_preserves_reviewable_drafts(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            let engine = Arc::new(
                app.runtime
                    .block_on(DatabaseEngine::connect(ConnectionConfig::new(
                        DatabaseKind::SQLite,
                        "sqlite::memory:",
                    )))
                    .unwrap(),
            );
            app.runtime
                .block_on(engine.execute_sql("CREATE TABLE items(id INTEGER PRIMARY KEY)"))
                .unwrap();
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Designer".into(),
                DatabaseKind::PostgreSQL,
                None,
                window,
                cx,
            );
            session.engine = Some(engine);
            session.schema_objects.push(dbx_core::SchemaObject {
                kind: dbx_core::SchemaObjectKind::Enum,
                name: "mood".into(),
                schema: Some("public".into()),
                table: None,
                definition: Some("CREATE TYPE public.mood AS ENUM ('happy', 'sad')".into()),
            });
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Structure(Box::new(StructureTab {
                    scroll: ScrollHandle::new(),
                    expanded_sections: HashSet::new(),
                    designer: None,
                    table: TableRef::new("items"),
                    columns: app
                        .runtime
                        .block_on(
                            session
                                .engine
                                .as_ref()
                                .unwrap()
                                .describe_table(&TableRef::new("items")),
                        )
                        .unwrap(),
                    foreign_keys: Vec::new(),
                    indexes: vec![
                        index_fixture("id_lookup", None, false),
                        index_fixture("id_pkey", Some("id_pkey"), true),
                    ],
                    checks: vec![dbx_core::CheckConstraintInfo {
                        name: Some("positive_id".into()),
                        expression: "id > 0".into(),
                    }],
                    definition: None,
                    busy: false,
                    error: None,
                })),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Structure;
            app.sessions.push(session);
            app.active_session_id = Some(session_id);
            app.connection_picker_open = false;
            app.startup_recovery_started = true;
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| app.open_designer_for(session_id, window, cx))
        });
        // Exercise the actual native layout at compact and wide window sizes.
        for (scale, width) in [
            (100, 720.),
            (100, 1440.),
            (200, 720.),
            (200, 1440.),
            (100, 1440.),
        ] {
            cx.update(|_, cx| app.update(cx, |app, _| app.ui_scale = scale));
            cx.simulate_resize(gpui::size(px(width), px(1000.)));
            for action in 0..10 {
                cx.update(|_, cx| {
                    app.update(cx, |app, _| {
                        app.designer_mut(session_id).unwrap().action = action
                    })
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let panel = cx
                    .debug_bounds("table-designer")
                    .expect("designer is visible");
                for id in [
                    "designer-field-0",
                    "designer-input-0",
                    "designer-queue",
                    "designer-draft",
                    "designer-action-9",
                ] {
                    let bounds = cx.debug_bounds(id).expect("designer control is visible");
                    assert!(
                        bounds.origin.x >= panel.origin.x && bounds.right() <= panel.right(),
                        "{id} spills outside the designer at width {width}: {bounds:?}"
                    );
                    assert!(bounds.size.height > px(0.), "{id} is collapsed");
                }
                assert!(
                    panel.right() <= px(width),
                    "designer exceeds the window width"
                );
                if scale == 100 && width == 1440. && action == 9 {
                    assert!(
                        panel.size.height < ui_px(340.),
                        "simple forms should not inherit a tall navigation sidebar: {panel:?}"
                    );
                }
                let input = cx.debug_bounds("designer-input-0").unwrap();
                assert!(
                    input.size.height >= ui_px(32.),
                    "input retains its minimum height and can grow to fit the font"
                );
                let queue = cx.debug_bounds("designer-queue").unwrap();
                assert!(
                    queue.size.width < ui_px(180.),
                    "plan action should remain compact"
                );
            }
        }
        // Use native buttons to choose existing metadata and check task changes
        // clear the previous fields without losing the queued SQL.
        for (action, choice, expected) in [
            (4, "designer-index-id_lookup", "id_lookup"),
            (9, "designer-constraint-id_pkey", "id_pkey"),
        ] {
            let nav = cx
                .debug_bounds(if action == 4 {
                    "designer-action-4"
                } else {
                    "designer-action-9"
                })
                .unwrap();
            cx.simulate_click(nav.center(), gpui::Modifiers::none());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            app.read_with(cx, |app, cx| {
                let designer = match &app.session(session_id).unwrap().secondary_tabs[0].kind {
                    SecondaryTabKind::Structure(structure) => structure.designer.as_ref().unwrap(),
                    _ => panic!(),
                };
                assert!(designer.fields[0].read(cx).text(cx).is_empty());
            });
            let bounds = cx.debug_bounds(choice).unwrap();
            cx.simulate_click(bounds.center(), gpui::Modifiers::none());
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert_eq!(
                        app.designer_mut(session_id).unwrap().fields[0]
                            .read(cx)
                            .text(cx),
                        expected
                    );
                    app.build_designer_draft(session_id, true, window, cx);
                })
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let designer = app.designer_mut(session_id).unwrap();
                assert_eq!(designer.drafts.len(), 2);
                assert!(designer.drafts[0].contains("DROP INDEX"));
                assert!(designer.drafts[1].contains("DROP CONSTRAINT"));
                designer.action = 4;
                designer.fields[0].update(cx, |editor, cx| editor.set_text("id_pkey", cx));
                app.build_designer_draft(session_id, true, window, cx);
                assert_eq!(app.designer_mut(session_id).unwrap().drafts.len(), 2);
                // Review a queued plan after choosing a new, still-empty action.
                for field in &app.designer_mut(session_id).unwrap().fields {
                    field.update(cx, |editor, cx| editor.set_text(String::new(), cx));
                }
                app.draft_designer_for(session_id, window, cx);
                assert!(
                    app.active_query_tab(session_id)
                        .unwrap()
                        .query_text
                        .read(cx)
                        .contains("DROP CONSTRAINT")
                );
                app.activate_secondary_tab_for(session_id, tab_id, window, cx);
                app.designer_mut(session_id).unwrap().drafts.clear();
            })
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.designer_mut(session_id).unwrap().action = 0;
                let tab = &app.session(session_id).unwrap().secondary_tabs[0];
                let SecondaryTabKind::Structure(structure) = &tab.kind else {
                    panic!()
                };
                let designer = structure.designer.as_ref().unwrap();
                designer.fields[0].update(cx, |editor, cx| editor.set_text("display name", cx));
                designer.fields[1].update(cx, |editor, cx| editor.set_text("TEXT", cx));
                app.build_designer_draft(session_id, true, window, cx);
                app.toggle_designer_for(session_id, window, cx);
                assert!(app.render_designer_for(session_id, cx).is_none());
                app.toggle_designer_for(session_id, window, cx);
                let designer = app.designer_mut(session_id).unwrap();
                assert_eq!(designer.drafts.len(), 1);
                assert_eq!(designer.fields[0].read(cx).text(cx), "display name");
                assert!(designer.visible);
                app.draft_designer_for(session_id, window, cx);
                let query = app.active_query_tab(session_id).unwrap();
                assert!(
                    query
                        .query_text
                        .read(cx)
                        .contains("ALTER TABLE \"items\" ADD COLUMN \"display name\" TEXT")
                );
                let columns = app
                    .runtime
                    .block_on(
                        app.session(session_id)
                            .unwrap()
                            .engine
                            .as_ref()
                            .unwrap()
                            .describe_table(&TableRef::new("items")),
                    )
                    .unwrap();
                assert_eq!(columns.len(), 1);
            })
        });
    }
}
