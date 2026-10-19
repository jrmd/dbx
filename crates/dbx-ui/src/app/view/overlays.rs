use super::super::*;
use crate::app::explorer::{MenuTone, TableMenuRow};
use gpui_component::ActiveTheme as _;

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
        let modal_focus = dialog.focus.clone();
        let output_name_focus = output_name_editor.read(cx).focus_handle();
        let output_name = dialog.output_name.read(cx).clone();
        let can_export = selected_count > 0 && !output_name.trim().is_empty();
        let app = cx.entity();
        let background = self.modal_is_background(&modal_focus);

        let table_rows =
            tables.into_iter().map(|table| {
                let key = table_selection_key(&table);
                let selected = selected_tables.contains(&key);
                let label = table_sidebar_label(&table, None);
                Button::new(SharedString::from(format!(
                    "database-export-{}",
                    table_sidebar_id(&table)
                )))
                .h(crate::theme::control_height(30.))
                .w_full()
                .px(crate::theme::ui_px(8.))
                .rounded(crate::theme::ui_px(5.))
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
                        .size(crate::theme::ui_px(14.))
                        .rounded(crate::theme::ui_px(3.))
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
                .child(icon(Icon::Table, theme().text_muted).size(crate::theme::ui_px(14.)))
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
                .px(crate::theme::ui_px(10.))
                .rounded(crate::theme::ui_px(5.))
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
                    theme().accent_text
                } else {
                    theme().text_muted
                })
                .text_size(crate::theme::text_px(10.))
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

        let overlay = div()
            .occlude()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .absolute()
            .top(crate::theme::ui_px(0.))
            .right(crate::theme::ui_px(0.))
            .bottom(crate::theme::ui_px(0.))
            .left(crate::theme::ui_px(0.))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme().overlay)
            .child(
                div()
                    .id("database-export-dialog")
                    .w(crate::theme::ui_px(560.))
                    .max_h(crate::theme::ui_px(680.))
                    .rounded(crate::theme::ui_px(RADIUS_GLASS))
                    .border_1()
                    .border_color(theme().hairline)
                    .bg(theme().glass_raised)
                    .shadow(glass_shadow(28.))
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(14.))
                            .border_b_1()
                            .border_color(theme().border)
                            .flex()
                            .items_start()
                            .justify_between()
                            .child(
                                div().flex().flex_col().child(
                                    div()
                                        .text_size(crate::theme::text_px(16.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme().text)
                                        .child("Export database"),
                                ),
                            )
                            .child(
                                Button::new("close-database-export")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .accessible_name("Close export")
                                    .tooltip("Close export")
                                    .child(icon(Icon::Close, theme().text_muted))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_database_export(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .pt(crate::theme::ui_px(14.))
                            .pb(crate::theme::ui_px(8.))
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(crate::theme::text_px(10.))
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
                                    .text_color(theme().accent_text)
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_all_database_export_tables(cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("database-export-table-list")
                            .mx(crate::theme::ui_px(16.))
                            .h(crate::theme::ui_px(190.))
                            .p(crate::theme::ui_px(5.))
                            .rounded(crate::theme::ui_px(6.))
                            .border_1()
                            .border_color(theme().border)
                            .bg(theme().canvas)
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(crate::theme::ui_px(2.))
                            .children(table_rows),
                    )
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .pt(crate::theme::ui_px(12.))
                            .flex()
                            .flex_col()
                            .gap(crate::theme::ui_px(6.))
                            .child(
                                div()
                                    .text_size(crate::theme::text_px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text_muted)
                                    .child("OUTPUT FORMAT"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(crate::theme::ui_px(5.))
                                    .children(format_choices),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(crate::theme::ui_px(16.))
                                    .child(schema_toggle)
                                    .child(gzip_toggle),
                            ),
                    )
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .pt(crate::theme::ui_px(12.))
                            .flex()
                            .flex_col()
                            .gap(crate::theme::ui_px(5.))
                            .child(
                                div()
                                    .text_size(crate::theme::text_px(10.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text_muted)
                                    .child("OUTPUT NAME"),
                            )
                            .child(editor::input(
                                output_name_editor,
                                output_name_focus.clone(),
                                false,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(12.))
                            .flex()
                            .items_center()
                            .gap(crate::theme::ui_px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(crate::theme::text_px(10.))
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
                                .on_click(cx.listener(
                                    |this, _, _, cx| this.choose_database_export_directory(cx),
                                )),
                            ),
                    )
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(12.))
                            .border_t_1()
                            .border_color(theme().border)
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div())
                            .child(
                                div()
                                    .flex()
                                    .gap(crate::theme::ui_px(7.))
                                    .child(
                                        button(
                                            "cancel-database-export",
                                            "Cancel",
                                            ButtonKind::Quiet,
                                        )
                                        .cursor_pointer()
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.cancel_database_export(window, cx);
                                            }),
                                        ),
                                    )
                                    .child(
                                        button(
                                            "run-database-export",
                                            format!(
                                                "Export {selected_count} table{}",
                                                if selected_count == 1 { "" } else { "s" }
                                            ),
                                            ButtonKind::Primary,
                                        )
                                        .disabled(!can_export)
                                        .when(
                                            can_export,
                                            |button| {
                                                button.cursor_pointer().on_click(cx.listener(
                                                    |this, _, window, cx| {
                                                        this.execute_database_export(window, cx)
                                                    },
                                                ))
                                            },
                                        ),
                                    ),
                            ),
                    )
                    .focus_trap("database-export-focus-trap", &modal_focus)
                    .accessible_dialog(
                        "export-dialog",
                        "Export database",
                        "Choose tables, output format, and destination.",
                        background,
                        false,
                    ),
            );

        deferred(overlay).with_priority(30).into_any_element()
    }

    pub(super) fn render_confirmation_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.confirmation_dialog.as_ref() else {
            return div().into_any_element();
        };
        let title = dialog.title.clone();
        let detail = dialog.detail.clone();
        let sql = dialog.sql.clone();
        let confirm_label = dialog.confirm_label;
        let confirmation_matches = self.table_confirmation_matches(cx);
        let typed_input = matches!(dialog.action, ConfirmationAction::Table { .. })
            .then(|| {
                self.table_confirmation_input
                    .as_ref()
                    .map(|(name, input, _)| (name.clone(), input.clone()))
            })
            .flatten();
        let tone = dialog.tone;
        let priority = if matches!(dialog.action, ConfirmationAction::LockVault) {
            60
        } else {
            40
        };
        let focus = dialog.focus.clone();
        let background = self.modal_is_background(&focus);
        let accessible_title = title.clone();
        let accessible_detail = match &sql {
            Some(sql) => format!("{detail}\nSQL: {sql}"),
            None => detail.clone(),
        };
        let confirm_kind = match tone {
            ConfirmationTone::Warning => ButtonKind::Primary,
            ConfirmationTone::Danger => ButtonKind::Danger,
        };

        let overlay = div()
            .occlude()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .id("confirmation-overlay")
            .absolute()
            .top(crate::theme::ui_px(0.))
            .right(crate::theme::ui_px(0.))
            .bottom(crate::theme::ui_px(0.))
            .left(crate::theme::ui_px(0.))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme().overlay)
            .child(
                div()
                    .id("confirmation-dialog")
                    .key_context("DbxConfirmation")
                    .debug_selector(|| "confirmation-dialog".into())
                    .w(crate::theme::ui_px(if sql.is_some() { 640. } else { 420. }))
                    .max_w(relative(0.95))
                    .max_h(relative(0.9))
                    .flex()
                    .flex_col()
                    .rounded(crate::theme::ui_px(RADIUS_GLASS))
                    .border_1()
                    .border_color(theme().hairline)
                    .bg(theme().glass_raised)
                    .shadow(glass_shadow(28.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(14.))
                            .border_b_1()
                            .border_color(theme().border)
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(crate::theme::ui_px(12.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .items_center()
                                    .gap(crate::theme::ui_px(7.))
                                    .child(
                                        div()
                                            .text_size(crate::theme::text_px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme().text)
                                            .child(title),
                                    ),
                            )
                            .child(
                                Button::new("close-confirmation")
                                    .with_size(Size::XSmall)
                                    .compact()
                                    .ghost()
                                    .accessible_name("Close confirmation")
                                    .tooltip("Cancel")
                                    .child(icon(Icon::Close, theme().text_muted))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_confirmation(window, cx)
                                    })),
                            ),
                    )
                    .when(!detail.is_empty(), |dialog| {
                        dialog.child(
                            div()
                                .px(crate::theme::ui_px(16.))
                                .py(crate::theme::ui_px(16.))
                                .text_size(crate::theme::text_px(12.))
                                .text_color(theme().text_muted)
                                .line_height(gpui::relative(1.5))
                                .child(detail),
                        )
                    })
                    .when_some(sql, |dialog, sql| {
                        dialog.child(
                            div()
                                .id("confirmation-sql")
                                .flex_1()
                                .min_h_0()
                                .overflow_scroll()
                                .px(crate::theme::ui_px(16.))
                                .py(crate::theme::ui_px(12.))
                                .bg(theme().canvas)
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(crate::theme::mono_text_px(11.))
                                .text_color(theme().text)
                                .line_height(gpui::relative(1.5))
                                .children(
                                    sql.lines()
                                        .map(|line| {
                                            div().whitespace_nowrap().child(line.to_owned())
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                        )
                    })
                    .when_some(typed_input, |dialog, (name, input)| {
                        dialog.child(
                            div()
                                .px(crate::theme::ui_px(16.))
                                .pb(crate::theme::ui_px(16.))
                                .flex()
                                .flex_col()
                                .gap(crate::theme::ui_px(8.))
                                .child(format!(
                                    "Production connection: type {name} exactly to confirm."
                                ))
                                .child(editor::input_with_key_context(
                                    input.clone(),
                                    input.read(cx).focus_handle(),
                                    false,
                                    "DbxTextEditor DbxTableConfirmation",
                                    cx,
                                )),
                        )
                    })
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(12.))
                            .border_t_1()
                            .border_color(theme().border)
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(crate::theme::ui_px(8.))
                            .child(
                                button("cancel-confirmation", "Cancel", ButtonKind::Quiet)
                                    .debug_selector(|| "cancel-confirmation".into())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.cancel_confirmation(window, cx)
                                    })),
                            )
                            .child(
                                button("confirm-action", confirm_label, confirm_kind)
                                    .disabled(!confirmation_matches)
                                    .debug_selector(|| "confirm-action".into())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_pending_action(window, cx)
                                    })),
                            ),
                    )
                    .focus_trap("confirmation-focus-trap", &focus)
                    .accessible_dialog(
                        "confirmation-dialog",
                        accessible_title,
                        accessible_detail,
                        background,
                        true,
                    ),
            );

        deferred(overlay).with_priority(priority).into_any_element()
    }

    pub(super) fn render_mutation_error_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(dialog) = self.mutation_error_dialog.as_ref() else {
            return div().into_any_element();
        };
        let title = dialog.title.clone();
        let detail = dialog.detail.clone();
        let focus = dialog.focus.clone();
        let background = self.modal_is_background(&focus);
        let accessible_title = title.clone();
        let accessible_detail = detail.clone();

        let overlay = div()
            .occlude()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .absolute()
            .top(crate::theme::ui_px(0.))
            .right(crate::theme::ui_px(0.))
            .bottom(crate::theme::ui_px(0.))
            .left(crate::theme::ui_px(0.))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme().overlay)
            .child(
                div()
                    .id("mutation-error-dialog")
                    .w(crate::theme::ui_px(460.))
                    .max_w(relative(0.95))
                    .max_h(relative(0.9))
                    .rounded(crate::theme::ui_px(RADIUS_GLASS))
                    .border_1()
                    .border_color(theme().hairline)
                    .bg(theme().glass_raised)
                    .shadow(glass_shadow(28.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(14.))
                            .border_b_1()
                            .border_color(theme().border)
                            .flex()
                            .items_center()
                            .gap(crate::theme::ui_px(8.))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .text_size(crate::theme::text_px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme().text)
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .id("mutation-error-scroll")
                            .flex_1()
                            .min_h_0()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(16.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(crate::theme::ui_px(12.))
                            .child(
                                div()
                                    .p(crate::theme::ui_px(12.))
                                    .rounded(crate::theme::ui_px(6.))
                                    .border_1()
                                    .border_color(theme().border)
                                    .bg(theme().canvas)
                                    .text_size(crate::theme::text_px(11.))
                                    .text_color(theme().danger_text)
                                    .line_height(gpui::relative(1.5))
                                    .child(detail),
                            ),
                    )
                    .child(
                        div()
                            .px(crate::theme::ui_px(16.))
                            .py(crate::theme::ui_px(12.))
                            .border_t_1()
                            .border_color(theme().border)
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(crate::theme::ui_px(12.))
                            .child(div())
                            .child(
                                button(
                                    "dismiss-mutation-error",
                                    "Back to row",
                                    ButtonKind::Primary,
                                )
                                .cursor_pointer()
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.dismiss_mutation_error(window, cx),
                                )),
                            ),
                    )
                    .focus_trap("mutation-error-focus-trap", &focus)
                    .accessible_dialog(
                        "mutation-error-dialog",
                        accessible_title,
                        accessible_detail,
                        background,
                        true,
                    ),
            );

        deferred(overlay).with_priority(50).into_any_element()
    }

    pub(super) fn render_table_context_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(menu) = self.table_context_menu.clone() else {
            return div().into_any_element();
        };
        let rows = self.table_menu_rows_for(&menu);

        let app = cx.entity().downgrade();
        let items = rows.into_iter().enumerate().map(|(index, row)| match row {
            TableMenuRow::Separator => div()
                .my(crate::theme::ui_px(4.))
                .border_t_1()
                .border_color(theme().border)
                .into_any_element(),
            TableMenuRow::Item {
                command,
                label,
                enabled,
                tone,
            } => {
                let highlighted = enabled && menu.cursor == Some(index);
                let highlight = if tone == MenuTone::Normal {
                    theme().accent_soft
                } else {
                    theme().panel
                };
                div()
                    .id(("table-menu-item", index))
                    .role(gpui::Role::MenuItem)
                    .aria_label(label)
                    .a11y_synthetic_children(move |builder| {
                        if !enabled {
                            builder.parent_node().set_disabled();
                        }
                    })
                    .when(highlighted && menu.focus.is_focused(window), |view| {
                        view.aria_active_descendant()
                    })
                    .on_a11y_action(gpui::AccessibleAction::Click, {
                        let app = app.clone();
                        move |_, window, cx| {
                            if enabled {
                                let _ = app.update(cx, |this, cx| {
                                    this.run_table_menu_command(command, window, cx)
                                });
                            }
                        }
                    })
                    .px(crate::theme::ui_px(8.))
                    .py(crate::theme::ui_px(7.))
                    .rounded(crate::theme::ui_px(5.))
                    .text_color(match (enabled, tone) {
                        (false, _) => theme().text_muted,
                        (true, MenuTone::Normal) => theme().text,
                        (true, MenuTone::Warning) => theme().warning_text,
                        (true, MenuTone::Danger) => theme().danger_text,
                    })
                    .when(highlighted, |view| view.bg(highlight))
                    .when(enabled, |view| {
                        view.cursor_pointer()
                            .hover(move |style| style.bg(highlight))
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_table_menu_command(command, window, cx)
                    }))
                    .into_any_element()
            }
        });

        let menu_view = deferred(
            anchored()
                .position(menu.position)
                .snap_to_window_with_margin(crate::theme::ui_px(8.))
                .child(
                    div()
                        .id("table-context-menu")
                        .role(gpui::Role::Menu)
                        .aria_label(table_sidebar_label(&menu.table, None))
                        .a11y_synthetic_children({
                            let background = self.modal_is_background(&menu.focus);
                            move |builder| {
                                if background {
                                    builder.parent_node().set_hidden();
                                }
                            }
                        })
                        .key_context("DbxTableMenu")
                        .track_focus(&menu.focus)
                        .occlude()
                        .w(crate::theme::ui_px(220.))
                        .p(crate::theme::ui_px(6.))
                        .rounded(crate::theme::ui_px(RADIUS_PANEL))
                        .border_1()
                        .border_color(theme().hairline)
                        .bg(theme().glass_raised)
                        .shadow(glass_shadow(18.))
                        .text_size(crate::theme::text_px(12.))
                        .on_mouse_down_out(
                            cx.listener(|this, _, _, cx| this.close_table_context_menu(cx)),
                        )
                        .child(
                            div()
                                .px(crate::theme::ui_px(8.))
                                .pt(crate::theme::ui_px(5.))
                                .pb(crate::theme::ui_px(7.))
                                .text_size(crate::theme::text_px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme().text_muted)
                                .child(table_sidebar_label(&menu.table, None)),
                        )
                        .children(items),
                ),
        )
        .with_priority(10);
        div()
            .child(crate::popups::scroll_shield(window, 9))
            .child(menu_view)
            .into_any_element()
    }
}
