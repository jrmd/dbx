//! Keyboard focus traversal. Tab and Shift-Tab walk every tab stop in the
//! window; while a modal is open they stay inside it, so the workspace behind
//! the scrim can't be reached until the modal closes.

use super::*;

/// Tab and Shift-Tab walk every control; F6 does the same from widgets that
/// keep Tab for themselves, like the SQL editor's indent.
///
/// These carry no context on purpose. After a tab restores, focus can sit on a
/// handle that isn't painted, which leaves no key context to match. GPUI ranks
/// a context-free binding as the deepest and breaks ties by registration order,
/// so register these before the editor and grid bindings: indent and cell
/// commit-and-move then still win in their own widgets.
pub fn focus_key_bindings() -> Vec<gpui::KeyBinding> {
    vec![
        gpui::KeyBinding::new("tab", FocusNext, None),
        gpui::KeyBinding::new("shift-tab", FocusPrevious, None),
        gpui::KeyBinding::new("f6", FocusNext, None),
        gpui::KeyBinding::new("shift-f6", FocusPrevious, None),
    ]
}

/// Bound on wrap-around steps. Backgrounds hold a few dozen tab stops, so this
/// only stops a modal with no stops from spinning.
const MAX_FOCUS_STEPS: usize = 2048;

impl DbxApp {
    /// Whether the workspace is the screen being drawn, which is where the
    /// export dialog, mutation error and table menu live. Mirrors the choice
    /// `render` makes between settings, the connection list and the workspace.
    pub(super) fn workspace_screen_drawn(&self) -> bool {
        self.vault_state == Some(VaultState::Unlocked)
            && !self.settings_open
            && !self.connection_picker_open
            && self.active_session().is_some()
    }

    /// Focus handles of the open modals, topmost first. Each is tracked on its
    /// modal's container element. Only modals that are painted are listed: a
    /// handle nothing draws can't hold focus, and chasing one would fight
    /// `reclaim_stray_focus` every frame.
    pub(super) fn modal_traps(&self) -> Vec<FocusHandle> {
        if self.vault_state != Some(VaultState::Unlocked) {
            return Vec::new();
        }
        let mut traps = Vec::new();
        traps.extend(self.confirmation_dialog.as_ref().map(|d| d.focus.clone()));
        if self.workspace_screen_drawn() {
            traps.extend(self.mutation_error_dialog.as_ref().map(|d| d.focus.clone()));
        }
        traps.extend(self.data_import_dialog.as_ref().map(|d| d.focus.clone()));
        traps.extend(
            self.profile_transfer_dialog
                .as_ref()
                .map(|d| d.focus.clone()),
        );
        traps.extend(self.quick_open.as_ref().map(|d| d.focus.clone()));
        if self.workspace_screen_drawn() {
            traps.extend(
                self.database_export_dialog
                    .as_ref()
                    .map(|d| d.focus.clone()),
            );
            traps.extend(self.table_context_menu.as_ref().map(|m| m.focus.clone()));
        }
        traps
    }

    pub(super) fn modal_is_background(&self, focus: &FocusHandle) -> bool {
        self.modal_traps().first().is_some_and(|top| top != focus)
    }

