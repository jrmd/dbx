//! AppKit navigation is a projection of existing session state. Every native
//! intent is checked against the current session before using shared handlers.
use super::*;
use crate::workspace::SavedQuery;
use explorer::{TableMenuRow, table_menu_rows};
use gpui::AssetSource;
use serde_json::{Value, json};

impl DbxApp {
    pub(super) fn native_navigation_active(&self) -> bool {
        self.native_glass.active()
    }

    pub(crate) fn refresh_native_navigation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for command in self.native_glass.drain() {
            let kind = command["kind"].as_str().unwrap_or_default();
            if kind == "nativeContextMenu" {
                if let Some(id) = command["menuID"].as_u64() {
                    let path = if self.modal_traps().is_empty() {
                        command["path"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|index| {
                                index.as_u64().and_then(|index| usize::try_from(index).ok())
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    };
                    // Menu callbacks often update DbxApp itself. Run after this
                    // navigation update releases its entity borrow.
                    window.defer(cx, move |window, cx| {
                        gpui_component::menu::complete_native_context_menu(id, &path, window, cx);
                    });
                }
                continue;
            }
            if kind == "geometry" {
                self.native_glass.left_inset = command["left"].as_f64().unwrap_or(0.) as f32;
                self.native_glass.top_inset = command["top"].as_f64().unwrap_or(52.) as f32;
                if self.workspace_screen_drawn()
                    && !self.inspector_page()
                    && self.modal_traps().is_empty()
                    && self.native_glass.left_inset > 0.
                {
                    self.explorer_width = Some(self.native_glass.left_inset);
                }
                cx.notify();
                continue;
            }
            if !self.modal_traps().is_empty() {
                continue;
            }
            let session_id = command["session"]
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok());
            // Reject actions queued for a connection that was closed/switched.
            if session_id != self.active_session_id {
                continue;
            }
            let text = command["value"].as_str().unwrap_or_default();
            match kind {
                "settings" => {
                    if self.settings_open {
                        self.close_settings(window, cx)
                    } else {
                        self.open_settings(window, cx)
                    }
                }
                "lock" => self.request_lock_vault(window, cx),
                "newConnection" => self.new_connection_action(&NewConnection, window, cx),
                "newQuery" => self.new_query_action(&NewQuery, window, cx),
                "refresh" => self.refresh_action(&RefreshData, window, cx),
                "toggleSidebar" => self.toggle_sidebar_action(&ToggleSidebar, window, cx),
                "data" => self.set_active_pane(Pane::Data, window, cx),
                "structure" => self.set_active_pane(Pane::Structure, window, cx),
                "closeConnection" => {
                    if let Some(id) = command["id"]
                        .as_str()
                        .and_then(|id| Uuid::parse_str(id).ok())
                        .or(session_id)
                    {
                        self.close_session(id, cx);
                    }
                }
                "moveConnection" => {
                    let id = |key: &str| {
                        command[key]
                            .as_str()
                            .and_then(|id| Uuid::parse_str(id).ok())
                    };
                    if let (Some(id), Some(target)) = (id("id"), id("target")) {
                        self.move_session(id, target, cx)
                    }
                }
                "connection" => {
                    if let Some(id) = command["id"]
                        .as_str()
                        .and_then(|id| Uuid::parse_str(id).ok())
                    {
                        self.activate_session(id, cx)
                    }
                }
                _ => {
                    let Some(id) = session_id else {
                        continue;
                    };
                    match kind {
                        "closeOtherConnections" => {
                            let target = command["id"]
                                .as_str()
                                .and_then(|value| Uuid::parse_str(value).ok())
                                .unwrap_or(id);
                            if self.session(target).is_some() {
                                self.close_other_sessions(target, cx);
                            }
                        }
                        "search" => {
                            if let Some(session) = self.session(id) {
                                session.editors.sidebar_search.update(cx, |search, cx| {
                                    *search = text.to_owned();
                                    cx.notify();
                                });
                            }
                        }
                        "database" => self.switch_database_for(id, text.to_owned(), cx),
                        "schema" => self.select_schema_filter_for(
                            id,
                            (!text.is_empty()).then(|| text.to_owned()),
                            cx,
                        ),
                        "refreshExplorer" => self.refresh_tables_for(id, cx),
                        "newTable" => self.create_table_template_for(id, window, cx),
                        "diagram" => self.open_diagram_for(id, window, cx),
                        "backup" => self.choose_native_backup(id, false, cx),
                        "restore" => self.choose_native_backup(id, true, cx),
                        "transferLog" => self.show_transfer_log(id, window, cx),
                        "mcp" => self.toggle_mcp(id, cx),
                        "mcpCopy" => self.copy_mcp_pairing(id, cx),
                        "mcpLog" => self.show_mcp_activity(id, window, cx),
                        "export" => self.begin_database_export(id, window, cx),
                        "import" => self.begin_database_import(id, window, cx),
                        "selection" | "tableCommand" => {
                            self.native_table_action(id, &command, window, cx)
                        }
                        _ => {}
                    }
                }
            }
            cx.notify();
        }
        if self.native_glass.take_dirty() {
            self.sync_native_navigation(cx);
        }
        let (offset, maximum) = self.native_scroll_geometry(cx);
        self.native_glass.scroll(offset, maximum);
    }

