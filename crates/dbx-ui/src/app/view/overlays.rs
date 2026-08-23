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
        let app = cx.entity();

        let table_rows =
            tables.into_iter().map(|table| {
                let key = table_selection_key(&table);
                let selected = selected_tables.contains(&key);
                let label = table_sidebar_label(&table, None);
                Button::new(SharedString::from(format!(
                    "database-export-{}",
                    table_sidebar_id(&table)
                )))
                .h(px(30.))
                .w_full()
                .px(px(8.))
                .rounded(px(5.))
                .with_size(Size::Small)
                .compact()
                .ghost()
                .selected(selected)
                .bg(if selected {
                    theme().accent_soft
                } else {
                    theme().panel
                })
                .text_color(if selected {
                    theme().text
                } else {
                    theme().text_muted
                })
                .cursor_pointer()
                .child(
                    div()
                        .size(px(14.))
                        .rounded(px(3.))
                        .border_1()
                        .border_color(if selected {
                            theme().accent
                        } else {
                            theme().border_strong
                        })
                        .bg(if selected {
                            theme().accent
                        } else {
                            theme().canvas
                        })
                        .flex(),
                )
                .child(icon(Icon::Table, theme().text_muted).size(px(14.)))
                .child(div().flex_1().truncate().child(label))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_database_export_table(key.clone(), cx)
                }))
            });

        let format_choices = [DumpFormat::Sql, DumpFormat::Csv, DumpFormat::Tsv]
            .into_iter()
            .map(|choice| {
                let selected = choice == format;
                Button::new(SharedString::from(format!(
                    "database-export-format-{choice}"
                )))
                .label(choice.to_string())
                .px(px(10.))
                .rounded(px(5.))
                .with_size(Size::XSmall)
                .compact()
                .ghost()
                .selected(selected)
                .bg(if selected {
                    theme().accent_soft
                } else {
                    theme().panel_raised
                })
                .text_color(if selected {
                    theme().accent
                } else {
                    theme().text_muted
                })
                .text_size(px(10.))
                .cursor_pointer()
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_database_export_format(choice, cx)),
                )
            });

        let schema_toggle = gpui_component::checkbox::Checkbox::new("database-export-schema-only")
            .label("Schema only")
            .checked(schema_only)
            .disabled(format != DumpFormat::Sql)
            .with_size(Size::Small)
            .on_click({
                let app = app.clone();
                move |_, _, cx| {
                    app.update(cx, |this, cx| this.toggle_database_export_schema_only(cx))
                }
            });

        let gzip_toggle = gpui_component::checkbox::Checkbox::new("database-export-gzip")
            .label("Gzip output")
            .checked(gzipped)
            .with_size(Size::Small)
            .on_click({
                let app = app.clone();
                move |_, _, cx| app.update(cx, |this, cx| this.toggle_database_export_gzip(cx))
            });

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
            .bg(theme().overlay)
            .child(
                div()
                    .id("database-export-dialog")
                    .w(px(560.))
                    .max_h(px(680.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme().border_strong)
                    .bg(theme().panel)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(14.))
                            .border_b_1()
                            .border_color(theme().border)
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
                                            .text_color(theme().text)
                                            .child("Export database"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(theme().text_muted)
                                            .child(destination_label),
                                    ),
                            )
                            .child(
                                Button::new("close-database-export")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Close export")
                                    .child(icon(Icon::Close, theme().text_muted))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_database_export(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .pt(px(14.))
                            .pb(px(8.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text_muted)
                                    .child(format!("TABLES · {selected_count} selected")),
                            )
                            .child(
                                Button::new("database-export-select-all")
                                    .label(select_all_label)
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .text_color(theme().accent)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_all_database_export_tables(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("database-export-table-list")
                            .mx(px(16.))
                            .h(px(190.))
                            .p(px(5.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(theme().border)
                            .bg(theme().canvas)
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .children(table_rows),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .pt(px(12.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text_muted)
                                    .child("OUTPUT FORMAT"),
                            )
                            .child(div().flex().gap(px(5.)).children(format_choices))
                            .child(div().flex().gap(px(16.)).child(schema_toggle).child(gzip_toggle)),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .pt(px(12.))
                            .flex()
                            .flex_col()
                            .gap(px(5.))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text_muted)
                                    .child("OUTPUT NAME"),
                            )
                            .child(editor::input(
                                output_name_editor,
                                output_name_focus.clone(),
                                false,
                            )),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(10.))
                                    .text_color(theme().text_muted)
                                    .truncate()
                                    .child(output_directory),
                            )
                            .child(
                                button(
                                    "database-export-choose-folder",
                                    "Choose folder…",
                                    ButtonKind::Quiet,
                                )
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose_database_export_directory(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(theme().border)
                            .bg(theme().panel_raised)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .max_w(px(340.))
                                    .text_size(px(9.))
                                    .text_color(theme().text_muted)
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
                                        button(
                                            "cancel-database-export",
                                            "Cancel",
                                            ButtonKind::Quiet,
                                        )
                                            .cursor_pointer()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.cancel_database_export(cx)
                                            })),
                                    )
                                    .child(
                                        button(
                                            "run-database-export",
                                            format!("Export {selected_count} table{}", if selected_count == 1 { "" } else { "s" }),
                                            ButtonKind::Primary,
                                        )
                                            .disabled(!can_export)
                                            .when(can_export, |button| {
                                                button.cursor_pointer().on_click(cx.listener(
                                                    |this, _, _, cx| {
                                                        this.execute_database_export(cx)
                                                    },
                                                ))
                                            }),
                                    ),
                            ),
                    )
                    .focus_trap("database-export-focus-trap", &output_name_focus),
            );

        deferred(overlay).with_priority(30).into_any_element()
    }

    pub(super) fn render_confirmation_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.confirmation_dialog.as_ref() else {
            return div().into_any_element();
        };
        let title = dialog.title.clone();
        let detail = dialog.detail.clone();
        let confirm_label = dialog.confirm_label;
        let tone = dialog.tone;
        let focus = dialog.focus.clone();
        let (tone_label, tone_color, confirm_kind) = match tone {
            ConfirmationTone::Warning => ("Review action", theme().warning, ButtonKind::Primary),
            ConfirmationTone::Danger => ("Destructive", theme().danger, ButtonKind::Danger),
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
            .bg(theme().overlay)
            .child(
                div()
                    .id("confirmation-dialog")
                    .w(px(420.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme().border_strong)
                    .bg(theme().panel)
                    .overflow_hidden()
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(14.))
                            .border_b_1()
                            .border_color(theme().border)
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(px(12.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(px(7.))
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme().text)
                                            .child(title),
                                    )
                                    .child(badge(tone_label, tone_color)),
                            )
                            .child(
                                Button::new("close-confirmation")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .tooltip("Cancel")
                                    .child(icon(Icon::Close, theme().text_muted))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_confirmation(window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(16.))
                            .text_size(px(12.))
                            .text_color(theme().text_muted)
                            .line_height(gpui::relative(1.5))
                            .child(detail),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(theme().border)
                            .bg(theme().panel_raised)
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                button("cancel-confirmation", "Cancel", ButtonKind::Quiet)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_confirmation(window, cx)
                                    })),
                            )
                            .child(
                                button("confirm-action", confirm_label, confirm_kind)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_pending_action(window, cx)
                                    })),
                            ),
                    )
                    .focus_trap("confirmation-focus-trap", &focus),
            );

        deferred(overlay).with_priority(40).into_any_element()
    }

    pub(super) fn render_mutation_error_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.mutation_error_dialog.as_ref() else {
            return div().into_any_element();
        };
        let title = dialog.title.clone();
        let detail = dialog.detail.clone();
        let focus = dialog.focus.clone();

        let overlay = div()
            .absolute()
            .top(px(0.))
            .right(px(0.))
            .bottom(px(0.))
            .left(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme().overlay)
            .child(
                div()
                    .id("mutation-error-dialog")
                    .w(px(460.))
                    .max_h(px(520.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(theme().border_strong)
                    .bg(theme().panel)
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(14.))
                            .border_b_1()
                            .border_color(theme().border)
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text)
                                    .child(title),
                            )
                            .child(badge("Save failed", theme().danger)),
                    )
                    .child(
                        div()
                            .id("mutation-error-scroll")
                            .flex_1()
                            .min_h_0()
                            .px(px(16.))
                            .py(px(16.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme().text_muted)
                                    .line_height(gpui::relative(1.5))
                                    .child(
                                        "This row change could not be applied. Your draft is still open.",
                                    ),
                            )
                            .child(
                                div()
                                    .p(px(12.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(theme().border)
                                    .bg(theme().canvas)
                                    .text_size(px(11.))
                                    .text_color(theme().danger)
                                    .line_height(gpui::relative(1.5))
                                    .child(detail),
                            ),
                    )
                    .child(
                        div()
                            .px(px(16.))
                            .py(px(12.))
                            .border_t_1()
                            .border_color(theme().border)
                            .bg(theme().panel_raised)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.))
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(px(9.))
                                    .text_color(theme().text_muted)
                                    .child("Correct the value or expression, then try again."),
                            )
                            .child(
                                button("dismiss-mutation-error", "Back to row", ButtonKind::Primary)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.dismiss_mutation_error(window, cx)
                                    })),
                            ),
                    )
                    .focus_trap("mutation-error-focus-trap", &focus),
            );

        deferred(overlay).with_priority(50).into_any_element()
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
                        .border_color(theme().border_strong)
                        .bg(theme().panel_raised)
                        .text_size(px(12.))
                        .on_mouse_down_out(
                            cx.listener(|this, _, _, cx| this.close_table_context_menu(cx)),
                        )
                        .child(
                            div()
                                .px(px(8.))
                                .pt(px(5.))
                                .pb(px(7.))
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme().text_muted)
                                .child(table_sidebar_label(&menu.table, None)),
                        )
                        .child(
                            div()
                                .id("context-open-structure")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(theme().accent_soft))
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
                                .id("context-open-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .hover(|style| style.bg(theme().accent_soft))
                                .child("Open data")
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
                                .hover(|style| style.bg(theme().accent_soft))
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
                        .child(div().my(px(4.)).border_t_1().border_color(theme().border))
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(4.))
                                .text_size(px(9.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme().text_muted)
                                .child("TRANSFER"),
                        )
                        .child(
                            div()
                                .id("context-export-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if transfer_enabled {
                                    theme().text
                                } else {
                                    theme().text_muted
                                })
                                .when(transfer_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(theme().accent_soft))
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
                                    theme().text
                                } else {
                                    theme().text_muted
                                })
                                .when(transfer_enabled, |view| {
                                    view.cursor_pointer()
                                        .hover(|style| style.bg(theme().accent_soft))
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
                        .child(div().my(px(4.)).border_t_1().border_color(theme().border))
                        .child(
                            div()
                                .px(px(8.))
                                .py(px(4.))
                                .text_size(px(9.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme().text_muted)
                                .child("DESTRUCTIVE"),
                        )
                        .child(
                            div()
                                .id("context-truncate-table")
                                .px(px(8.))
                                .py(px(7.))
                                .rounded(px(5.))
                                .text_color(if destructive_enabled {
                                    theme().warning
                                } else {
                                    theme().text_muted
                                })
                                .when(destructive_enabled, |view| {
                                    view.cursor_pointer().hover(|style| style.bg(theme().panel))
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
                                    theme().danger
                                } else {
                                    theme().text_muted
                                })
                                .when(destructive_enabled, |view| {
                                    view.cursor_pointer().hover(|style| style.bg(theme().panel))
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
