//! Table context-menu metadata and named operations.
use super::explorer::TableMenuCommand;
use super::*;

pub(super) struct TableNamePrompt {
    session_id: SessionId,
    table: TableInfo,
    duplicate: bool,
    name: Entity<TextEditor>,
    pub(super) focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    _subscription: Subscription,
}

fn quoted_table(kind: DatabaseKind, table: &TableRef) -> dbx_core::Result<String> {
    let name = dbx_core::quote_identifier(kind, &table.name)?;
    match &table.schema {
        Some(schema) => Ok(format!(
            "{}.{name}",
            dbx_core::quote_identifier(kind, schema)?
        )),
        None => Ok(name),
    }
}
fn named_table_sql(
    kind: DatabaseKind,
    table: &TableRef,
    name: &str,
    duplicate: bool,
) -> dbx_core::Result<String> {
    let source = quoted_table(kind, table)?;
    let target = quoted_table(
        kind,
        &TableRef {
            schema: table.schema.clone(),
            name: name.to_owned(),
        },
    )?;
    let short_name = dbx_core::quote_identifier(kind, name)?;
    Ok(if duplicate {
        match kind.dialect() {
            DatabaseKind::SqlServer => format!("SELECT * INTO {target} FROM {source};"),
            _ => format!("CREATE TABLE {target} AS SELECT * FROM {source};"),
        }
    } else {
        match kind.dialect() {
            DatabaseKind::MySQL | DatabaseKind::ClickHouse => {
                format!("RENAME TABLE {source} TO {target};")
            }
            DatabaseKind::SqlServer => format!(
                "EXEC sp_rename N'{}', N'{}';",
                source.replace('\'', "''"),
                name.replace('\'', "''")
            ),
            _ => format!("ALTER TABLE {source} RENAME TO {short_name};"),
        }
    })
}

