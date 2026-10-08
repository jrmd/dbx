use super::*;
use crate::workspace::SavedQuery;

fn split_columns(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) struct Designer {
    fields: Vec<Entity<TextEditor>>,
    action: usize,
    nullable: bool,
    unique: bool,
    drafts: Vec<String>,
    pending: bool,
}

impl DbxApp {
    pub(super) fn open_designer_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            });
        }
        cx.notify();
    }

    fn draft_designer_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.build_designer_draft(session_id, false, window, cx);
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
                change,
                designer.drafts.clone(),
            ))
        }) else {
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
            if !names.contains(&name) {
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
        let action = designer.action;
        let fields = designer.fields.clone();
        let labels = match action {
            0 => vec!["Column name", "Type", "Default expression (optional)"],
            1 => vec!["Existing column", "New name"],
            2 => vec!["Column to drop"],
            3 => vec!["Index name", "Columns, separated by commas"],
            4 => vec!["Index to drop"],
            5 => vec![
                "Existing column",
                "New complete type",
                "Default (empty removes default)",
            ],
            6 => vec![
                "Constraint name",
                "Primary key columns, separated by commas",
            ],
            7 => vec![
                "Constraint name",
                "Local columns, separated by commas",
                "Referenced table",
                "Referenced columns, separated by commas",
                "Referenced schema (optional)",
            ],
            8 => vec!["Constraint name", "Check expression"],
            _ => vec!["Constraint to drop"],
        };
        Some(
            div()
                .p(px(12.))
                .border_1()
                .border_color(theme().border)
                .rounded(px(6.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div().flex().flex_wrap().gap(px(6.)).children(
                        [
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
                        ]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            button(
                                SharedString::from(format!("designer-action-{index}")),
                                label,
                                if index == action {
                                    ButtonKind::Primary
                                } else {
                                    ButtonKind::Quiet
                                },
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(session) = this.session_mut(session_id)
                                        && let Some(tab) =
                                            session.secondary_tabs.iter_mut().find(|tab| {
                                                Some(tab.id) == session.active_secondary_tab
                                            })
                                        && let SecondaryTabKind::Structure(structure) =
                                            &mut tab.kind
                                        && let Some(designer) = &mut structure.designer
                                    {
                                        designer.action = index;
                                    }
                                    cx.notify();
                                },
                            ))
                        }),
                    ),
                )
                .children(labels.into_iter().zip(fields).map(|(label, editor)| {
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(div().w(px(200.)).child(label))
                        .child(editor)
                }))
                .when(action == 0 || action == 3 || action == 5, |view| {
                    view.child(
                        button(
                            "designer-option",
                            if action == 0 || action == 5 {
                                if designer.nullable {
                                    "Nullable: yes"
                                } else {
                                    "Nullable: no"
                                }
                            } else if designer.unique {
                                "Unique: yes"
                            } else {
                                "Unique: no"
                            },
                            ButtonKind::Quiet,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(session) = this.session_mut(session_id)
                                && let Some(tab) = session
                                    .secondary_tabs
                                    .iter_mut()
                                    .find(|tab| Some(tab.id) == session.active_secondary_tab)
                                && let SecondaryTabKind::Structure(structure) = &mut tab.kind
                                && let Some(designer) = &mut structure.designer
                            {
                                if action == 0 || action == 5 {
                                    designer.nullable = !designer.nullable;
                                } else {
                                    designer.unique = !designer.unique;
                                }
                            }
                            cx.notify();
                        })),
                    )
                })
                .when(
                    !structure.columns.is_empty() && matches!(action, 1 | 2 | 5),
                    |view| {
                        view.child(div().flex().flex_wrap().gap(px(4.)).children(
                            structure.columns.iter().map(|column| {
                                let name = column.name.clone();
                                let data_type = column.data_type.clone();
                                let default = column.default_value.clone().unwrap_or_default();
                                let nullable = column.nullable;
                                button(
                                    SharedString::from(format!("designer-column-{name}")),
                                    name.clone(),
                                    ButtonKind::Quiet,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if let Some(session) = this.session_mut(session_id)
                                            && let Some(tab) =
                                                session.secondary_tabs.iter_mut().find(|tab| {
                                                    Some(tab.id) == session.active_secondary_tab
                                                })
                                            && let SecondaryTabKind::Structure(structure) =
                                                &mut tab.kind
                                            && let Some(designer) = &mut structure.designer
                                        {
                                            designer.fields[0].update(cx, |editor, cx| {
                                                editor.set_text(name.clone(), cx)
                                            });
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
                                    },
                                ))
                            }),
                        ))
                    },
                )
                .when(matches!(action, 0 | 5), |view| view.child(div().flex().flex_wrap().gap(px(4.)).children(
                    ["integer", "bigint", "text", "varchar(255)", "boolean", "numeric(12,2)", "timestamp"].into_iter().map(|value| {
                        button(SharedString::from(format!("designer-type-{value}")), value, ButtonKind::Quiet).on_click(cx.listener(move |this, _, _, cx| this.set_designer_field(session_id, 1, value.into(), cx)))
                    })
                )))
                .when(matches!(action, 3 | 6 | 7), |view| view.child(div().flex().flex_wrap().gap(px(4.)).children(
                    structure.columns.iter().map(|column| {
                        let name = column.name.clone();
                        button(SharedString::from(format!("designer-local-{name}")), name.clone(), ButtonKind::Quiet).on_click(cx.listener(move |this, _, _, cx| this.append_designer_column(session_id, 1, name.clone(), cx)))
                    })
                )))
                .when(action == 7, |view| view.child(div().max_h(px(120.)).id("designer-reference-tables").overflow_y_scroll().flex().flex_wrap().gap(px(4.)).children(
                    session.tables.iter().filter(|table| table.kind == EntityKind::Table).map(|table| {
                        let table = table.clone();
                        button(SharedString::from(format!("designer-ref-{:?}-{}", table.schema, table.name)), match &table.schema { Some(schema) => format!("{schema}.{}", table.name), None => table.name.clone() }, ButtonKind::Quiet).on_click(cx.listener(move |this, _, _, cx| {
                            this.set_designer_field(session_id, 2, table.name.clone(), cx);
                            this.set_designer_field(session_id, 4, table.schema.clone().unwrap_or_default(), cx);
                            this.set_designer_field(session_id, 3, String::new(), cx);
                        }))
                    })
                )).child(div().flex().flex_wrap().gap(px(4.)).children(
                    session.completion_columns.get(&completion_table_key(&TableRef { name: designer.fields[2].read(cx).text(cx).trim().into(), schema: { let schema = designer.fields[4].read(cx).text(cx); (!schema.trim().is_empty()).then(|| schema.trim().into()) } })).into_iter().flatten().map(|column| {
                        let name = column.name.clone();
                        button(SharedString::from(format!("designer-ref-column-{name}")), name.clone(), ButtonKind::Quiet).on_click(cx.listener(move |this, _, _, cx| this.append_designer_column(session_id, 3, name.clone(), cx)))
                    })
                )))
                .when(!designer.drafts.is_empty(), |view| view.child(button("designer-clear-plan", "Clear queued plan", ButtonKind::Quiet).on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(designer) = this.designer_mut(session_id) { designer.drafts.clear(); }
                    cx.notify();
                }))))
                .when(session.kind == DatabaseKind::MySQL && action == 5, |view| view.child(div().text_sm().text_color(theme().warning).child("MODIFY replaces the complete column definition. Existing AUTO_INCREMENT, COLLATE, COMMENT and ON UPDATE attributes are preserved from metadata. Generated columns require manual SQL.")))
                .child(
                    button(
                        "designer-queue",
                        format!("Add to plan ({} queued)", designer.drafts.len()),
                        ButtonKind::Quiet,
                    )
                    .disabled(designer.pending)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.build_designer_draft(session_id, true, window, cx)
                    })),
                )
                .child(
                    button("designer-draft", "Review SQL plan", ButtonKind::Primary).on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.draft_designer_for(session_id, window, cx)
                        }),
                    ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn designer_opens_a_reviewable_draft_without_altering_the_table(cx: &mut gpui::TestAppContext) {
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
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            session.engine = Some(engine);
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Structure(Box::new(StructureTab {
                    designer: None,
                    table: TableRef::new("items"),
                    columns: Vec::new(),
                    foreign_keys: Vec::new(),
                    indexes: Vec::new(),
                    checks: Vec::new(),
                    definition: None,
                    busy: false,
                    error: None,
                })),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Structure;
            app.sessions.push(session);
            app.active_session_id = Some(session_id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_designer_for(session_id, window, cx);
                let tab = &app.session(session_id).unwrap().secondary_tabs[0];
                let SecondaryTabKind::Structure(structure) = &tab.kind else {
                    panic!()
                };
                let designer = structure.designer.as_ref().unwrap();
                designer.fields[0].update(cx, |editor, cx| editor.set_text("display name", cx));
                designer.fields[1].update(cx, |editor, cx| editor.set_text("TEXT", cx));
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