    fn native_table_action(
        &mut self,
        id: SessionId,
        command: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: BTreeSet<String> = command["ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let Some(session) = self.session_mut(id) else {
            return;
        };
        let tables: Vec<_> = session
            .sidebar
            .list
            .visible
            .iter()
            .filter(|table| ids.contains(&table_sidebar_id(table)))
            .cloned()
            .collect();
        session.sidebar.selected = tables
            .iter()
            .map(table_sidebar_id)
            .chain(
                session
                    .schema_objects
                    .iter()
                    .map(native_object_id)
                    .filter(|id| ids.contains(id)),
            )
            .collect();
        session.sidebar.cursor = tables.first().map(table_sidebar_id);
        if command["kind"] == "tableCommand" {
            let Some(table) = tables.first().cloned() else {
                return;
            };
            let name = command["command"].as_str().unwrap_or_default();
            let menu_command = table_menu_rows(true).into_iter().find_map(|row| match row {
                TableMenuRow::Item { command, .. } if format!("{command:?}") == name => {
                    Some(command)
                }
                _ => None,
            });
            if let Some(menu_command) = menu_command {
                self.open_table_context_menu(id, table, point(px(0.), px(0.)), window, cx);
                // Existing handler revalidates permissions, multi-selection and
                // confirmations. Native menus never bypass mutation safeguards.
                self.run_table_menu_command(menu_command, window, cx);
                self.table_context_menu = None;
            }
        } else if command["open"] == true {
            if let Some(table) = tables.first() {
                self.select_table_for(id, table.clone(), window, cx);
            } else if let Some(object) = self.session(id).and_then(|session| {
                session
                    .schema_objects
                    .iter()
                    .find(|object| ids.contains(&native_object_id(object)))
                    .cloned()
            }) {
                if let Some(definition) = object.definition {
                    self.open_saved_query_for(
                        id,
                        SavedQuery {
                            name: object.name,
                            sql: definition,
                        },
                        window,
                        cx,
                    );
                } else {
                    self.show_toast(
                        ToastKind::Info,
                        "This account cannot read the object definition",
                        cx,
                    );
                }
            }
        }
    }

    fn native_scroll_geometry(&self, cx: &App) -> (f32, f32) {
        if self.settings_open {
            (
                (-self.settings_scroll.offset().y).as_f32(),
                self.settings_scroll.max_offset().y.as_f32(),
            )
        } else if let Some(session) = self.active_session() {
            if session.pane == Pane::Query {
                self.active_query_editor_for(session.id)
                    .map(|editor| editor.read(cx).scroll_geometry())
                    .unwrap_or_default()
            } else if let Some(data) = session.active_data_tab() {
                let grid = data.data_grid.read(cx);
                let scroll = grid.vertical_scroll_handle.0.borrow();
                (
                    (-scroll.base_handle.offset().y).as_f32(),
                    scroll.base_handle.max_offset().y.as_f32(),
                )
            } else {
                (0., 0.)
            }
        } else {
            (0., 0.)
        }
    }