    /// Pull focus back to the window root when it sits on a handle that isn't
    /// painted, such as the editor of a tab that was restored but isn't showing.
    /// With no focused element in the tree, shortcuts and Tab have nowhere to
    /// dispatch.
    ///
    /// Containment is judged against the last painted frame, so an element
    /// that was only just given focus looks stray for one render. The first
    /// sighting asks for another render; focus moves only if it's still
    /// outside once that frame has painted.
    pub(super) fn reclaim_stray_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.focus_handle.contains_focused(window, cx) {
            self.stray_focus_seen = false;
        } else if std::mem::replace(&mut self.stray_focus_seen, true) {
            self.stray_focus_seen = false;
            self.focus_handle.focus(window, cx);
        } else {
            cx.notify();
        }
    }

    /// Move focus into the topmost modal when it opens, and back into it after
    /// a click on its scrim lands focus on the window root.
    pub(super) fn ensure_modal_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let top = self.modal_traps().into_iter().next();
        let opened = top.is_some() && top != self.last_modal_trap;
        self.last_modal_trap = top.clone();
        let Some(trap) = top else {
            self.modal_focus_pending = false;
            return;
        };
        let on_root = match window.focused(cx) {
            None => true,
            Some(handle) => handle == self.focus_handle,
        };
        if on_root {
            self.modal_focus_pending = false;
            trap.focus(window, cx);
        } else if opened {
            // Not every opener focuses its modal, and focus may be on an editor
            // or menu behind the scrim. Check once the modal has painted, so one
            // that already focused a field of its own isn't overridden.
            self.modal_focus_pending = true;
            cx.notify();
        } else if std::mem::take(&mut self.modal_focus_pending)
            && !trap.contains_focused(window, cx)
        {
            trap.focus(window, cx);
        }
    }

    /// Tab (`forward`) or Shift-Tab. Wraps within the topmost modal, or across
    /// the whole window when none is open.
    pub(super) fn move_focus(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let step = |window: &mut Window, cx: &mut Context<Self>| {
            if forward {
                window.focus_next(cx);
            } else {
                window.focus_prev(cx);
            }
        };
        let Some(trap) = self.modal_traps().into_iter().next() else {
            step(window, cx);
            return;
        };
        if !trap.contains_focused(window, cx) {
            trap.focus(window, cx);
        }
        let start = window.focused(cx);
        step(window, cx);
        // Stepping past the modal's last (or before its first) stop lands in
        // the workspace; keep going until the walk wraps back inside.
        let mut steps = 0;
        while !trap.contains_focused(window, cx) && steps < MAX_FOCUS_STEPS {
            step(window, cx);
            steps += 1;
        }
        if !trap.contains_focused(window, cx) {
            // A modal without tab stops (a context menu) keeps its own focus.
            match start {
                Some(start) => start.focus(window, cx),
                None => trap.focus(window, cx),
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    fn bind_keys(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        cx.update(|cx| {
            // Same order as `main`.
            cx.bind_keys(focus_key_bindings());
            cx.bind_keys(editor::default_key_bindings());
        });
    }

    fn settle(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn focused(cx: &mut VisualTestContext) -> Option<FocusHandle> {
        cx.update(|window, cx| window.focused(cx))
    }

    fn distinct(handles: &[Option<FocusHandle>]) -> usize {
        let mut seen: Vec<&Option<FocusHandle>> = Vec::new();
        for handle in handles {
            if !seen.contains(&handle) {
                seen.push(handle);
            }
        }
        seen.len()
    }

    fn walk(cx: &mut VisualTestContext, keys: &str, steps: usize) -> Vec<Option<FocusHandle>> {
        (0..steps)
            .map(|_| {
                cx.simulate_keystrokes(keys);
                settle(cx);
                focused(cx)
            })
            .collect()
    }

    fn control_focus(cx: &mut VisualTestContext, selector: &str) -> FocusHandle {
        let bounds = cx
            .debug_bounds(Box::leak(selector.to_owned().into_boxed_str()))
            .expect("control must be visible");
        // Focus without clicking: releasing outside cancels the mouse click.
        cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, Default::default());
        cx.simulate_mouse_up(
            gpui::point(gpui::px(-10.), gpui::px(-10.)),
            gpui::MouseButton::Left,
            Default::default(),
        );
        settle(cx);
        focused(cx).expect("control must take focus")
    }

    #[gpui::test]
    fn keyboard_tab_close_keeps_focus_on_a_neighbor(cx: &mut TestAppContext) {
        bind_keys(cx);
        let session_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.sessions = vec![ConnectionSession::new(
                session_id,
                None,
                "Focus test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            )];
            app.active_session_id = Some(session_id);
            app.connection_picker_open = false;
            for _ in 0..3 {
                app.add_query_tab_for(session_id, window, cx);
            }
            app
        });
        settle(cx);
        let ids = cx.update(|_, cx| {
            app.read(cx)
                .session(session_id)
                .unwrap()
                .secondary_tabs
                .iter()
                .map(|tab| tab.id)
                .collect::<Vec<_>>()
        });
        control_focus(cx, &format!("close-document-{}", ids[2]));
        walk(cx, "tab", 1);
        let add_query = focused(cx).unwrap();
        walk(cx, "shift-tab", 1);
        walk(cx, "shift-tab", 1);
        let next = focused(cx).unwrap();
        let closing = control_focus(cx, &format!("close-document-{}", ids[1]));
        // Arrive through keyboard traversal, exactly as in the reported bug.
        cx.update(|window, cx| app.read(cx).focus_handle.clone().focus(window, cx));
        let mut reached = false;
        for _ in 0..100 {
            walk(cx, "f6", 1);
            if focused(cx) == Some(closing.clone()) {
                reached = true;
                break;
            }
        }
        assert!(
            reached,
            "keyboard traversal must reach the tab close button"
        );
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        for _ in 0..3 {
            settle(cx);
        }
        assert_eq!(
            cx.update(|_, cx| app
                .read(cx)
                .session(session_id)
                .unwrap()
                .secondary_tabs
                .len()),
            2
        );
        assert_eq!(
            focused(cx),
            Some(next),
            "closing a tab must keep focus beside it"
        );

        // Closing the last tab in the strip, including the only remaining tab,
        // leaves focus on New query so the keyboard workflow can continue.
        for id in [ids[2], ids[0]] {
            control_focus(cx, &format!("close-document-{id}"));
            cx.simulate_keystrokes("enter");
            cx.simulate_event(gpui::KeyUpEvent {
                keystroke: gpui::Keystroke::parse("enter").unwrap(),
            });
            for _ in 0..3 {
                settle(cx);
            }
            assert_eq!(focused(cx), Some(add_query.clone()));
        }
    }

    #[gpui::test]
    fn new_connection_steps_focus_the_drawn_field_and_keep_it(cx: &mut TestAppContext) {
        bind_keys(cx);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.vault_state = Some(VaultState::Unlocked);
                app.saved_connections.clear();
                app.begin_new_connection(cx);
            })
        });
        let import =
            cx.update(|_, cx| app.read(cx).draft.import_url_editor.read(cx).focus_handle());
        for _ in 0..3 {
            settle(cx);
            assert_eq!(focused(cx), Some(import.clone()));
        }
        cx.update(|_, cx| app.update(cx, |app, cx| app.select_kind(DatabaseKind::PostgreSQL, cx)));
        let name = cx.update(|_, cx| {
            app.read(cx)
                .draft
                .connection_name_editor
                .read(cx)
                .focus_handle()
        });
        for _ in 0..3 {
            settle(cx);
            assert_eq!(focused(cx), Some(name.clone()));
        }
        cx.simulate_keystrokes("tab");
        let moved = focused(cx);
        assert_ne!(moved, Some(name));
        settle(cx);
        assert_eq!(focused(cx), moved, "ordinary redraws must not steal focus");
        cx.update(|_, cx| app.update(cx, |app, cx| app.change_connection_kind(cx)));
        for _ in 0..3 {
            settle(cx);
            assert_eq!(focused(cx), Some(import.clone()));
        }
    }

    #[gpui::test]
    fn tab_walks_the_connection_form_fields(cx: &mut TestAppContext) {
        bind_keys(cx);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.vault_state = Some(VaultState::Unlocked);
                app.saved_connections.clear();
                app.select_kind(DatabaseKind::PostgreSQL, cx);
            })
        });
        settle(cx);

        let forward = walk(cx, "tab", 8);
        assert!(forward.iter().all(Option::is_some));
        assert!(
            distinct(&forward) >= 5,
            "Tab must reach the form's fields and buttons, saw {}",
            distinct(&forward)
        );

        // Shift-Tab retraces the same path.
        let back = walk(cx, "shift-tab", 1);
        assert_eq!(back[0], forward[forward.len() - 2]);
    }

    #[gpui::test]
    fn vim_setting_is_keyboard_operable_and_applies_to_sql_tabs(cx: &mut TestAppContext) {
        bind_keys(cx);
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            app.settings_store = Some(store.clone());
            app.vim_mode = false;
            app.sessions.clear();
            for kind in [DatabaseKind::SQLite, DatabaseKind::Redis] {
                let id = Uuid::new_v4();
                app.sessions.push(ConnectionSession::new(
                    id,
                    None,
                    "Keyboard test".into(),
                    kind,
                    None,
                    window,
                    cx,
                ));
                app.add_query_tab_for(id, window, cx);
            }
            app.active_session_id = Some(app.sessions[0].id);
            app.connection_picker_open = false;
            app.settings_section = SettingsSection::Editor;
            app.open_settings(window, cx);
            app
        });
        settle(cx);
        let switch = control_focus(cx, "settings-vim-mode");
        cx.update(|window, cx| app.read(cx).settings_focus.clone().focus(window, cx));
        let reached = (0..100).any(|_| walk(cx, "tab", 1)[0] == Some(switch.clone()));
        assert!(reached, "Tab must reach the Vim mode switch");
        cx.simulate_keystrokes("enter");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("enter").unwrap(),
        });
        settle(cx);
        assert!(cx.update(|_, cx| app.read(cx).vim_mode));
        assert!(store.load().unwrap().vim_mode);
        cx.update(|_, cx| {
            let app = app.read(cx);
            for (index, session) in app.sessions.iter().enumerate() {
                let SecondaryTabKind::Query(query) = &session.secondary_tabs[0].kind else {
                    unreachable!()
                };
                assert_eq!(query.query_editor.read(cx).vim_mode().is_some(), index == 0);
            }
        });
        cx.simulate_keystrokes("escape");
        settle(cx);
        let editor = cx.update(|_, cx| {
            let app = app.read(cx);
            let SecondaryTabKind::Query(query) = &app.sessions[0].secondary_tabs[0].kind else {
                unreachable!()
            };
            query.query_editor.clone()
        });
        cx.update(|window, cx| editor.read(cx).focus_handle().focus(window, cx));
        cx.update(|_, cx| editor.update(cx, |editor, cx| editor.set_text("SEL", cx)));
        settle(cx);
        assert!(
            cx.debug_bounds("sql-completion-menu").is_none(),
            "Normal mode must not show a typing completion menu"
        );
        cx.simulate_keystrokes("shift-a");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.vim_mode()),
            Some(editor::VimMode::Insert)
        );
        settle(cx);
        assert!(
            cx.debug_bounds("sql-completion-menu").is_some(),
            "Insert mode retains SQL completion"
        );
        cx.simulate_keystrokes("escape");
        assert_eq!(
            editor.read_with(cx, |editor, _| editor.vim_mode()),
            Some(editor::VimMode::Normal)
        );
        settle(cx);
        assert!(cx.debug_bounds("sql-completion-menu").is_none());
        cx.simulate_keystrokes("f6");
        assert_ne!(
            focused(cx),
            Some(editor.read_with(cx, |editor, _| editor.focus_handle()))
        );
    }

    #[gpui::test]
    fn tab_stays_inside_an_open_confirmation_dialog(cx: &mut TestAppContext) {
        bind_keys(cx);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        let focus = cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.vault_state = Some(VaultState::Unlocked);
                let focus = cx.focus_handle();
                app.confirmation_dialog = Some(ConfirmationDialog {
                    title: "Delete connection?".into(),
                    detail: "Gone for good.".into(),
                    confirm_label: "Delete",
                    tone: ConfirmationTone::Danger,
                    action: ConfirmationAction::LockVault,
                    focus: focus.clone(),
                    return_focus: None,
                    sql: None,
                });
                focus.focus(window, cx);
                focus
            })
        });
        settle(cx);

        for keys in ["tab", "shift-tab"] {
            let visited = walk(cx, keys, 9);
            for _ in &visited {
                cx.update(|window, cx| {
                    assert!(
                        focus.contains_focused(window, cx),
                        "{keys} moved focus out of the dialog"
                    )
                });
            }
            assert!(
                distinct(&visited) >= 3,
                "{keys} should cycle the close, cancel and confirm buttons, saw {}",
                distinct(&visited)
            );
            assert_eq!(
                visited[0], visited[3],
                "{keys} should wrap after the dialog's three buttons"
            );
        }
    }

    struct SqlFocusHarness {
        editor: Entity<TextEditor>,
        focus_moves: usize,
    }

    impl Render for SqlFocusHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let focus = self.editor.read(cx).focus_handle();
            div()
                .size_full()
                .on_action(cx.listener(|this, _: &FocusNext, _, _| this.focus_moves += 1))
                .child(editor::sql_input_fill(self.editor.clone(), focus, cx))
        }
    }

    #[gpui::test]
    fn tab_still_indents_in_the_sql_editor_and_f6_leaves_it(cx: &mut TestAppContext) {
        bind_keys(cx);
        let (harness, cx) = cx.add_window_view(|window, cx| {
            let value = cx.new(|_| String::new());
            let editor = cx.new(|cx| TextEditor::new_sql(value, None, window, cx));
            editor.read(cx).focus_handle().focus(window, cx);
            SqlFocusHarness {
                editor,
                focus_moves: 0,
            }
        });
        settle(cx);
        cx.simulate_keystrokes("tab");
        let (text, moves) = cx.update(|_, cx| {
            let harness = harness.read(cx);
            (harness.editor.read(cx).text(cx), harness.focus_moves)
        });
        assert!(!text.is_empty(), "Tab in the SQL editor must indent");
        assert_eq!(moves, 0, "Tab in the SQL editor must not move focus");

        cx.simulate_keystrokes("f6");
        let moves = cx.update(|_, cx| harness.read(cx).focus_moves);
        assert_eq!(moves, 1, "F6 must leave the SQL editor");
    }

    #[gpui::test]
    fn workspace_modals_are_not_trapped_while_the_workspace_is_hidden(cx: &mut TestAppContext) {
        bind_keys(cx);
        let (app, cx) = cx.add_window_view(DbxApp::new);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.vault_state = Some(VaultState::Unlocked);
                // No session: the connection list is the screen, so the
                // workspace's overlays aren't painted.
                app.table_context_menu = Some(TableContextMenu {
                    session_id: Uuid::new_v4(),
                    table: TableInfo::table("items", None),
                    position: gpui::point(gpui::px(0.), gpui::px(0.)),
                    cursor: None,
                    focus: cx.focus_handle(),
                });
                assert!(
                    app.modal_traps().is_empty(),
                    "a modal nothing paints must not claim focus"
                );
                // Quick open lives at the root, so it is always painted.
                app.open_quick_open_action(&OpenQuickOpen, window, cx);
                assert_eq!(app.modal_traps().len(), 1);
            })
        });
    }

    #[gpui::test]
    fn tab_leaves_the_data_grid_instead_of_selecting_the_next_column(cx: &mut TestAppContext) {
        bind_keys(cx);
        let session_id = Uuid::new_v4();
        let tab_id = Uuid::new_v4();
        let (app, cx) = cx.add_window_view(|window, cx| {
            let mut app = DbxApp::new(window, cx);
            app.vault_state = Some(VaultState::Unlocked);
            let mut session = ConnectionSession::new(
                session_id,
                None,
                "Grid test".into(),
                DatabaseKind::SQLite,
                None,
                window,
                cx,
            );
            session.tables = vec![TableInfo::table("items", None)];
            let mut id = ColumnInfo::result("id", 0, "INTEGER");
            id.primary_key = true;
            let columns = vec![id, ColumnInfo::result("name", 1, "TEXT")];
            let mut data =
                DataTab::new(session_id, tab_id, TableRef::new("items"), true, window, cx);
            data.table_columns = columns.clone();
            data.result_table = Some(data.table.clone());
            data.set_result(
                Some(QueryResult {
                    columns,
                    rows: vec![RowData::new(vec![
                        CellValue::Integer(1),
                        CellValue::Text("one".into()),
                    ])],
                    ..QueryResult::empty(None, 0)
                }),
                &session.tables,
                cx,
            );
            session.secondary_tabs.push(SecondaryTab {
                id: tab_id,
                kind: SecondaryTabKind::Data(Box::new(data)),
            });
            session.active_secondary_tab = Some(tab_id);
            session.pane = Pane::Data;
            app.sessions = vec![session];
            app.active_session_id = Some(session_id);
            app
        });
        settle(cx);
        let grid = cx.update(|_, cx| {
            let app = app.read(cx);
            let session = app.session(session_id).unwrap();
            match &session.secondary_tabs[0].kind {
                SecondaryTabKind::Data(data) => data.data_grid.read(cx).focus_handle(cx),
                _ => unreachable!(),
            }
        });
        cx.update(|window, cx| grid.focus(window, cx));
        settle(cx);
        cx.update(|window, _| assert!(grid.is_focused(window)));
        cx.simulate_keystrokes("tab");
        settle(cx);
        cx.update(|window, _| {
            assert!(
                !grid.is_focused(window),
                "Tab must leave the grid rather than move to the next column"
            )
        });
    }
}
