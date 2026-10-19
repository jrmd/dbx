//! Keyboard navigation for the table explorer, its context menu, and the tabs
//! of the active connection.
use super::*;
use gpui::{KeyBinding, ScrollStrategy};

const EXPLORER_CONTEXT: &str = "DbxExplorer";
const TABLE_MENU_CONTEXT: &str = "DbxTableMenu";

/// Key bindings for explorer, context-menu and tab navigation. Shared by the
/// application and its tests so both exercise the same key routing.
pub(crate) fn key_bindings() -> Vec<KeyBinding> {
    let explorer = Some(EXPLORER_CONTEXT);
    let menu = Some(TABLE_MENU_CONTEXT);
    vec![
        KeyBinding::new("down", ExplorerNext, explorer),
        KeyBinding::new("up", ExplorerPrevious, explorer),
        KeyBinding::new("home", ExplorerFirst, explorer),
        KeyBinding::new("end", ExplorerLast, explorer),
        KeyBinding::new("enter", ExplorerOpen, explorer),
        KeyBinding::new("shift-f10", ExplorerContextMenu, explorer),
        KeyBinding::new("menu", ExplorerContextMenu, explorer),
        KeyBinding::new("down", TableMenuNext, menu),
        KeyBinding::new("up", TableMenuPrevious, menu),
        KeyBinding::new("home", TableMenuFirst, menu),
        KeyBinding::new("end", TableMenuLast, menu),
        KeyBinding::new("enter", TableMenuConfirm, menu),
        KeyBinding::new("cmd-shift-e", FocusExplorer, None),
        KeyBinding::new("ctrl-shift-e", FocusExplorer, None),
        KeyBinding::new("alt-pagedown", NextTab, None),
        KeyBinding::new("alt-pageup", PreviousTab, None),
        KeyBinding::new("cmd-alt-right", NextTab, None),
        KeyBinding::new("cmd-alt-left", PreviousTab, None),
    ]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Motion {
    Next,
    Previous,
    First,
    Last,
}

/// Where a cursor over `enabled` rows lands after `motion`. Disabled rows
/// (and separators) are skipped; stepping past either end wraps.
pub(super) fn move_cursor(enabled: &[bool], from: Option<usize>, motion: Motion) -> Option<usize> {
    let count = enabled.len();
    let first = enabled.iter().position(|enabled| *enabled)?;
    let last = enabled.iter().rposition(|enabled| *enabled)?;
    match motion {
        Motion::First => Some(first),
        Motion::Last => Some(last),
        Motion::Next => match from {
            None => Some(first),
            Some(from) => (1..=count)
                .map(|offset| (from + offset) % count)
                .find(|index| enabled[*index]),
        },
        Motion::Previous => match from {
            None => Some(last),
            Some(from) => (1..=count)
                .map(|offset| (from + count - offset % count) % count)
                .find(|index| enabled[*index]),
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TableMenuCommand {
    OpenStructure,
    OpenData,
    CopyName,
    ViewDdl,
    CopyDdl,
    Rename,
    Duplicate,
    CopyInsert,
    ColumnStatistics,
    FullColumnStatistics,
    Refresh,
    Export,
    Import,
    Capture,
    Append,
    Compare,
    Truncate,
    Drop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuTone {
    Normal,
    Warning,
    Danger,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TableMenuRow {
    Item {
        command: TableMenuCommand,
        label: &'static str,
        enabled: bool,
        tone: MenuTone,
    },
    Separator,
}

impl TableMenuRow {
    fn is_enabled_item(&self) -> bool {
        matches!(self, Self::Item { enabled: true, .. })
    }
}

/// The context menu's rows. `writable` gates the items that change or move
/// table data; opening and refreshing are always available.
pub(super) fn table_menu_rows(writable: bool) -> Vec<TableMenuRow> {
    use MenuTone::*;
    use TableMenuCommand::*;
    let item = |command, label, enabled, tone| TableMenuRow::Item {
        command,
        label,
        enabled,
        tone,
    };
    vec![
        item(OpenStructure, "Open structure", true, Normal),
        item(OpenData, "Open data", true, Normal),
        item(CopyName, "Copy name", true, Normal),
        item(ViewDdl, "View DDL", true, Normal),
        item(CopyDdl, "Copy DDL", true, Normal),
        item(CopyInsert, "Copy INSERT (up to 1,000 rows)", true, Normal),
        item(
            ColumnStatistics,
            "Column statistics (loaded rows)",
            true,
            Normal,
        ),
        item(
            FullColumnStatistics,
            "Column statistics (full table, scans all rows)",
            true,
            Normal,
        ),
        item(Rename, "Rename…", writable, Normal),
        item(Duplicate, "Duplicate…", writable, Normal),
        item(Refresh, "Refresh table", true, Normal),
        TableMenuRow::Separator,
        item(Export, "Export data…", writable, Normal),
        item(Import, "Import data…", writable, Normal),
        item(
            Capture,
            "Capture table for cross-connection copy",
            true,
            Normal,
        ),
        item(Append, "Append captured data…", true, Normal),
        item(Compare, "Compare with captured data…", true, Normal),
        TableMenuRow::Separator,
        item(Truncate, "Truncate table…", writable, Warning),
        item(Drop, "Delete table…", writable, Danger),
    ]
}

impl DbxApp {
    fn workspace_visible(&self) -> bool {
        !self.settings_open
            && !self.connection_picker_open
            && self
                .active_session()
                .is_some_and(|session| session.engine.is_some())
    }

    /// The row the keyboard cursor is on: the stored one if it is still
    /// listed, otherwise the open table.
    fn explorer_anchor(&self, session_id: SessionId) -> Option<usize> {
        let session = self.session(session_id)?;
        let visible = &session.sidebar.list.visible;
        session
            .sidebar
            .cursor
            .as_deref()
            .and_then(|id| {
                visible
                    .iter()
                    .position(|table| table_sidebar_id(table) == id)
            })
            .or_else(|| {
                let open = &session.active_data_tab()?.table;
                visible
                    .iter()
                    .position(|table| table.name == open.name && table.schema == open.schema)
            })
    }

    /// The row to draw the cursor on while the explorer has focus.
    pub(super) fn explorer_cursor_id(&self, session_id: SessionId) -> Option<String> {
        let session = self.session(session_id)?;
        let visible = &session.sidebar.list.visible;
        let index = self.explorer_anchor(session_id).unwrap_or(0);
        visible.get(index).map(table_sidebar_id)
    }

    pub(super) fn set_explorer_cursor(
        &mut self,
        session_id: SessionId,
        table: &TableInfo,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session_mut(session_id) {
            session.sidebar.cursor = Some(table_sidebar_id(table));
            cx.notify();
        }
    }

    pub(super) fn scroll_explorer_accessibly(
        &mut self,
        session_id: SessionId,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_session_id != Some(session_id) {
            return;
        }
        let anchor = self.explorer_anchor(session_id).unwrap_or(0);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let rows = &session.sidebar.list.visible;
        if rows.is_empty() {
            return;
        }
        let target = anchor.saturating_add_signed(delta).min(rows.len() - 1);
        session.sidebar.cursor = Some(table_sidebar_id(&rows[target]));
        session
            .sidebar
            .scroll
            .scroll_to_item(target, ScrollStrategy::Nearest);
        session.sidebar.focus.focus(window, cx);
        cx.notify();
    }

    fn move_explorer_cursor(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let Some(session_id) = self.active_session_id else {
            return;
        };
        let anchor = self.explorer_anchor(session_id);
        let Some(session) = self.session_mut(session_id) else {
            return;
        };
        let rows = session.sidebar.list.visible.clone();
        if rows.is_empty() {
            return;
        }
        let enabled = vec![true; rows.len()];
        // Without a cursor or an open table, the first press lands on row one.
        let from = anchor;
        let Some(target) = move_cursor(&enabled, from, motion) else {
            return;
        };
        // The list ends rather than wrapping, unlike menus.
        let target = match (motion, from) {
            (Motion::Next, Some(from)) => (from + 1).min(rows.len() - 1),
            (Motion::Previous, Some(from)) => from.saturating_sub(1),
            _ => target,
        };
        session.sidebar.cursor = Some(table_sidebar_id(&rows[target]));
        session
            .sidebar
            .scroll
            .scroll_to_item(target, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Down arrow in the search box continues into the list.
    pub(super) fn focus_explorer_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session_id) = self.active_session_id else {
            return;
        };
        if self.explorer_anchor(session_id).is_none() {
            self.move_explorer_cursor(Motion::First, cx);
        }
        if let Some(session) = self.session(session_id) {
            session.sidebar.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn cursor_table(&self, session_id: SessionId) -> Option<TableInfo> {
        let session = self.session(session_id)?;
        let index = self.explorer_anchor(session_id)?;
        session.sidebar.list.visible.get(index).cloned()
    }

    pub(super) fn focus_explorer_action(
        &mut self,
        _: &FocusExplorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_visible() {
            return;
        }
        self.sidebar_hidden = false;
        self.focus_explorer_list(window, cx);
    }

    pub(super) fn explorer_next_action(
        &mut self,
        _: &ExplorerNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_explorer_cursor(Motion::Next, cx);
    }

    pub(super) fn explorer_previous_action(
        &mut self,
        _: &ExplorerPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_explorer_cursor(Motion::Previous, cx);
    }

    pub(super) fn explorer_first_action(
        &mut self,
        _: &ExplorerFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_explorer_cursor(Motion::First, cx);
    }

    pub(super) fn explorer_last_action(
        &mut self,
        _: &ExplorerLast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_explorer_cursor(Motion::Last, cx);
    }

    /// Enter opens the table exactly as a click does.
    pub(super) fn explorer_open_action(
        &mut self,
        _: &ExplorerOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.active_session_id else {
            return;
        };
        if let Some(table) = self.cursor_table(session_id) {
            self.set_explorer_cursor(session_id, &table, cx);
            self.select_table_for(session_id, table, window, cx);
        }
    }

    pub(super) fn explorer_context_menu_action(
        &mut self,
        _: &ExplorerContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.active_session_id else {
            return;
        };
        let Some(table) = self.cursor_table(session_id) else {
            return;
        };
        let Some(session) = self.session(session_id) else {
            return;
        };
        // Anchor under the cursor row. Bounds from an earlier frame may belong
        // to another row (the list may have just scrolled), so fall back to
        // the list's corner rather than a guess.
        let id = table_sidebar_id(&table);
        let row = session.sidebar.cursor_bounds.take();
        let position = match row {
            Some((row_id, bounds)) if row_id == id => point(
                bounds.origin.x + crate::theme::ui_px(28.),
                bounds.origin.y + bounds.size.height,
            ),
            _ => session.sidebar.list_bounds.get().map_or(
                point(crate::theme::ui_px(80.), crate::theme::ui_px(120.)),
                |bounds| {
                    point(
                        bounds.origin.x + crate::theme::ui_px(28.),
                        bounds.origin.y + crate::theme::ui_px(28.),
                    )
                },
            ),
        };
        self.open_table_context_menu(session_id, table, position, window, cx);
        if let Some(menu) = &mut self.table_context_menu {
            let rows = table_menu_rows(true);
            let enabled = rows
                .iter()
                .map(TableMenuRow::is_enabled_item)
                .collect::<Vec<_>>();
            menu.cursor = move_cursor(&enabled, None, Motion::First);
        }
        cx.notify();
    }

    pub(super) fn open_table_context_menu(
        &mut self,
        session_id: SessionId,
        table: TableInfo,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.table_context_menu = Some(TableContextMenu {
            session_id,
            table,
            position,
            cursor: None,
            focus,
        });
        cx.notify();
    }

    /// Escape: close the menu and put focus back on the explorer.
    pub(super) fn dismiss_table_context_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(menu) = self.table_context_menu.take() else {
            return false;
        };
        self.focus_session_explorer(menu.session_id, window, cx);
        true
    }

    fn focus_session_explorer(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.session(session_id) {
            Some(session) => session.sidebar.focus.focus(window, cx),
            None => self.focus_handle.focus(window, cx),
        }
    }

    /// Rows for `menu`, with write-capable items enabled only when the
    /// connection can take them.
    pub(super) fn table_menu_rows_for(&self, menu: &TableContextMenu) -> Vec<TableMenuRow> {
        let writable = self.session(menu.session_id).is_some_and(|session| {
            session.kind.is_sql()
                && !session.busy
                && session
                    .engine
                    .as_ref()
                    .is_some_and(|engine| !engine.is_read_only())
                && menu.table.kind == EntityKind::Table
        });
        let sql = self
            .session(menu.session_id)
            .is_some_and(|session| session.kind.is_sql());
        table_menu_rows(writable)
            .into_iter()
            .map(|row| match row {
                TableMenuRow::Item {
                    command: TableMenuCommand::Export,
                    label,
                    tone,
                    ..
                } => TableMenuRow::Item {
                    command: TableMenuCommand::Export,
                    label,
                    enabled: sql
                        && self
                            .session(menu.session_id)
                            .is_some_and(|session| !session.busy && session.engine.is_some()),
                    tone,
                },
                TableMenuRow::Item {
                    command,
                    label,
                    enabled,
                    tone,
                } if matches!(
                    command,
                    TableMenuCommand::ViewDdl
                        | TableMenuCommand::CopyDdl
                        | TableMenuCommand::CopyInsert
                ) =>
                {
                    TableMenuRow::Item {
                        command,
                        label,
                        enabled: enabled && sql,
                        tone,
                    }
                }
                TableMenuRow::Item {
                    command: TableMenuCommand::ColumnStatistics,
                    label,
                    enabled,
                    tone,
                } => TableMenuRow::Item {
                    command: TableMenuCommand::ColumnStatistics,
                    label,
                    enabled: enabled
                        && self
                            .session(menu.session_id)
                            .and_then(|session| session.active_data_tab())
                            .is_some_and(|data| {
                                data.table.name == menu.table.name
                                    && data.table.schema == menu.table.schema
                                    && data.result.is_some()
                            }),
                    tone,
                },
                TableMenuRow::Item {
                    command: TableMenuCommand::FullColumnStatistics,
                    label,
                    enabled,
                    tone,
                } => {
                    TableMenuRow::Item {
                        command: TableMenuCommand::FullColumnStatistics,
                        label,
                        tone,
                        enabled: enabled
                            && self.session(menu.session_id).is_some_and(|session| {
                                session.engine.as_ref().is_some_and(|engine| {
                                    QuerySession::supports_isolated_reads(engine)
                                }) && session.active_data_tab().is_some_and(|data| {
                                    data.table.name == menu.table.name
                                        && data.table.schema == menu.table.schema
                                        && !data.table_columns.is_empty()
                                })
                            }),
                    }
                }
                row => row,
            })
            .collect()
    }

    fn move_table_menu(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let Some(menu) = self.table_context_menu.clone() else {
            return;
        };
        let enabled = self
            .table_menu_rows_for(&menu)
            .iter()
            .map(TableMenuRow::is_enabled_item)
            .collect::<Vec<_>>();
        if let Some(menu) = &mut self.table_context_menu {
            menu.cursor = move_cursor(&enabled, menu.cursor, motion);
        }
        cx.notify();
    }

    pub(super) fn table_menu_next_action(
        &mut self,
        _: &TableMenuNext,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_table_menu(Motion::Next, cx);
    }

    pub(super) fn table_menu_previous_action(
        &mut self,
        _: &TableMenuPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_table_menu(Motion::Previous, cx);
    }

    pub(super) fn table_menu_first_action(
        &mut self,
        _: &TableMenuFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_table_menu(Motion::First, cx);
    }

    pub(super) fn table_menu_last_action(
        &mut self,
        _: &TableMenuLast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_table_menu(Motion::Last, cx);
    }

    pub(super) fn table_menu_confirm_action(
        &mut self,
        _: &TableMenuConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.table_context_menu.clone() else {
            return;
        };
        let rows = self.table_menu_rows_for(&menu);
        if let Some(TableMenuRow::Item { command, .. }) =
            menu.cursor.and_then(|cursor| rows.get(cursor).copied())
        {
            self.run_table_menu_command(command, window, cx);
        }
    }

    /// Runs a menu command for mouse and keyboard alike. The menu closes and
    /// focus returns to the explorer first, so any dialog the command opens
    /// remembers a live element to hand focus back to.
    pub(super) fn run_table_menu_command(
        &mut self,
        command: TableMenuCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.table_context_menu.clone() else {
            return;
        };
        let allowed = self.table_menu_rows_for(&menu).iter().any(|row| {
            matches!(row, TableMenuRow::Item { command: item, enabled: true, .. } if *item == command)
        });
        if !allowed {
            return;
        }
        self.table_context_menu = None;
        let session_id = menu.session_id;
        let table = menu.table;
        self.focus_session_explorer(session_id, window, cx);
        match command {
            TableMenuCommand::ColumnStatistics => {
                self.show_loaded_table_column_statistics(session_id, window, cx)
            }
            TableMenuCommand::FullColumnStatistics => {
                self.show_full_table_column_statistics(session_id, window, cx)
            }
            TableMenuCommand::CopyName => cx.write_to_clipboard(ClipboardItem::new_string(
                table_ref_label(&table_ref(&table)),
            )),
            TableMenuCommand::ViewDdl
            | TableMenuCommand::CopyDdl
            | TableMenuCommand::CopyInsert => {
                self.table_metadata_command(session_id, table, command, window, cx)
            }
            TableMenuCommand::Rename | TableMenuCommand::Duplicate => self.open_table_name_prompt(
                session_id,
                table,
                command == TableMenuCommand::Duplicate,
                window,
                cx,
            ),
            TableMenuCommand::OpenStructure => self.open_structure_tab_for(session_id, table, cx),
            TableMenuCommand::OpenData | TableMenuCommand::Refresh => {
                self.select_table_for(session_id, table, window, cx)
            }
            TableMenuCommand::Export => self.begin_table_export(session_id, table, cx),
            TableMenuCommand::Import => self.begin_table_import(session_id, table, window, cx),
            TableMenuCommand::Capture => {
                self.copy_or_compare_table(session_id, table, 0, window, cx)
            }
            TableMenuCommand::Append => {
                self.copy_or_compare_table(session_id, table, 1, window, cx)
            }
            TableMenuCommand::Compare => {
                self.copy_or_compare_table(session_id, table, 2, window, cx)
            }
            TableMenuCommand::Truncate => {
                self.confirm_table_action(TableAction::Truncate, session_id, table, window, cx)
            }
            TableMenuCommand::Drop => {
                self.confirm_table_action(TableAction::Drop, session_id, table, window, cx)
            }
        }
        cx.notify();
    }

    /// Move to the next or previous tab of the active connection, wrapping.
    fn cycle_secondary_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_visible() {
            return;
        }
        let Some(session) = self.active_session() else {
            return;
        };
        let count = session.secondary_tabs.len();
        if count < 2 {
            return;
        }
        let current = session
            .active_secondary_tab
            .and_then(|id| session.secondary_tabs.iter().position(|tab| tab.id == id))
            .unwrap_or(0);
        let next = if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        };
        let (session_id, tab_id) = (session.id, session.secondary_tabs[next].id);
        self.activate_secondary_tab_for(session_id, tab_id, window, cx);
    }

    pub(super) fn next_tab_action(
        &mut self,
        _: &NextTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_secondary_tab(true, window, cx);
    }

    pub(super) fn previous_tab_action(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cycle_secondary_tab(false, window, cx);
    }
}
