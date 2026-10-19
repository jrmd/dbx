//! Commands shared by the palette and keyboard shortcuts settings.
use super::*;
use crate::keybindings::{self, KeymapState};
use gpui_component::ActiveTheme;

pub(super) struct ShortcutEditor {
    editor: Entity<TextEditor>,
    error: Option<String>,
    search: Entity<TextEditor>,
    advanced: bool,
    _search_subscription: Subscription,
}

pub(super) fn palette_commands() -> &'static [(&'static str, &'static str)] {
    &[
        ("dbx_ui::NewConnection", "New connection"),
        ("dbx_ui::NewQuery", "New query"),
        ("dbx_ui::CloseTab", "Close tab"),
        ("dbx_ui::OpenQuickOpen", "Go to table, query or connection"),
        ("dbx_ui::OpenSettings", "Open settings"),
        (
            "dbx_ui::OpenKeyboardShortcuts",
            "Keyboard shortcuts and keymap",
        ),
        ("dbx_ui::SearchQueryHistory", "Search query history"),
        ("dbx_ui::RefreshData", "Refresh data"),
        ("dbx_ui::ToggleSidebar", "Toggle sidebar"),
        ("dbx_ui::FocusExplorer", "Focus explorer"),
        ("dbx_ui::NextConnection", "Next connection"),
        ("dbx_ui::PreviousConnection", "Previous connection"),
        ("dbx_ui::NextTab", "Next tab"),
        ("dbx_ui::PreviousTab", "Previous tab"),
        ("dbx_ui::RunQuery", "Run query"),
        ("dbx_ui::RunQueryAll", "Run all statements"),
        ("dbx_ui::FormatQuery", "Format query"),
        ("dbx_ui::ChooseTheme", "Choose theme"),
        ("dbx_ui::LightAppearance", "Use light appearance"),
        ("dbx_ui::DarkAppearance", "Use dark appearance"),
        ("dbx_ui::SystemAppearance", "Follow system appearance"),
        ("dbx_ui::BrowseSqlFiles", "Browse SQL files"),
        ("dbx_ui::FavoriteSqlFiles", "Browse favorite SQL files"),
        ("dbx_ui::OpenSqlFile", "Open SQL file"),
        ("dbx_ui::SaveSqlFile", "Save current query as SQL file"),
        ("dbx_ui::ChooseSqlFolder", "Choose SQL library folder"),
        ("dbx_ui::ExplainQuery", "Explain query plan"),
        ("dbx_ui::AnalyzeQuery", "Explain and analyze query"),
        (
            "dbx_ui::QueryStatistics",
            "Query column statistics · loaded rows",
        ),
        (
            "dbx_ui::TableStatistics",
            "Table column statistics · loaded rows",
        ),
        (
            "dbx_ui::FullTableStatistics",
            "Scan full table for column statistics",
        ),
        ("dbx_ui::WatchQuery", "Watch query every 5 seconds"),
        ("dbx_ui::StopWatch", "Stop query watch"),
        (
            "dbx_ui::InspectSchema",
            "Compare schema with captured baseline",
        ),
        ("dbx_ui::CaptureSchema", "Capture schema baseline"),
        ("dbx_ui::ExportDatabase", "Export database"),
        ("dbx_ui::ExportQueryCsv", "Export loaded query rows as CSV"),
        (
            "dbx_ui::ExportQueryXlsx",
            "Export loaded query rows as Excel",
        ),
        ("dbx_ui::ExportFullQueryXlsx", "Export full query as Excel"),
        ("dbx_ui::ExportFullTableXlsx", "Export full table as Excel"),
        ("dbx_ui::CheckForUpdates", "Check for updates"),
    ]
}

#[derive(Clone)]
struct ShortcutRow {
    action: String,
    label: String,
    group: &'static str,
    keys: Vec<String>,
    contexts: Vec<String>,
}

