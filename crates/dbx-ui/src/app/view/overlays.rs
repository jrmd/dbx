use super::super::*;

impl DbxApp {
    pub(super) fn render_database_export_dialog(
        &mut self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(dialog) = self.database_export_dialog.as_ref() else {
            return div().into_any_element();
        };
        let tables = dialog.tables.clone();
        let selected_tables = dialog.selected_tables.clone();
        let selected_count = selected_tables.len();
        let all_selected = selected_count == tables.len();
        let format = dialog.format;
        let schema_only = dialog.schema_only;
        let gzipped = dialog.gzipped;
        let output_directory = dialog.output_directory.display().to_string();
        let output_name_editor = dialog.output_name_editor.clone();
        let output_name_focus = output_name_editor.read(cx).focus_handle();
        let output_name = dialog.output_name.read(cx).clone();
        let can_export = selected_count > 0 && !output_name.trim().is_empty();

        let table_rows = tables.into_iter().map(|table| {
            let key = table_selection_key(&table);
            let selected = selected_tables.contains(&key);
            let label = table_sidebar_label(&table, None);
            div()
                .id(SharedString::from(format!(
                    "database-export-{}",
                    table_sidebar_id(&table)
                )))
                .h(px(30.))
                .px(px(8.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(if selected {
                    THEME.accent_soft
                } else {
                    THEME.panel
                })
                .text_color(if selected {
                    THEME.text
                } else {
                    THEME.text_muted
                })
                .cursor_pointer()
                .hover(|style| style.bg(THEME.panel_raised))
                .child(
                    div()
                        .size(px(14.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(if selected {
                            THEME.accent
                        } else {
                            THEME.border_strong
                        })
                        .bg(if selected { THEME.accent } else { THEME.canvas })
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(10.))
                        .text_color(THEME.canvas)
                        .child(if selected { "✓" } else { "" }),
                )
                .child(icon(Icon::Table, THEME.text_muted).size(px(14.)))
                .child(div().flex_1().truncate().child(label))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_database_export_table(key.clone(), cx)
                }))
        });

        let format_choices = [DumpFormat::Sql, DumpFormat::Csv, DumpFormat::Tsv]
            .into_iter()
            .map(|choice| {
                let selected = choice == format;
                div()
                    .id(SharedString::from(format!(
                        "database-export-format-{choice}"
                    )))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(5.))
                    .bg(if selected {
                        THEME.accent_soft
                    } else {
                        THEME.panel_raised
                    })
                    .text_color(if selected {
                        THEME.accent
                    } else {
                        THEME.text_muted
                    })
                    .text_size(px(10.))
                    .cursor_pointer()
                    .hover(|style| style.bg(THEME.accent_soft).text_color(THEME.text))
                    .child(choice.to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_database_export_format(choice, cx)
                    }))
            });

        let schema_toggle = div()
            .id("database-export-schema-only")
            .flex()
            .items_center()
            .gap(px(7.))
            .text_size(px(10.))
            .text_color(if format == DumpFormat::Sql {
                THEME.text
            } else {
                THEME.text_muted
            })
            .when(format == DumpFormat::Sql, |view| {
                view.cursor_pointer().on_click(
                    cx.listener(|this, _, _, cx| this.toggle_database_export_schema_only(cx)),
                )
            })
            .child(
                div()
                    .size(px(14.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(if schema_only && format == DumpFormat::Sql {
                        THEME.accent
                    } else {
                        THEME.border_strong
                    })
                    .bg(if schema_only && format == DumpFormat::Sql {
                        THEME.accent
                    } else {
                        THEME.canvas
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(10.))
                    .text_color(THEME.canvas)
                    .child(if schema_only && format == DumpFormat::Sql {
                        "✓"
                    } else {
                        ""
                    }),
            )
            .child("Schema only");

        let gzip_toggle = div()
            .id("database-export-gzip")
            .flex()
            .items_center()
            .gap(px(7.))
            .text_size(px(10.))
            .text_color(THEME.text)
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| this.toggle_database_export_gzip(cx)))
            .child(
                div()
                    .size(px(14.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(if gzipped {
                        THEME.accent
                    } else {
                        THEME.border_strong
                    })
                    .bg(if gzipped { THEME.accent } else { THEME.canvas })
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(10.))
                    .text_color(THEME.canvas)
                    .child(if gzipped { "✓" } else { "" }),
            )
            .child("Gzip output");

        let select_all_label = if all_selected {
            "Clear all"
        } else {
            "Select all"
        };
        let destination_label = if format == DumpFormat::Sql {
            format!(
                "One {} file{} · schema and data unless Schema only is selected",
                format.extension().to_ascii_uppercase(),
                if gzipped { " (gzip)" } else { "" }
            )
        } else {
            format!(
                "One {} file per table{}",
                format.extension().to_ascii_uppercase(),
                if gzipped { " (gzip)" } else { "" }
            )
        };

        let overlay = div()
            .absolute()
            .top(px(0.))
            .right(px(0.))
            .bottom(px(0.))
            .left(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::rgba(0x00000088))
            .child(
                div()
                    .id("database-export-dialog")
                    .w(px(560.))
                    .max_h(px(680.))
                    .p(px(18.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(THEME.border_strong)
                    .bg(THEME.panel_raised)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(THEME.text)
                                            .child("Export database"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(THEME.text_muted)
                                            .child(destination_label),
                                    ),
                            )
                            .child(
                                div()
                                    .id("close-database-export")
                                    .size(px(24.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|style| style.bg(THEME.panel))
                                    .child(icon(Icon::Close, THEME.text_muted).size(px(12.)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_database_export(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(THEME.text_muted)
                                    .child(format!("TABLES · {selected_count} selected")),
                            )
                            .child(
                                div()
                                    .id("database-export-select-all")
                                    .px(px(8.))
                                    .py(px(4.))
                                    .rounded(px(4.))
                                    .text_size(px(10.))
                                    .text_color(THEME.accent)
                                    .cursor_pointer()
                                    .hover(|style| style.bg(THEME.accent_soft))
                                    .child(select_all_label)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_all_database_export_tables(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("database-export-table-list")
                            .h(px(190.))
                            .p(px(5.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(THEME.border)
                            .bg(THEME.canvas)
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .children(table_rows),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(THEME.text_muted)
                                    .child("OUTPUT FORMAT"),
                            )
                            .child(div().flex().gap(px(5.)).children(format_choices))
                            .child(div().flex().gap(px(16.)).child(schema_toggle).child(gzip_toggle)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(THEME.text_muted)
                                    .child("OUTPUT NAME"),
                            )
                            .child(editor::input(output_name_editor, output_name_focus, false)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(10.))
                                    .text_color(THEME.text_muted)
                                    .truncate()
                                    .child(output_directory),
                            )
                            .child(
                                div()
                                    .id("database-export-choose-folder")
                                    .px(px(9.))
                                    .py(px(6.))
                                    .rounded(px(5.))
                                    .border_1()
                                    .border_color(THEME.border)
                                    .text_size(px(10.))
                                    .text_color(THEME.text)
                                    .cursor_pointer()
                                    .hover(|style| style.border_color(THEME.accent))
                                    .child("Choose folder…")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose_database_export_directory(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pt(px(4.))
                            .child(
                                div()
                                    .max_w(px(340.))
                                    .text_size(px(9.))
                                    .text_color(THEME.text_muted)
                                    .child(if format == DumpFormat::Sql {
                                        "SQL includes table columns, primary keys, and selected foreign keys."
                                    } else {
                                        "Delimited exports create one independently usable file per selected table."
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(7.))
                                    .child(
                                        div()
                                            .id("cancel-database-export")
                                            .px(px(10.))
                                            .py(px(7.))
                                            .rounded(px(5.))
                                            .border_1()
                                            .border_color(THEME.border)
                                            .text_size(px(10.))
                                            .text_color(THEME.text)
                                            .cursor_pointer()
                                            .child("Cancel")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.cancel_database_export(cx)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .id("run-database-export")
                                            .px(px(11.))
                                            .py(px(7.))
                                            .rounded(px(5.))
                                            .bg(if can_export {
                                                THEME.accent
                                            } else {
                                                THEME.panel
                                            })
                                            .text_size(px(10.))
                                            .text_color(if can_export {
                                                THEME.canvas
                                            } else {
                                                THEME.text_muted
                                            })
                                            .when(can_export, |view| {
                                                view.cursor_pointer().on_click(cx.listener(
                                                    |this, _, _, cx| {
                                                        this.execute_database_export(cx)
                                                    },
                                                ))
                                            })
                                            .child(format!("Export {selected_count} table{}", if selected_count == 1 { "" } else { "s" })),
                                    ),
                            ),
                    ),
            );

        deferred(overlay).with_priority(30).into_any_element()
    }

    pub(super) fn render_table_context_menu(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(menu) = self.table_context_menu.clone() else {
            return div().into_any_element();
        };
        let destructive_enabled = self.session(menu.session_id).is_some_and(|session| {
            session.kind.is_sql()
                && !session.busy
                && session.engine.is_some()
                && menu.table.kind == EntityKind::Table
        });
        let transfer_enabled = destructive_enabled;
        let open_table = menu.table.clone();
        let open_structure = menu.table.clone();
        let refresh_table = menu.table.clone();
        let export_table_item = menu.table.clone();
        let import_table_item = menu.table.clone();
        let truncate_table = menu.table.clone();
        let drop_table = menu.table.clone();
        let session_id = menu.session_id;

        deferred(
            anchored()
                .position(menu.position)
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id("table-context-menu")
                        .w(px(220.))
                        .p(px(6.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(THEME.border_strong)
                        .bg(THEME.panel_raised)
                        .text_size(px(12.))
                        .on_mouse_down_out(
                            cx.listener(|this, _, _, cx| this.close_table_context_menu(cx)),
                        )
                        .child(
                            div()
                                .id("context-open-structure")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.accent_soft))
                                .child("Open structure")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.table_context_menu = None;
                                    this.open_structure_tab_for(
                                        session_id,
                                        open_structure.clone(),
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(6.))
                                .text_size(px(10.))
                                .text_color(THEME.text_muted)
                                .child(table_sidebar_label(&menu.table, None)),
                        )
                        .child(
                            div()
                                .id("context-open-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.accent_soft))
                                .child("Open")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.table_context_menu = None;
                                    this.select_table_for(
                                        session_id,
                                        open_table.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            div()
                                .id("context-refresh-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(THEME.accent_soft))
                                .child("Refresh table")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.table_context_menu = None;
                                    this.select_table_for(
                                        session_id,
                                        refresh_table.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(div().my(px(4.)).border_t_1().border_color(THEME.border))
                        .child(
                            div()
                                .id("context-export-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if transfer_enabled {
                                    THEME.text
                                } else {
                                    THEME.text_muted
                                })
                                .when(transfer_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(THEME.accent_soft))
                                })
                                .child("Export data…")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if transfer_enabled {
                                        let table = export_table_item.clone();
                                        this.table_context_menu = None;
                                        this.begin_table_export(session_id, table, cx);
                                    }
                                })),
                        )
                        .child(
                            div()
                                .id("context-import-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if transfer_enabled {
                                    THEME.text
                                } else {
                                    THEME.text_muted
                                })
                                .when(transfer_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(THEME.accent_soft))
                                })
                                .child("Import data…")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if transfer_enabled {
                                        let table = import_table_item.clone();
                                        this.table_context_menu = None;
                                        this.begin_table_import(session_id, table, window, cx);
                                    }
                                })),
                        )
                        .child(div().my(px(4.)).border_t_1().border_color(THEME.border))
                        .child(
                            div()
                                .id("context-truncate-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if destructive_enabled {
                                    THEME.warning
                                } else {
                                    THEME.text_muted
                                })
                                .when(destructive_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(THEME.accent_soft))
                                })
                                .child("Truncate table…")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if destructive_enabled {
                                        this.confirm_table_action(
                                            TableAction::Truncate,
                                            session_id,
                                            truncate_table.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        )
                        .child(
                            div()
                                .id("context-delete-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if destructive_enabled {
                                    THEME.danger
                                } else {
                                    THEME.text_muted
                                })
                                .when(destructive_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(THEME.accent_soft))
                                })
                                .child("Delete table…")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if destructive_enabled {
                                        this.confirm_table_action(
                                            TableAction::Drop,
                                            session_id,
                                            drop_table.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        ),
                ),
        )
        .with_priority(10)
        .into_any_element()
    }
}