impl DbxApp {
    pub(super) fn open_table_name_prompt(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        duplicate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name =
            cx.new(|cx| TextEditor::empty(false, window, cx).accessibility_label("New table name"));
        if duplicate {
            name.update(cx, |editor, cx| {
                editor.set_text(format!("{}_copy", table.name), cx)
            });
        }
        let subscription = cx.observe(&name, |_, _, cx| cx.notify());
        let return_focus = window.focused(cx);
        name.read(cx).focus_handle().focus(window, cx);
        self.table_name_prompt = Some(TableNamePrompt {
            session_id,
            table,
            duplicate,
            name,
            focus: cx.focus_handle(),
            return_focus,
            _subscription: subscription,
        });
        cx.notify();
    }
    pub(super) fn close_table_name_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(prompt) = self.table_name_prompt.take() {
            if let Some(focus) = prompt.return_focus {
                focus.focus(window, cx);
            }
            cx.notify();
        }
    }
    pub(super) fn submit_table_name_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = &self.table_name_prompt else {
            return;
        };
        let name = prompt.name.read(cx).text(cx);
        if name.is_empty() || name == prompt.table.name {
            return;
        }
        let Some(session) = self.session(prompt.session_id) else {
            return;
        };
        if session.busy
            || session
                .engine
                .as_ref()
                .is_none_or(|engine| engine.is_read_only())
        {
            return;
        }
        let sql = match named_table_sql(
            session.kind,
            &table_ref(&prompt.table),
            &name,
            prompt.duplicate,
        ) {
            Ok(sql) => sql,
            Err(error) => {
                self.show_toast(ToastKind::Error, error.to_string(), cx);
                return;
            }
        };
        let session_id = prompt.session_id;
        self.close_table_name_prompt(window, cx);
        // Put the exact operation in an editable SQL tab for review. Running it
        // follows the same execution, parameter and production-safety path.
        self.open_generated_table_sql(session_id, sql, window, cx);
    }
    fn open_generated_table_sql(
        &mut self,
        session_id: SessionId,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_session(session_id, cx);
        self.add_query_tab_for(session_id, window, cx);
        if let Some(query) = self.active_query_tab_mut(session_id) {
            query
                .query_editor
                .update(cx, |editor, cx| editor.set_text(sql, cx));
        }
        cx.notify();
    }
    pub(super) fn table_metadata_command(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        command: TableMenuCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(session_id) else {
            return;
        };
        let Some(engine) = session.engine.clone() else {
            return;
        };
        let expected_engine = engine.clone();
        let database = session.current_database.clone();
        let kind = session.kind;
        let task = self.runtime.spawn(async move {
            let target = table_ref(&table);
            let structure = engine.table_structure(&target).await?;
            let sql = match command {
                TableMenuCommand::CopyInsert => {
                    const COPY_LIMIT: usize = 1_000;
                    let result = engine
                        .query_table(
                            &target,
                            &[],
                            &[],
                            &[],
                            Some(Page {
                                limit: COPY_LIMIT as u32 + 1,
                                offset: 0,
                            }),
                            QueryOptions {
                                max_rows: Some(COPY_LIMIT + 1),
                            },
                        )
                        .await?;
                    let columns = result
                        .columns
                        .iter()
                        .map(|column| column.name.clone())
                        .collect::<Vec<_>>();
                    let limited = result.truncated || result.rows.len() > COPY_LIMIT;
                    let mut sql = if limited {
                        "-- First 1,000 rows only. Use Export data for the complete table.\n"
                            .to_owned()
                    } else {
                        String::new()
                    };
                    for row in result.rows.iter().take(COPY_LIMIT) {
                        sql.push_str(&dbx_core::render_sql_insert(
                            kind,
                            &target,
                            &columns,
                            &row.values,
                        )?);
                        sql.push_str(";\n");
                    }
                    if result.rows.is_empty() {
                        sql.push_str("-- The table contains no rows.\n");
                    }
                    sql
                }
                _ => {
                    if let Some(definition) = &structure.definition {
                        if table.kind == EntityKind::View
                            && !definition
                                .trim_start()
                                .to_ascii_uppercase()
                                .starts_with("CREATE")
                        {
                            format!(
                                "CREATE VIEW {} AS\n{definition};",
                                quoted_table(kind, &target)?
                            )
                        } else {
                            definition.clone()
                        }
                    } else {
                        let mut sql = dbx_core::render_sql_schema(
                            kind,
                            &target,
                            &structure,
                            std::slice::from_ref(&target),
                        )?;
                        for index in dbx_core::render_sql_indexes(kind, &target, &structure)? {
                            sql.push('\n');
                            sql.push_str(&index);
                        }
                        sql
                    }
                }
            };
            Ok::<_, dbx_core::DbxError>(sql)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await?;
            this.update_in(cx, |this, window, cx| {
                if !this.session(session_id).is_some_and(|session| session.current_database == database && session.engine.as_ref().is_some_and(|engine| Arc::ptr_eq(engine, &expected_engine))) { return; }
                match result {
                    Ok(sql) if command == TableMenuCommand::ViewDdl => this.open_generated_table_sql(session_id, sql, window, cx),
                    Ok(sql) => {
                        let message = if sql.starts_with("-- First 1,000") { "Copied first 1,000 rows as INSERT statements. Use Export data for all rows." } else { "Copied SQL" };
                        cx.write_to_clipboard(ClipboardItem::new_string(sql)); this.show_toast(ToastKind::Success, message, cx);
                    }
                    Err(error) => this.show_toast(ToastKind::Error, error.to_string(), cx),
                }
            })?;
            Ok::<(), anyhow::Error>(())
        }).detach();
    }
    pub(super) fn render_table_name_prompt(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(prompt) = &self.table_name_prompt else {
            return div().into_any_element();
        };
        let focus = prompt.focus.clone();
        let input = prompt.name.clone();
        let valid =
            !input.read(cx).text(cx).is_empty() && input.read(cx).text(cx) != prompt.table.name;
        let title = if prompt.duplicate {
            "Duplicate table"
        } else {
            "Rename table"
        };
        let detail = if prompt.duplicate {
            "Creates a SQL draft that copies the columns and rows. Indexes, constraints, triggers, and grants are not copied. Review and run the draft to apply it."
        } else {
            "Choose the new table name. Review and run the generated SQL draft to apply the rename."
        };
        let background = self.modal_is_background(&focus);
        deferred(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme().overlay)
                .child(
                    glass_raised(div(), RADIUS_GLASS)
                        .id("table-name-dialog")
                        .w(crate::theme::ui_px(440.))
                        .p(crate::theme::ui_px(18.))
                        .flex()
                        .flex_col()
                        .gap(crate::theme::ui_px(14.))
                        .child(title)
                        .child(div().text_color(theme().text_muted).child(detail))
                        .child(editor::input_with_key_context(
                            input.clone(),
                            input.read(cx).focus_handle(),
                            false,
                            "DbxTextEditor DbxTableName",
                            cx,
                        ))
                        .child(
                            div()
                                .flex()
                                .justify_end()
                                .gap(crate::theme::ui_px(8.))
                                .child(
                                    button("cancel-table-name", "Cancel", ButtonKind::Quiet)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.close_table_name_prompt(window, cx)
                                        })),
                                )
                                .child(
                                    button("review-table-name", "Review SQL", ButtonKind::Primary)
                                        .disabled(!valid)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.submit_table_name_prompt(window, cx)
                                        })),
                                ),
                        )
                        .focus_trap("table-name-focus", &focus)
                        .accessible_dialog("table-name-dialog", title, detail, background, true),
                ),
        )
        .with_priority(45)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_operations_quote_identifiers_and_keep_the_source_schema() {
        let source = TableRef {
            schema: Some("odd schema".into()),
            name: "a\"b".into(),
        };
        assert_eq!(
            named_table_sql(DatabaseKind::PostgreSQL, &source, "copy\"name", true).unwrap(),
            "CREATE TABLE \"odd schema\".\"copy\"\"name\" AS SELECT * FROM \"odd schema\".\"a\"\"b\";"
        );
        assert!(
            named_table_sql(DatabaseKind::MySQL, &source, "new`name", false)
                .unwrap()
                .contains("`new``name`")
        );
        assert!(
            named_table_sql(
                DatabaseKind::SqlServer,
                &source,
                "x'; DROP TABLE users;--",
                false
            )
            .unwrap()
            .contains("x''; DROP TABLE users;--")
        );
    }
    #[gpui::test]
    fn production_table_confirmation_guards_the_action_path_and_exact_name(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.runtime = Arc::new(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap(),
            );
            let mut session = ConnectionSession::new(
                id,
                None,
                "Production".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            session.tag = Some(crate::profiles::ConnectionTag {
                id: Uuid::new_v4(),
                name: "production".into(),
                color: 0xff0000,
            });
            app.sessions.push(session);
            app.active_session_id = Some(id);
            app
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.confirm_table_action(
                    TableAction::Drop,
                    id,
                    TableInfo::table("Orders", None),
                    window,
                    cx,
                );
                app.confirm_pending_action(window, cx);
                assert!(
                    app.confirmation_dialog.is_some(),
                    "Enter/action must not bypass an empty name"
                );
                for mismatch in ["orders", " Orders", "Orders ", "public.Orders"] {
                    app.table_confirmation_input
                        .as_ref()
                        .unwrap()
                        .1
                        .update(cx, |editor, cx| editor.set_text(mismatch, cx));
                    app.confirm_pending_action(window, cx);
                    assert!(app.confirmation_dialog.is_some(), "accepted {mismatch:?}");
                }
                app.table_confirmation_input
                    .as_ref()
                    .unwrap()
                    .1
                    .update(cx, |editor, cx| editor.set_text("Orders", cx));
                assert!(app.table_confirmation_matches(cx));
                app.confirm_pending_action(window, cx);
                assert!(app.confirmation_dialog.is_none());
                assert!(app.table_confirmation_input.is_none());
                // A name that confirmed one table must never authorize a batch.
                let tables = vec![
                    TableInfo::table("Orders", Some("public".into())),
                    TableInfo::table("Orders", Some("archive".into())),
                ];
                app.confirm_table_actions(TableAction::Truncate, id, tables.clone(), window, cx);
                let input = app.table_confirmation_input.as_ref().unwrap().1.clone();
                input.update(cx, |editor, cx| editor.set_text("Orders", cx));
                app.confirm_pending_action(window, cx);
                assert!(app.confirmation_dialog.is_some());
                input.update(cx, |editor, cx| {
                    editor.set_text(table_action_confirmation_text(&tables), cx)
                });
                assert!(app.table_confirmation_matches(cx));
                app.cancel_confirmation(window, cx);
                assert!(app.table_confirmation_input.is_none());
            })
        });
    }
}
