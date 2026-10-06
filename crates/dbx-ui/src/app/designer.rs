use super::*;
use crate::workspace::SavedQuery;

pub(super) struct Designer {
    fields: Vec<Entity<TextEditor>>,
    action: usize,
    nullable: bool,
    unique: bool,
}

impl DbxApp {
    pub(super) fn open_designer_for(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let fields = (0..4)
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
        let Some((kind, table, change)) = self.session(session_id).and_then(|session| {
            let tab = session
                .secondary_tabs
                .iter()
                .find(|tab| Some(tab.id) == session.active_secondary_tab)?;
            let SecondaryTabKind::Structure(structure) = &tab.kind else {
                return None;
            };
            let designer = structure.designer.as_ref()?;
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
                _ => dbx_core::TableAlteration::DropIndex {
                    name: values[0].trim().into(),
                },
            };
            Some((session.kind, structure.table.clone(), change))
        }) else {
            return;
        };
        match dbx_core::draft_table_alteration(kind, &table, &change) {
            Ok(sql) => self.open_saved_query_for(
                session_id,
                SavedQuery {
                    name: format!("Alter {}", table.name),
                    sql,
                },
                window,
                cx,
            ),
            Err(error) => self.show_toast(ToastKind::Error, error.to_string(), cx),
        }
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
            _ => vec!["Index to drop"],
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
                    div().flex().gap(px(6.)).children(
                        [
                            "Add column",
                            "Rename column",
                            "Drop column",
                            "Add index",
                            "Drop index",
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
                .when(action == 0 || action == 3, |view| {
                    view.child(
                        button(
                            "designer-option",
                            if action == 0 {
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
                                if action == 0 {
                                    designer.nullable = !designer.nullable;
                                } else {
                                    designer.unique = !designer.unique;
                                }
                            }
                            cx.notify();
                        })),
                    )
                })
                .child(
                    button("designer-draft", "Create SQL draft", ButtonKind::Primary).on_click(
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