    fn sync_native_navigation(&mut self, cx: &mut Context<Self>) {
        if !self.native_glass.active() {
            return;
        }
        if let Some(id) = self.active_session_id {
            let search = self
                .session(id)
                .map(|session| session.editors.sidebar_search.read(cx).clone())
                .unwrap_or_default();
            if let Some(session) = self.session_mut(id) {
                session.sidebar.list.refresh(
                    session.kind,
                    &session.tables,
                    session.tables_revision,
                    session.schema_filter.as_deref(),
                    &search,
                );
                let visible: BTreeSet<_> = session
                    .sidebar
                    .list
                    .visible
                    .iter()
                    .map(table_sidebar_id)
                    .chain(
                        session
                            .schema_objects
                            .iter()
                            .filter(|object| {
                                object.name.to_lowercase().contains(&search.to_lowercase())
                            })
                            .map(native_object_id),
                    )
                    .collect();
                session.sidebar.selected.retain(|id| visible.contains(id));
            }
        }
        let unlocked = self.vault_state == Some(VaultState::Unlocked);
        let session = self.active_session();
        let connected = unlocked && session.is_some_and(|session| session.engine.is_some());
        let visible = self.workspace_screen_drawn()
            && connected
            && !self.sidebar_hidden
            && !self.inspector_page();
        let mut rows = Vec::new();
        let mut selected = Vec::new();
        let mut schemas = Vec::new();
        let mut databases = Vec::new();
        let mut search = String::new();
        let mut status = "Explorer".to_owned();
        let mut menu = Vec::new();
        let mut explorer_menu = Vec::new();
        if let Some(session) = session {
            search = session.editors.sidebar_search.read(cx).clone();
            for table in session.sidebar.list.visible.iter() {
                rows.push(json!({"id": table_sidebar_id(table), "label": table_sidebar_label(table, session.schema_filter.as_deref()), "symbol": if table.kind == EntityKind::View { "eye" } else { "tablecells" }, "writable": table.kind == EntityKind::Table}));
            }
            for object in &session.schema_objects {
                if object.name.to_lowercase().contains(&search.to_lowercase()) {
                    rows.push(json!({"id": native_object_id(object), "label": format!("{} · {:?}", object.name, object.kind), "symbol": "chevron.left.forwardslash.chevron.right"}));
                }
            }
            selected = session.sidebar.selected.iter().cloned().collect();
            schemas = session
                .sidebar
                .list
                .schema_options
                .iter()
                .map(|schema| {
                    schema
                        .clone()
                        .unwrap_or_else(|| ALL_SCHEMAS_LABEL.to_owned())
                })
                .collect();
            databases = session.databases.clone();
            status = if session.busy {
                "Loading…".into()
            } else if session.error.is_some() {
                "Couldn’t load explorer".into()
            } else if rows.is_empty() && !search.is_empty() {
                "No matches".into()
            } else {
                format!("Explorer · {}", rows.len())
            };
            let writable = session
                .engine
                .as_ref()
                .is_some_and(|engine| !engine.is_read_only());
            let chosen: Vec<_> = session
                .sidebar
                .list
                .visible
                .iter()
                .filter(|table| session.sidebar.selected.contains(&table_sidebar_id(table)))
                .cloned()
                .collect();
            let menu_rows = chosen
                .first()
                .or_else(|| session.sidebar.list.visible.first())
                .map(|table| {
                    self.table_menu_rows_for(&TableContextMenu {
                        session_id: session.id,
                        table: table.clone(),
                        tables: if chosen.is_empty() {
                            vec![table.clone()]
                        } else {
                            chosen.clone()
                        },
                        position: point(px(0.), px(0.)),
                        cursor: None,
                        focus: self.focus_handle.clone(),
                    })
                })
                .unwrap_or_default();
            menu = menu_rows.into_iter().map(|row| match row {
                TableMenuRow::Separator => json!({"separator": true}),
                TableMenuRow::Item { command, label, enabled, .. } => json!({"command": format!("{command:?}"), "label": label, "enabled": enabled}),
            }).collect();
            let native_backup = session
                .engine
                .as_ref()
                .is_some_and(|engine| engine.capabilities().native_backup);
            let mcp = session.mcp_pairing.is_some();
            let sql = session.kind.is_sql();
            for (command, label, enabled) in [
                ("diagram", "Open database diagram", sql),
                ("backup", "Native backup…", native_backup),
                (
                    "restore",
                    "Restore native backup…",
                    native_backup && writable,
                ),
                ("transferLog", "View native job log", native_backup),
                (
                    "mcp",
                    if mcp {
                        "Stop MCP sharing"
                    } else {
                        "Share this database read-only via MCP"
                    },
                    sql,
                ),
                ("mcpCopy", "Copy private MCP pairing configuration", mcp),
                ("mcpLog", "View MCP activity", mcp),
                ("export", "Export database…", sql),
                (
                    "import",
                    "Import database…",
                    sql && writable && !session.busy,
                ),
                ("closeConnection", "Close connection", true),
                (
                    "closeOtherConnections",
                    "Close other connections",
                    self.sessions.len() > 1,
                ),
            ] {
                explorer_menu.push(json!({"command": command, "label": label, "enabled": enabled}));
            }
        }
        let scrim = {
            let color = theme().overlay;
            [color.r, color.g, color.b, color.a]
        };
        self.native_glass.sync(&json!({
            "session": self.active_session_id,
            "connections": if unlocked { self.sessions.iter().map(|session| {
                // `label` stays the full spoken name; AppKit styles the parts
                // and draws the tag as the same outlined pill GPUI tabs use.
                let tag = session.tag.as_ref().map(|tag| json!({"name": tag.name, "color": tag.color}));
                json!({"id": session.id, "name": session.name, "busy": session.busy, "tag": tag,
                    "logo": native_engine_logo(session.kind), "engine": session.kind.to_string(),
                    "label": format!("{}{}{}", session.name, if session.busy { " · working" } else { "" }, session.tag.as_ref().map(|tag| format!(" · {}", tag.name)).unwrap_or_default())})
            }).collect::<Vec<_>>() } else { Vec::new() },
            "connected": connected, "unlocked": unlocked, "sidebar": visible,
            // Settings, the connection list and inspector pages never show the
            // explorer, so the toggle would only flip hidden state there.
            "canToggleSidebar": connected && self.workspace_screen_drawn() && !self.inspector_page(),
            "canCreateTable": session.is_some_and(|session| session.kind.is_sql() && !session.busy && session.engine.as_ref().is_some_and(|engine| !engine.is_read_only())),
            "modal": !self.modal_traps().is_empty(), "scrim": scrim, "dark": crate::theme::is_dark(), "systemAppearance": self.appearance == Appearance::System,
            "rows": rows, "selected": selected, "schemas": schemas, "databases": databases,
            "schema": session.and_then(|session| session.schema_filter.as_deref()).unwrap_or(ALL_SCHEMAS_LABEL),
            "database": session.and_then(|session| session.current_database.as_deref()),
            "search": search, "status": status, "tableMenu": menu, "explorerMenu": explorer_menu,
        }));
    }
}