fn shortcut_group(context: &str) -> &'static str {
    let context = context.to_ascii_lowercase();
    if context.contains("datagrid") || context.contains("datatab") || context.contains("celleditor")
    {
        "Data grid"
    } else if context.contains("diagram") {
        "Schema diagram"
    } else if context.contains("explorer") || context.contains("tablemenu") {
        "Explorer"
    } else if context.contains("sql") || context.contains("query") || context.contains("find") {
        "Queries"
    } else if context == "global" {
        "General"
    } else if context.contains("texteditor") {
        "Text editing"
    } else {
        "Dialogs"
    }
}
fn preferred_shortcut(keys: &str, alternatives: &[String], mac: bool) -> bool {
    if !mac {
        return !keys.split_whitespace().any(|chord| chord.contains("cmd-"));
    }
    if keys.contains("ctrl-") && !keys.contains("cmd-") {
        let normalize = |keys: &str| {
            keys.split_whitespace()
                .map(|chord| {
                    gpui::Keystroke::parse(chord)
                        .map(|key| key.unparse())
                        .unwrap_or_else(|_| chord.to_owned())
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let command_alias = normalize(&keys.replace("ctrl-", "cmd-"));
        return !alternatives
            .iter()
            .any(|alternative| normalize(alternative) == command_alias);
    }
    true
}
fn shortcut_label(keys: &str, mac: bool) -> String {
    keys.split_whitespace()
        .map(|chord| {
            let mut key = chord;
            let mut modifiers = Vec::new();
            loop {
                let found = [
                    ("cmd-", if mac { "⌘" } else { "Super" }),
                    ("ctrl-", if mac { "⌃" } else { "Ctrl" }),
                    ("alt-", if mac { "⌥" } else { "Alt" }),
                    ("shift-", if mac { "⇧" } else { "Shift" }),
                    ("fn-", "Fn"),
                ]
                .into_iter()
                .find(|(prefix, _)| key.starts_with(prefix));
                let Some((prefix, label)) = found else {
                    break;
                };
                modifiers.push(label.to_owned());
                key = &key[prefix.len()..];
            }
            let key = match key {
                "enter" => "Enter".into(),
                "escape" => "Esc".into(),
                "space" => "Space".into(),
                "backspace" => "Backspace".into(),
                "delete" => "Delete".into(),
                "left" => "←".into(),
                "right" => "→".into(),
                "up" => "↑".into(),
                "down" => "↓".into(),
                _ => {
                    let mut chars = key.chars();
                    chars
                        .next()
                        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                        .unwrap_or_default()
                }
            };
            if mac {
                modifiers.sort_by_key(|modifier| match modifier.as_str() {
                    "⌃" => 0,
                    "⌥" => 1,
                    "⇧" => 2,
                    "⌘" => 3,
                    _ => 4,
                });
            }
            modifiers.push(key);
            modifiers.join(if mac { "" } else { "+" })
        })
        .collect::<Vec<_>>()
        .join(" then ")
}
fn shortcut_rows(cx: &App, mac: bool) -> Vec<ShortcutRow> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut grouped = std::collections::BTreeMap::<(String, &'static str), ShortcutRow>::new();
    for binding in keymap.bindings() {
        let action = binding.action().name();
        if action.contains("NoAction")
            || !keymap
                .bindings_for_action(binding.action())
                .any(|effective| {
                    effective.keystrokes() == binding.keystrokes()
                        && effective.predicate().map(|context| context.to_string())
                            == binding.predicate().map(|context| context.to_string())
                })
        {
            continue;
        }
        let keys = binding
            .keystrokes()
            .iter()
            .map(|key| key.unparse())
            .collect::<Vec<_>>()
            .join(" ");
        let context = binding
            .predicate()
            .map(|context| context.to_string())
            .unwrap_or_else(|| "Global".into());
        let group = shortcut_group(&context);
        let label = palette_commands()
            .iter()
            .find(|(name, _)| *name == action)
            .map(|(_, label)| (*label).to_owned())
            .unwrap_or_else(|| keybindings::label(action));
        let row = grouped
            .entry((action.to_owned(), group))
            .or_insert_with(|| ShortcutRow {
                action: action.into(),
                label,
                group,
                keys: Vec::new(),
                contexts: Vec::new(),
            });
        if !row.keys.contains(&keys) {
            row.keys.push(keys);
        }
        if !row.contexts.contains(&context) {
            row.contexts.push(context);
        }
    }
    for (action, label) in palette_commands() {
        if !grouped.keys().any(|(name, _)| name == action) {
            let group = if action.contains("Appearance")
                || action.contains("Theme")
                || action.contains("Settings")
                || action.contains("Connection")
            {
                "General"
            } else {
                "Queries"
            };
            grouped.insert(
                ((*action).to_owned(), group),
                ShortcutRow {
                    action: (*action).into(),
                    label: (*label).into(),
                    group,
                    keys: Vec::new(),
                    contexts: vec!["Global".into()],
                },
            );
        }
    }
    let mut rows = grouped
        .into_values()
        .filter_map(|mut row| {
            let alternatives = row.keys.clone();
            row.keys
                .retain(|keys| preferred_shortcut(keys, &alternatives, mac));
            (!row.keys.is_empty()
                || palette_commands()
                    .iter()
                    .any(|(action, _)| *action == row.action))
            .then_some(row)
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.group
            .cmp(right.group)
            .then(left.label.cmp(&right.label))
    });
    rows
}
pub(super) fn shortcut_for(name: &str, cx: &App) -> String {
    let Ok(action) = cx.build_action(name, None) else {
        return String::new();
    };
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let keys = keymap
        .bindings_for_action(action.as_ref())
        .map(|binding| {
            binding
                .keystrokes()
                .iter()
                .map(|key| key.unparse())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>();
    keys.iter()
        .rev()
        .find(|keys_for_action| {
            preferred_shortcut(keys_for_action, &keys, cfg!(target_os = "macos"))
        })
        .map(|keys| shortcut_label(keys, cfg!(target_os = "macos")))
        .unwrap_or_default()
}
pub(super) fn command_icon(name: &str) -> Icon {
    if name.contains("Theme") || name.contains("Appearance") {
        Icon::Appearance
    } else if name.contains("Export") {
        Icon::Download
    } else if name.contains("Schema") {
        Icon::Structure
    } else if name.contains("Explain") || name.contains("Analyze") || name.contains("Statistics") {
        Icon::Search
    } else if name.contains("Watch") || name.contains("Refresh") {
        Icon::Refresh
    } else if name.contains("Connection") {
        Icon::Database
    } else if name.contains("Settings") || name.contains("Shortcuts") {
        Icon::Settings
    } else if name.contains("Sidebar") || name.contains("Explorer") {
        Icon::Sidebar
    } else if name.contains("New") {
        Icon::Add
    } else if name.contains("Close") {
        Icon::Close
    } else {
        Icon::Query
    }
}
impl DbxApp {
    pub(super) fn open_command_palette_action(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_quick_open_action(&OpenQuickOpen, window, cx);
        if let Some(open) = &self.quick_open {
            open.query.update(cx, |editor, cx| editor.set_text(">", cx));
        }
    }
    pub(super) fn open_sql_file_action(
        &mut self,
        _: &OpenSqlFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.palette_command_available("dbx_ui::OpenSqlFile") {
            self.show_toast(
                ToastKind::Info,
                "Open a SQL connection before opening a query file",
                cx,
            );
            return;
        }
        self.run_palette_command("dbx_ui::OpenSqlFile", window, cx);
    }
    pub(super) fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
    }
    pub(super) fn open_shortcuts_action(
        &mut self,
        _: &OpenKeyboardShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings_section(SettingsSection::Shortcuts, window, cx);
    }
    pub(super) fn search_history_action(
        &mut self,
        _: &SearchQueryHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_history(window, cx);
    }

    pub(super) fn palette_command_available(&self, name: &str) -> bool {
        if matches!(
            name,
            "dbx_ui::NewConnection"
                | "dbx_ui::OpenSettings"
                | "dbx_ui::OpenKeyboardShortcuts"
                | "dbx_ui::OpenQuickOpen"
                | "dbx_ui::CheckForUpdates"
                | "dbx_ui::ChooseTheme"
                | "dbx_ui::LightAppearance"
                | "dbx_ui::DarkAppearance"
                | "dbx_ui::SystemAppearance"
                | "dbx_ui::ChooseSqlFolder"
        ) {
            return true;
        }
        let Some(session) = self
            .active_session()
            .filter(|session| session.engine.is_some())
        else {
            return false;
        };
        let query = self.active_query_tab(session.id);
        let data = session
            .active_secondary_tab
            .and_then(|tab_id| session.data_tab(tab_id));
        let sql_query = session.kind.is_sql() && query.is_some_and(|query| !query.busy);
        match name {
            "dbx_ui::RunQuery" | "dbx_ui::RunQueryAll" => query.is_some_and(|query| !query.busy),
            "dbx_ui::FormatQuery" => sql_query,
            "dbx_ui::RefreshData" => data.is_some_and(|data| !data.busy) && !session.busy,
            "dbx_ui::ExplainQuery" => {
                sql_query
                    && matches!(
                        session.kind,
                        DatabaseKind::PostgreSQL
                            | DatabaseKind::MySQL
                            | DatabaseKind::SQLite
                            | DatabaseKind::Turso
                            | DatabaseKind::CloudflareD1
                            | DatabaseKind::DuckDB
                            | DatabaseKind::CockroachDB
                            | DatabaseKind::ClickHouse
                    )
            }
            "dbx_ui::AnalyzeQuery" => {
                sql_query
                    && matches!(
                        session.kind,
                        DatabaseKind::PostgreSQL | DatabaseKind::CockroachDB | DatabaseKind::MySQL
                    )
                    && matches!(session.engine.as_deref(), Some(DatabaseEngine::Sql(_)))
            }
            "dbx_ui::WatchQuery" => {
                sql_query
                    && query.is_some_and(|query| {
                        !query.in_transaction && query.watch_interval.is_none()
                    })
                    && session.engine.as_ref().is_some_and(|engine| {
                        dbx_core::QuerySession::supports_isolated_reads(engine)
                    })
            }
            "dbx_ui::StopWatch" => query.is_some_and(|query| query.watch_interval.is_some()),
            "dbx_ui::QueryStatistics" | "dbx_ui::ExportQueryCsv" | "dbx_ui::ExportQueryXlsx" => {
                query.is_some_and(|query| {
                    query
                        .result
                        .as_ref()
                        .is_some_and(|result| !result.columns.is_empty())
                })
            }
            "dbx_ui::TableStatistics" => data.is_some_and(|data| data.result.is_some()),
            "dbx_ui::FullTableStatistics" => {
                data.is_some_and(|data| !data.table_columns.is_empty())
                    && !session.busy
                    && session.engine.as_ref().is_some_and(|engine| {
                        dbx_core::QuerySession::supports_isolated_reads(engine)
                    })
            }
            "dbx_ui::ExportFullQueryXlsx" => {
                sql_query
                    && !session.busy
                    && query.is_some_and(|query| !query.in_transaction)
                    && session
                        .engine
                        .as_ref()
                        .is_some_and(|engine| engine.capabilities().streaming_query_export)
            }
            "dbx_ui::ExportFullTableXlsx" => {
                data.is_some()
                    && !session.busy
                    && session
                        .engine
                        .as_ref()
                        .is_some_and(|engine| engine.capabilities().streaming_query_export)
            }
            "dbx_ui::InspectSchema" | "dbx_ui::CaptureSchema" => {
                session.kind.is_sql() && !session.busy && !session.secondary_tabs.iter().any(|tab| matches!(&tab.kind, SecondaryTabKind::Query(query) if query.in_transaction)) && (name != "dbx_ui::InspectSchema" || session.schema_baseline.as_ref().is_some_and(|baseline| Some(&baseline.database) == session.current_database.as_ref()))
            }
            "dbx_ui::ExportDatabase" => session.kind.is_sql() && !session.busy,
            "dbx_ui::BrowseSqlFiles" | "dbx_ui::FavoriteSqlFiles" | "dbx_ui::OpenSqlFile" => {
                session.kind.is_sql()
            }
            "dbx_ui::SaveSqlFile" => session.kind.is_sql() && query.is_some(),
            "dbx_ui::NextConnection" | "dbx_ui::PreviousConnection" => self.sessions.len() > 1,
            "dbx_ui::NextTab" | "dbx_ui::PreviousTab" => session.secondary_tabs.len() > 1,
            _ => true,
        }
    }
    pub(super) fn run_palette_command(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.palette_command_available(name) {
            return;
        }
        match name {
            "dbx_ui::ChooseTheme" => {
                self.open_settings_section(SettingsSection::Appearance, window, cx);
                return;
            }
            "dbx_ui::LightAppearance" | "dbx_ui::DarkAppearance" | "dbx_ui::SystemAppearance" => {
                self.set_appearance_preference(
                    match name {
                        "dbx_ui::LightAppearance" => Appearance::Light,
                        "dbx_ui::DarkAppearance" => Appearance::Dark,
                        _ => Appearance::System,
                    },
                    window,
                    cx,
                );
                return;
            }
            "dbx_ui::ChooseSqlFolder" => {
                self.choose_sql_library_folder(cx);
                return;
            }
            _ => {}
        }
        if let Some(session_id) = self.active_session_id() {
            if !matches!(
                name,
                "dbx_ui::OpenSettings"
                    | "dbx_ui::OpenKeyboardShortcuts"
                    | "dbx_ui::CheckForUpdates"
                    | "dbx_ui::NewConnection"
            ) {
                if self.settings_open {
                    self.close_settings(window, cx);
                }
                self.connection_picker_open = false;
            }
            match name {
                "dbx_ui::RunQuery" | "dbx_ui::RunQueryAll" => {
                    self.request_run_query_for(session_id, name.ends_with("All"), window, cx);
                    return;
                }
                "dbx_ui::FormatQuery" => {
                    self.format_query_for(session_id, window, cx);
                    return;
                }
                "dbx_ui::BrowseSqlFiles" | "dbx_ui::FavoriteSqlFiles" => {
                    self.browse_sql_files(name == "dbx_ui::FavoriteSqlFiles", window, cx);
                    return;
                }
                "dbx_ui::OpenSqlFile" => {
                    self.open_sql_file_for(session_id, window, cx);
                    return;
                }
                "dbx_ui::SaveSqlFile" => {
                    self.save_query_sql_file_for(session_id, cx);
                    return;
                }
                "dbx_ui::ExplainQuery" => {
                    self.explain_query_for(session_id, cx);
                    return;
                }
                "dbx_ui::AnalyzeQuery" => {
                    self.request_analyze_query_for(session_id, window, cx);
                    return;
                }
                "dbx_ui::QueryStatistics" => {
                    self.show_loaded_column_statistics(session_id, window, cx);
                    return;
                }
                "dbx_ui::TableStatistics" => {
                    self.show_loaded_table_column_statistics(session_id, window, cx);
                    return;
                }
                "dbx_ui::FullTableStatistics" => {
                    self.show_full_table_column_statistics(session_id, window, cx);
                    return;
                }
                "dbx_ui::WatchQuery" => {
                    self.start_query_watch(session_id, 5, cx);
                    return;
                }
                "dbx_ui::StopWatch" => {
                    self.stop_query_watch(session_id, cx);
                    return;
                }
                "dbx_ui::InspectSchema" | "dbx_ui::CaptureSchema" => {
                    self.inspect_schema_for(
                        session_id,
                        name == "dbx_ui::CaptureSchema",
                        window,
                        cx,
                    );
                    return;
                }
                "dbx_ui::ExportDatabase" => {
                    self.begin_database_export(session_id, window, cx);
                    return;
                }
                "dbx_ui::ExportQueryCsv" | "dbx_ui::ExportQueryXlsx" => {
                    self.request_query_result_export_for(
                        session_id,
                        if name.ends_with("Csv") {
                            QueryResultExportFormat::Csv
                        } else {
                            QueryResultExportFormat::Xlsx
                        },
                        QueryResultExportAction::Export,
                        window,
                        cx,
                    );
                    return;
                }
                "dbx_ui::ExportFullQueryXlsx" => {
                    self.export_full_query_xlsx_for(session_id, cx);
                    return;
                }
                "dbx_ui::ExportFullTableXlsx" => {
                    if let Some(tab_id) = self
                        .session(session_id)
                        .and_then(|session| session.active_secondary_tab)
                    {
                        self.export_full_table_xlsx_for(session_id, tab_id, cx);
                    }
                    return;
                }
                _ => {}
            }
        }
        if let Ok(action) = cx.build_action(name, None) {
            window.dispatch_action(action, cx);
        }
    }

    pub(super) fn render_settings_shortcuts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.shortcut_editor.is_none() {
            let (document, error) = if cx.has_global::<KeymapState>() {
                let state = cx.global::<KeymapState>();
                (state.document.clone(), state.load_error.clone())
            } else {
                (keybindings::initial_document(), None)
            };
            let editor = cx.new(|cx| {
                let text = cx.new(|_| document);
                TextEditor::new_json(text, window, cx).accessibility_label("Editable JSON keymap")
            });
            let search = cx.new(|cx| {
                TextEditor::empty(false, window, cx)
                    .accessibility_label("Search keyboard shortcuts")
            });
            let subscription = cx.observe(&search, |_, _, cx| cx.notify());
            self.shortcut_editor = Some(ShortcutEditor {
                editor,
                error,
                search,
                advanced: false,
                _search_subscription: subscription,
            });
        }
        let state = self.shortcut_editor.as_ref().unwrap();
        let input = state.editor.clone();
        let search = state.search.clone();
        let error = state.error.clone();
        let advanced = state.advanced;
        let needle = search.read(cx).text(cx).to_lowercase();
        let rows = shortcut_rows(cx, cfg!(target_os = "macos"))
            .into_iter()
            .filter(|row| {
                needle.split_whitespace().all(|word| {
                    format!(
                        "{} {} {} {} {}",
                        row.label,
                        row.group,
                        row.action,
                        row.keys.join(" "),
                        if row.keys.is_empty() {
                            "Unassigned".into()
                        } else {
                            row.keys
                                .iter()
                                .map(|keys| shortcut_label(keys, cfg!(target_os = "macos")))
                                .collect::<Vec<_>>()
                                .join(" ")
                        }
                    )
                    .to_lowercase()
                    .contains(word)
                })
            })
            .collect::<Vec<_>>();
        let mut list = Vec::new();
        for group in [
            "General",
            "Queries",
            "Data grid",
            "Explorer",
            "Schema diagram",
            "Text editing",
            "Dialogs",
        ] {
            let group_rows = rows
                .iter()
                .filter(|row| row.group == group)
                .collect::<Vec<_>>();
            if group_rows.is_empty() {
                continue;
            }
            list.push(
                div()
                    .pt(crate::theme::ui_px(12.))
                    .text_size(crate::theme::text_px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(group)
                    .into_any_element(),
            );
            list.extend(group_rows.into_iter().map(|row| {
                let keys = if row.keys.is_empty() {
                    "Unassigned".to_owned()
                } else {
                    row.keys
                        .iter()
                        .map(|keys| shortcut_label(keys, cfg!(target_os = "macos")))
                        .collect::<Vec<_>>()
                        .join(" / ")
                };
                div()
                    .id(SharedString::from(format!(
                        "shortcut-{}-{}",
                        row.group, row.action
                    )))
                    .role(gpui::Role::Label)
                    .aria_label(format!("{}: {}. {}", row.label, keys, row.group))
                    .py(crate::theme::ui_px(9.))
                    .border_b_1()
                    .border_color(theme().hairline)
                    .flex()
                    .items_center()
                    .gap(crate::theme::ui_px(16.))
                    .tooltip(tip(format!(
                        "Action: {}\nContext: {}",
                        row.action,
                        row.contexts.join(" or ")
                    )))
                    .child(div().flex_1().child(row.label.clone()))
                    .child(
                        div()
                            .text_color(theme().text_muted)
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(keys),
                    )
                    .into_any_element()
            }));
        }
        div().flex().flex_col().gap(crate::theme::ui_px(12.))
            .child(div().text_color(theme().text_muted).child("Search by action, key or area. Hover an action for its identifier and context."))
            .child(editor::input(search.clone(), search.read(cx).focus_handle(), false, cx))
            .when(list.is_empty(), |view| view.child("No matching keyboard shortcuts"))
            .children(list)
            .child(button("toggle-advanced-keymap", if advanced { "Hide JSON keymap" } else { "Customize shortcuts with JSON…" }, ButtonKind::Quiet)
                .on_click(cx.listener(|this, _, _, cx| { if let Some(editor) = &mut this.shortcut_editor { editor.advanced = !editor.advanced; } cx.notify(); })))
            .when(advanced, |view| view.child(div().flex().flex_col().gap(crate::theme::ui_px(12.))
                .child(div().text_color(theme().text_muted).child(format!("Zed-style JSON. Changes apply after Save. Use null to disable a binding; separate chord keys with spaces. {}", keybindings::path().display())))
                .child(editor::input_with_height(input.clone(), input.read(cx).focus_handle(), true, crate::theme::ui_px(240.), cx))
                .when_some(error, |view, error| view.child(div().text_color(theme().danger).child(error)))
                .child(div().flex().gap(crate::theme::ui_px(8.))
                    .child(button("save-keymap", "Save keymap", ButtonKind::Primary).on_click(cx.listener(|this, _, _, cx| {
                        let Some(editor) = &this.shortcut_editor else { return; }; let document = editor.editor.read(cx).text(cx);
                        match keybindings::save(document, cx) { Ok(()) => { if let Some(editor) = &mut this.shortcut_editor { editor.error = None; } this.show_toast(ToastKind::Success, "Keymap saved", cx); }, Err(error) => { if let Some(editor) = &mut this.shortcut_editor { editor.error = Some(error); } } } cx.notify();
                    })))
                    .child(button("restore-keymap-editor", "Restore saved content", ButtonKind::Quiet).on_click(cx.listener(|this, _, _, cx| {
                        let document = if cx.has_global::<KeymapState>() { cx.global::<KeymapState>().document.clone() } else { keybindings::initial_document() };
                        if let Some(editor) = &mut this.shortcut_editor { editor.editor.update(cx, |editor, cx| editor.set_text(document, cx)); editor.error = None; } cx.notify();
                    }))))))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Action as _;

    #[gpui::test]
    fn registered_shortcuts_use_portable_keys_before_platform_formatting(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-n", NewConnection, None),
                gpui::KeyBinding::new("ctrl-n", NewConnection, None),
                gpui::KeyBinding::new("ctrl-tab", NewQuery, None),
                gpui::KeyBinding::new("cmd-alt-,", OpenKeyboardShortcuts, None),
                gpui::KeyBinding::new("ctrl-alt-,", OpenKeyboardShortcuts, None),
            ]);
            let rows = shortcut_rows(cx, true);
            let connection = rows
                .iter()
                .find(|row| row.action == NewConnection.name())
                .unwrap();
            assert_eq!(connection.keys, ["cmd-n"]);
            assert_eq!(shortcut_label(&connection.keys[0], true), "⌘N");
            let query = rows
                .iter()
                .find(|row| row.action == NewQuery.name())
                .unwrap();
            assert_eq!(query.keys, ["ctrl-tab"]);
            assert_eq!(shortcut_label(&query.keys[0], true), "⌃Tab");
            let shortcuts = rows
                .iter()
                .find(|row| row.action == OpenKeyboardShortcuts.name())
                .unwrap();
            assert_eq!(shortcuts.keys, ["alt-cmd-,"]);
            assert_eq!(shortcut_label(&shortcuts.keys[0], true), "⌥⌘,");
            assert_eq!(
                shortcut_for(OpenKeyboardShortcuts.name(), cx),
                if cfg!(target_os = "macos") {
                    "⌥⌘,"
                } else {
                    "Ctrl+Alt+,"
                }
            );
            assert_eq!(
                shortcut_for(NewConnection.name(), cx),
                if cfg!(target_os = "macos") {
                    "⌘N"
                } else {
                    "Ctrl+N"
                }
            );
        });
    }

    #[test]
    fn platform_shortcuts_keep_real_control_bindings_and_hide_alias_duplicates() {
        let keys = vec!["cmd-p".into(), "ctrl-p".into(), "ctrl-tab".into()];
        assert!(!preferred_shortcut("ctrl-p", &keys, true));
        assert!(preferred_shortcut("ctrl-tab", &keys, true));
        assert!(!preferred_shortcut("cmd-p", &keys, false));
        assert_eq!(shortcut_label("cmd-shift-p", true), "⇧⌘P");
        assert_eq!(shortcut_label("ctrl-k ctrl-s", false), "Ctrl+K then Ctrl+S");
        assert_eq!(shortcut_group("DbxDataGrid"), "Data grid");
        assert_eq!(shortcut_group("QueryWorkbench"), "Queries");
    }
}
