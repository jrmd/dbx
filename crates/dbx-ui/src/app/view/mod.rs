use gpui_component::ActiveTheme as _;
mod chrome;
mod connection;
mod data;
mod diagram;
mod overlays;
mod pane_resize;
mod query;
mod settings;

pub(super) use pane_resize::PaneResize;
use pane_resize::{GRID_MIN_WIDTH, INSPECTOR_MIN_WIDTH};

use super::*;

impl Render for DbxApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        self.native_glass.mark_dirty();
        #[cfg(target_os = "macos")]
        let (native_navigation, native_left, native_top) = (
            self.native_navigation_active(),
            self.native_glass.left_inset,
            self.native_glass.top_inset,
        );
        #[cfg(not(target_os = "macos"))]
        let (native_navigation, native_left, native_top) = (false, 0., 0.);
        let production = self.workspace_screen_drawn()
            && self
                .active_session()
                .and_then(|session| session.tag.as_ref())
                .is_some_and(ConnectionTag::is_production);
        if crate::theme::set_production_context(production) {
            crate::theme::sync_component_theme(Some(window), cx);
        }
        crate::theme::set_text_scales(self.ui_scale, self.editor_scale);
        crate::theme::set_font_sizes(self.font_size, self.mono_font_size, self.row_font_size);
        crate::theme::set_row_font_family(self.row_font_family.as_deref());
        {
            let theme = gpui_component::Theme::global_mut(cx);
            if let Some(family) = &self.font_family {
                theme.font_family = family.clone().into();
            }
            if let Some(family) = &self.mono_font_family {
                theme.mono_font_family = family.clone().into();
            }
            theme.font_size = px(16. * self.font_size as f32 / 12.);
            theme.mono_font_size = crate::theme::mono_text_px(13.);
        }
        window.set_rem_size(gpui::px(16. * crate::theme::ui_scale()));
        self.compact_layout = window.bounds().size.width < crate::theme::text_px(900.);
        self.narrow_workspace = window.bounds().size.width < crate::theme::text_px(1180.);
        // Preserve focused details when resizing into the compact layout,
        // before computing the explorer and main-pane arrangement.
        if self.narrow_workspace
            && let Some((session_id, tab_id)) = self.active_session().and_then(|session| {
                session
                    .active_data_tab_id()
                    .map(|tab_id| (session.id, tab_id))
            })
            && let Some(data) = self.data_tab_mut(session_id, tab_id)
            && data.inspector_open
            && data.inspector_focus.contains_focused(window, cx)
        {
            data.inspector_expanded = true;
        }
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
        self.reclaim_stray_focus(window, cx);
        self.ensure_modal_focus(window, cx);
        let unlocked = self.vault_state == Some(VaultState::Unlocked);
        if self.window_vault_unlocked != unlocked {
            self.window_vault_unlocked = unlocked;
            if let Some(handle) = window.window_handle().downcast::<DbxApp>() {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |app, window, _| {
                        if (app.vault_state == Some(VaultState::Unlocked)) == unlocked {
                            crate::window_geometry::apply(window, unlocked);
                        }
                    });
                });
            }
        }
        if std::mem::take(&mut self.connection_form_focus_pending)
            && unlocked
            && !self.settings_open
            && (self.connection_picker_open || self.active_session().is_none())
            && self.modal_traps().is_empty()
        {
            let editor = if self.draft.choosing_kind {
                &self.draft.import_url_editor
            } else if self.draft.creating_folder {
                &self.draft.new_folder_editor
            } else {
                &self.draft.connection_name_editor
            };
            editor.read(cx).focus_handle().focus(window, cx);
        }
        // The vault fields keep focus after unlocking even though they are no
        // longer drawn, which would leave every shortcut without a target.
        if unlocked
            && [
                &self.vault_editors.passphrase_editor,
                &self.vault_editors.confirmation_editor,
            ]
            .iter()
            .any(|editor| editor.read(cx).focus_handle().is_focused(window))
        {
            window.focus(&self.focus_handle, cx);
        }
        if unlocked && !self.startup_recovery_started {
            self.startup_recovery_started = true;
            self.restore_startup_workspace(window, cx);
        }
        self.validate_nonrelational_editor();
        let content = if !unlocked {
            self.render_connection(cx).into_any_element()
        } else if self.settings_open {
            self.render_settings(window, cx)
        } else if self.connection_picker_open || self.active_session().is_none() {
            self.render_connection(cx).into_any_element()
        } else {
            self.render_workspace(window, cx).into_any_element()
        };
        // The pane rail only makes sense once a connection is live.
        let connected = unlocked
            && self
                .active_session()
                .is_some_and(|session| session.engine.is_some());
        let modal_open = !self.modal_traps().is_empty();
        let workspace_drawn = self.workspace_screen_drawn();
        div()
            .id("dbx-app")
            .role(gpui::Role::Application)
            .aria_label(if production {
                format!(
                    "DBX — production connection {}",
                    self.active_session()
                        .map(|session| session.name.as_str())
                        .unwrap_or_default()
                )
            } else {
                "DBX".to_owned()
            })
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme().window)
            .font_family(cx.theme().font_family.clone())
            .text_size(px(16. * self.font_size as f32 / 12.))
            .text_color(theme().text)
            .track_focus(&self.focus_handle)
            .on_drop(
                cx.listener(|this, paths: &gpui::ExternalPaths, window, cx| {
                    if let Some(path) = paths.paths().first() {
                        this.open_connection_target(
                            path.to_string_lossy().into_owned(),
                            window,
                            cx,
                        );
                    }
                }),
            )
            .capture_key_down(cx.listener(|this, event, window, cx| {
                this.dismiss_overlay_on_escape(event, window, cx)
            }))
            .capture_key_down(cx.listener(Self::change_text_scale_on_key))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.row_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.row_drag = None),
            )
            .on_action(
                cx.listener(|this, _: &FocusNext, window, cx| this.move_focus(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusPrevious, window, cx| {
                    this.move_focus(false, window, cx)
                }),
            )
            .on_action(cx.listener(Self::refresh_action))
            .on_action(cx.listener(Self::new_connection_action))
            .on_action(cx.listener(Self::new_query_action))
            .on_action(cx.listener(Self::close_tab_action))
            .on_action(cx.listener(Self::next_connection_action))
            .on_action(cx.listener(Self::previous_connection_action))
            .on_action(cx.listener(Self::toggle_sidebar_action))
            .on_action(cx.listener(Self::open_quick_open_action))
            .on_action(cx.listener(Self::open_command_palette_action))
            .on_action(cx.listener(Self::open_sql_file_action))
            .on_action(cx.listener(|this, _: &ChooseTheme, window, cx| {
                this.run_palette_command("dbx_ui::ChooseTheme", window, cx)
            }))
            .on_action(cx.listener(|this, _: &LightAppearance, window, cx| {
                this.run_palette_command("dbx_ui::LightAppearance", window, cx)
            }))
            .on_action(cx.listener(|this, _: &DarkAppearance, window, cx| {
                this.run_palette_command("dbx_ui::DarkAppearance", window, cx)
            }))
            .on_action(cx.listener(|this, _: &SystemAppearance, window, cx| {
                this.run_palette_command("dbx_ui::SystemAppearance", window, cx)
            }))
            .on_action(cx.listener(|this, _: &BrowseSqlFiles, window, cx| {
                this.run_palette_command("dbx_ui::BrowseSqlFiles", window, cx)
            }))
            .on_action(cx.listener(|this, _: &FavoriteSqlFiles, window, cx| {
                this.run_palette_command("dbx_ui::FavoriteSqlFiles", window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveSqlFile, window, cx| {
                this.run_palette_command("dbx_ui::SaveSqlFile", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ChooseSqlFolder, window, cx| {
                this.run_palette_command("dbx_ui::ChooseSqlFolder", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExplainQuery, window, cx| {
                this.run_palette_command("dbx_ui::ExplainQuery", window, cx)
            }))
            .on_action(cx.listener(|this, _: &AnalyzeQuery, window, cx| {
                this.run_palette_command("dbx_ui::AnalyzeQuery", window, cx)
            }))
            .on_action(cx.listener(|this, _: &QueryStatistics, window, cx| {
                this.run_palette_command("dbx_ui::QueryStatistics", window, cx)
            }))
            .on_action(cx.listener(|this, _: &TableStatistics, window, cx| {
                this.run_palette_command("dbx_ui::TableStatistics", window, cx)
            }))
            .on_action(cx.listener(|this, _: &FullTableStatistics, window, cx| {
                this.run_palette_command("dbx_ui::FullTableStatistics", window, cx)
            }))
            .on_action(cx.listener(|this, _: &WatchQuery, window, cx| {
                this.run_palette_command("dbx_ui::WatchQuery", window, cx)
            }))
            .on_action(cx.listener(|this, _: &StopWatch, window, cx| {
                this.run_palette_command("dbx_ui::StopWatch", window, cx)
            }))
            .on_action(cx.listener(|this, _: &InspectSchema, window, cx| {
                this.run_palette_command("dbx_ui::InspectSchema", window, cx)
            }))
            .on_action(cx.listener(|this, _: &CaptureSchema, window, cx| {
                this.run_palette_command("dbx_ui::CaptureSchema", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportDatabase, window, cx| {
                this.run_palette_command("dbx_ui::ExportDatabase", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportQueryCsv, window, cx| {
                this.run_palette_command("dbx_ui::ExportQueryCsv", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportQueryXlsx, window, cx| {
                this.run_palette_command("dbx_ui::ExportQueryXlsx", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportFullQueryXlsx, window, cx| {
                this.run_palette_command("dbx_ui::ExportFullQueryXlsx", window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportFullTableXlsx, window, cx| {
                this.run_palette_command("dbx_ui::ExportFullTableXlsx", window, cx)
            }))
            .on_action(cx.listener(|this, _: &SubmitTableName, window, cx| {
                if this.tab_group_edit.is_some() {
                    this.submit_tab_group_edit(window, cx)
                } else {
                    this.submit_table_name_prompt(window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ConfirmAction, window, cx| {
                this.confirm_pending_action(window, cx)
            }))
            .on_action(cx.listener(Self::open_settings_action))
            .on_action(cx.listener(Self::open_shortcuts_action))
            .on_action(cx.listener(Self::search_history_action))
            .on_modifiers_changed(
                cx.listener(|this, event: &gpui::ModifiersChangedEvent, _, _| {
                    this.explorer_control_held = event.modifiers.control;
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, _| {
                this.explorer_control_held = event.modifiers.control;
            }))
            .on_action(cx.listener(Self::focus_explorer_action))
            .on_action(cx.listener(Self::next_tab_action))
            .on_action(cx.listener(Self::previous_tab_action))
            .on_action(cx.listener(Self::explorer_next_action))
            .on_action(cx.listener(Self::explorer_previous_action))
            .on_action(cx.listener(Self::explorer_first_action))
            .on_action(cx.listener(Self::explorer_last_action))
            .on_action(cx.listener(Self::explorer_open_action))
            .on_action(cx.listener(Self::explorer_context_menu_action))
            .on_action(cx.listener(Self::table_menu_next_action))
            .on_action(cx.listener(Self::table_menu_previous_action))
            .on_action(cx.listener(Self::table_menu_first_action))
            .on_action(cx.listener(Self::table_menu_last_action))
            .on_action(cx.listener(Self::table_menu_confirm_action))
            .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| this.check_for_updates(cx)))
            .child(
                div()
                    .id("app-background")
                    .role(gpui::Role::Pane)
                    .aria_label("DBX workspace")
                    .a11y_synthetic_children(move |builder| {
                        if modal_open {
                            builder.parent_node().set_hidden();
                        }
                    })
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(native_navigation, |view| {
                        // The same gutter separates the content from the
                        // floating native sidebar or, once it collapses (the
                        // inset drops to zero), from the window edge.
                        let gap = crate::theme::ui_px(GLASS_INSET).as_f32();
                        view.pt(px(native_top)).pl(px(native_left + gap))
                    })
                    .when(!native_navigation, |view| {
                        view.child(self.render_topbar(window, cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .flex()
                            .when(connected && !native_navigation, |view| {
                                view.child(self.render_app_rail(cx))
                            })
                            .when(!connected, |view| view.pl(crate::theme::ui_px(GLASS_INSET)))
                            .child(content),
                    ),
            )
            .child(self.render_toasts(cx))
            .when(workspace_drawn, |view| {
                view.child(self.render_table_context_menu(window, cx))
                    .child(self.render_database_export_dialog(window, cx))
                    .child(self.render_mutation_error_dialog(cx))
                    .child(self.render_value_popover(cx))
            })
            .when(unlocked, |view| {
                view.child(self.render_nonrelational_editor(cx))
                    .child(self.render_table_name_prompt(cx))
                    .child(self.render_quick_open(cx))
                    .child(self.render_confirmation_dialog(cx))
                    .child(self.render_profile_transfer(cx))
                    .child(self.render_data_import(cx))
            })
            .children(resize_edges(window))
    }
}

impl DbxApp {
    /// Glass capsules stacked above the status bar; click to dismiss early.
    fn render_toasts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let background = !self.modal_traps().is_empty();
        div()
            .id("notifications")
            .role(gpui::Role::Pane)
            .aria_label("Notifications")
            .a11y_synthetic_children(move |builder| {
                if background {
                    builder.parent_node().set_hidden();
                }
            })
            .absolute()
            .left_0()
            .right_0()
            .bottom(crate::theme::ui_px(76.))
            .flex()
            .flex_col()
            .items_center()
            .gap(crate::theme::ui_px(8.))
            .children(self.toasts.iter().map(|toast| {
                let id = toast.id;
                let accent = match toast.kind {
                    ToastKind::Info => theme().accent,
                    ToastKind::Success => theme().success,
                    ToastKind::Error => theme().danger,
                };
                glass_raised(
                    div()
                        .id(SharedString::from(format!("toast-{id}")))
                        .debug_selector(move || format!("toast-{id}")),
                    RADIUS_GLASS + 4.,
                )
                .rounded(crate::theme::ui_px(16.))
                .max_w(crate::theme::ui_px(520.))
                .min_h(crate::theme::control_height(36.))
                .py(crate::theme::ui_px(8.))
                .pl(crate::theme::ui_px(14.))
                .pr(crate::theme::ui_px(18.))
                .flex()
                .items_center()
                .gap(crate::theme::ui_px(10.))
                .occlude()
                .cursor_pointer()
                .text_size(crate::theme::text_px(12.))
                .font_weight(FontWeight::MEDIUM)
                .child(
                    div()
                        .size(crate::theme::ui_px(8.))
                        .flex_none()
                        .rounded_full()
                        .bg(accent),
                )
                // Keep dismissal outside the live region. Readers otherwise
                // prefer the button descendant and announce only "Dismiss
                // notification" instead of the operation's outcome.
                .child(
                    crate::accessibility::announcement(
                        SharedString::from(format!("toast-message-{id}")),
                        toast.message.clone(),
                        matches!(toast.kind, ToastKind::Error),
                    )
                    .min_w_0()
                    .whitespace_normal()
                    .child(toast.message.clone()),
                )
                .child(
                    glass_icon_button(("dismiss-toast", id), Icon::Close, false)
                        .aria_label("Dismiss notification")
                        .on_click(cx.listener(move |this, _, _, cx| this.dismiss_toast(id, cx))),
                )
                .tooltip(tip(toast.message.clone()))
                .on_hover(cx.listener(move |this, hovered, _, _| {
                    if let Some(toast) = this.toasts.iter_mut().find(|toast| toast.id == id) {
                        toast.hovered = *hovered;
                    }
                }))
                .on_click(cx.listener(move |this, _, _, cx| this.dismiss_toast(id, cx)))
            }))
    }
}

/// Invisible edge handles for client-decorated windows. Without these, a
/// floating DBX window on a compositor that leaves decorations to the client
/// (GNOME on Wayland, for example) could not be resized from its edges.
fn resize_edges(window: &Window) -> Vec<Stateful<Div>> {
    const EDGE: f32 = 5.;
    const CORNER: f32 = 12.;
    let Decorations::Client { tiling } = window.window_decorations() else {
        return Vec::new();
    };
    if window.is_maximized() || window.is_fullscreen() {
        return Vec::new();
    }
    let handle = |id: &'static str, edge: ResizeEdge, cursor: CursorStyle| {
        div()
            .id(id)
            .absolute()
            .occlude()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.start_window_resize(edge);
            })
    };
    let mut edges = Vec::new();
    if !tiling.top {
        edges.push(
            handle("resize-top", ResizeEdge::Top, CursorStyle::ResizeUpDown)
                .top_0()
                .left(crate::theme::ui_px(CORNER))
                .right(crate::theme::ui_px(CORNER))
                .h(crate::theme::ui_px(EDGE)),
        );
    }
    if !tiling.bottom {
        edges.push(
            handle(
                "resize-bottom",
                ResizeEdge::Bottom,
                CursorStyle::ResizeUpDown,
            )
            .bottom_0()
            .left(crate::theme::ui_px(CORNER))
            .right(crate::theme::ui_px(CORNER))
            .h(crate::theme::ui_px(EDGE)),
        );
    }
    if !tiling.left {
        edges.push(
            handle(
                "resize-left",
                ResizeEdge::Left,
                CursorStyle::ResizeLeftRight,
            )
            .left_0()
            .top(crate::theme::ui_px(CORNER))
            .bottom(crate::theme::ui_px(CORNER))
            .w(crate::theme::ui_px(EDGE)),
        );
    }
    if !tiling.right {
        edges.push(
            handle(
                "resize-right",
                ResizeEdge::Right,
                CursorStyle::ResizeLeftRight,
            )
            .right_0()
            .top(crate::theme::ui_px(CORNER))
            .bottom(crate::theme::ui_px(CORNER))
            .w(crate::theme::ui_px(EDGE)),
        );
    }
    let corners = [
        (
            "resize-top-left",
            ResizeEdge::TopLeft,
            CursorStyle::ResizeUpLeftDownRight,
            !tiling.top && !tiling.left,
        ),
        (
            "resize-top-right",
            ResizeEdge::TopRight,
            CursorStyle::ResizeUpRightDownLeft,
            !tiling.top && !tiling.right,
        ),
        (
            "resize-bottom-left",
            ResizeEdge::BottomLeft,
            CursorStyle::ResizeUpRightDownLeft,
            !tiling.bottom && !tiling.left,
        ),
        (
            "resize-bottom-right",
            ResizeEdge::BottomRight,
            CursorStyle::ResizeUpLeftDownRight,
            !tiling.bottom && !tiling.right,
        ),
    ];
    for (id, edge, cursor, enabled) in corners {
        if !enabled {
            continue;
        }
        let corner = handle(id, edge, cursor).size(crate::theme::ui_px(CORNER));
        edges.push(match edge {
            ResizeEdge::TopLeft => corner.top_0().left_0(),
            ResizeEdge::TopRight => corner.top_0().right_0(),
            ResizeEdge::BottomLeft => corner.bottom_0().left_0(),
            _ => corner.bottom_0().right_0(),
        });
    }
    edges
}