/// Use the same embedded engine marks as the GPUI connection tabs.
fn native_engine_logo(kind: dbx_core::DatabaseKind) -> &'static str {
    let path = dbx_ui_components::theme::database_logo_path(kind);
    let bytes = dbx_ui_components::assets::Assets
        .load(path)
        .expect("embedded database logo")
        .expect("database logo exists");
    // All embedded SVGs are static UTF-8 assets.
    match bytes {
        std::borrow::Cow::Borrowed(bytes) => {
            std::str::from_utf8(bytes).expect("UTF-8 database logo")
        }
        std::borrow::Cow::Owned(_) => unreachable!("database logos are embedded"),
    }
}

fn native_object_id(object: &dbx_core::SchemaObject) -> String {
    format!(
        "object:{:?}:{}:{}",
        object.kind,
        object.schema.as_deref().unwrap_or_default(),
        object.name
    )
}

#[cfg(test)]
mod native_logo_tests {
    use super::*;

    #[test]
    fn every_database_kind_has_an_embedded_svg_logo() {
        for kind in dbx_core::DatabaseKind::ALL {
            assert!(
                native_engine_logo(kind).contains("<svg"),
                "missing logo for {kind}"
            );
        }
    }

    #[gpui::test]
    fn native_context_menu_preserves_submenu_actions_and_cancellation(
        cx: &mut gpui::TestAppContext,
    ) {
        use gpui_component::menu::{
            PopupMenu, PopupMenuItem, complete_native_context_menu,
            install_native_context_presenter, present_native_context_menu,
        };
        use std::{
            cell::Cell,
            rc::Rc,
            sync::atomic::{AtomicU64, Ordering},
        };
        static ID: AtomicU64 = AtomicU64::new(0);
        fn capture(
            id: u64,
            items: &serde_json::Value,
            _: gpui::Point<gpui::Pixels>,
            _: &mut Window,
            _: &mut App,
        ) -> bool {
            assert_eq!(items[0]["label"], "Copy");
            assert_eq!(items[0]["children"][0]["label"], "JSON");
            assert_eq!(items[0]["children"][1]["enabled"], false);
            ID.store(id, Ordering::SeqCst);
            true
        }
        cx.update(|cx| {
            gpui_component::init(cx);
            install_native_context_presenter(capture, cx);
        });
        let (_, cx) = cx.add_window_view(|window, cx| DbxApp::new(window, cx));
        let calls = Rc::new(Cell::new(0));
        cx.update(|window, cx| {
            let calls_for_menu = calls.clone();
            let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                let child =
                    PopupMenu::build(window, cx, move |child, _, _| {
                        child
                            .item(PopupMenuItem::new("JSON").on_click(move |_, _, _| {
                                calls_for_menu.set(calls_for_menu.get() + 1)
                            }))
                            .item(PopupMenuItem::new("Disabled").disabled(true))
                    });
                menu.item(PopupMenuItem::submenu("Copy", child))
            });
            assert!(present_native_context_menu(
                menu.clone(),
                point(px(10.), px(10.)),
                window,
                cx
            ));
            complete_native_context_menu(ID.load(Ordering::SeqCst), &[0, 1], window, cx);
            assert_eq!(calls.get(), 0);
            assert!(present_native_context_menu(
                menu.clone(),
                point(px(10.), px(10.)),
                window,
                cx
            ));
            complete_native_context_menu(ID.load(Ordering::SeqCst), &[], window, cx);
            assert_eq!(calls.get(), 0);
            assert!(present_native_context_menu(
                menu,
                point(px(10.), px(10.)),
                window,
                cx
            ));
            complete_native_context_menu(ID.load(Ordering::SeqCst), &[0, 0], window, cx);
            assert_eq!(calls.get(), 1);
        });
    }
}
