mod app;
mod assets;
mod connection_fields;
mod device_unlock;
mod diagram;
mod editor;
#[allow(dead_code)]
mod filters;
mod profiles;
#[allow(dead_code)]
mod query_history;
#[allow(dead_code)]
mod row_drafts;
mod settings;
mod theme;
mod vault;

use app::DbxApp;
use gpui::{
    App, AppContext, Bounds, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, TitlebarOptions,
    WindowBounds, WindowDecorations, WindowOptions, point, px, size,
};

gpui::actions!(dbx_ui, [Quit, Minimize, Zoom]);

const APP_NAME: &str = "DBX";

fn dbx_window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(80.0), px(80.0)),
            size(px(1440.0), px(900.0)),
        ))),
        window_min_size: Some(size(px(960.0), px(640.0))),
        // GPUI's macOS backend only includes AppKit's resizable style mask when
        // a titlebar configuration is present. Preserve native traffic lights;
        // the shared titlebar reserves their leading space on macOS.
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            // Vertically centred in DBX's 46px unified titlebar.
            traffic_light_position: Some(point(px(16.0), px(16.0))),
        }),
        app_owns_titlebar_drag: true,
        window_decorations: Some(WindowDecorations::Client),
        window_background: theme::window_background(),
        app_id: Some("dev.jrmd.dbx".into()),
        ..Default::default()
    }
}

/// The native menu bar (macOS). Items dispatch the same actions as the
/// keyboard shortcuts, so the menu shows each binding next to its command.
fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: APP_NAME.into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Quit DBX", Quit),
            ],
            disabled: false,
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Connection", app::NewConnection),
                MenuItem::action("New Query", app::NewQuery),
                MenuItem::separator(),
                MenuItem::action("Close Tab", app::CloseTab),
            ],
            disabled: false,
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", editor::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", editor::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", editor::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", editor::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", editor::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", editor::SelectAll, OsAction::SelectAll),
            ],
            disabled: false,
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Toggle Explorer", app::ToggleSidebar),
                MenuItem::action("Refresh", app::RefreshData),
            ],
            disabled: false,
        },
        Menu {
            name: "Query".into(),
            items: vec![
                MenuItem::action("Run Statement", app::RunQuery),
                MenuItem::action("Run All", app::RunQueryAll),
                MenuItem::action("Cancel", app::CancelQuery),
                MenuItem::separator(),
                MenuItem::action("Format SQL", app::FormatQuery),
            ],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
                MenuItem::separator(),
                MenuItem::action("Next Connection", app::NextConnection),
                MenuItem::action("Previous Connection", app::PreviousConnection),
            ],
            disabled: false,
        },
    ]
}

fn main() {
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            let settings = settings::SettingsStore::new()
                .and_then(|store| store.load())
                .unwrap_or_else(|error| {
                    eprintln!("DBX could not load appearance settings: {error}");
                    settings::Settings::default()
                });
            theme::set_appearance(settings.appearance);
            theme::set_reduce_transparency(settings.reduce_transparency);
            theme::set_system_appearance(cx.window_appearance());
            theme::sync_component_theme(None, cx);
            cx.bind_keys(editor::default_key_bindings());
            cx.bind_keys([
                KeyBinding::new("tab", app::VaultFocusNext, Some("VaultGate")),
                KeyBinding::new("shift-tab", app::VaultFocusPrevious, Some("VaultGate")),
                KeyBinding::new("enter", app::SubmitVault, Some("VaultGate")),
                KeyBinding::new("enter", app::ApplyFilters, Some("DbxFilters")),
                KeyBinding::new("cmd-enter", app::RunQuery, Some(editor::SQL_EDITOR_CONTEXT)),
                KeyBinding::new(
                    "ctrl-enter",
                    app::RunQuery,
                    Some(editor::SQL_EDITOR_CONTEXT),
                ),
                KeyBinding::new(
                    "shift-cmd-enter",
                    app::RunQueryAll,
                    Some(editor::SQL_EDITOR_CONTEXT),
                ),
                KeyBinding::new(
                    "ctrl-shift-enter",
                    app::RunQueryAll,
                    Some(editor::SQL_EDITOR_CONTEXT),
                ),
                KeyBinding::new("escape", app::CancelQuery, Some(editor::SQL_EDITOR_CONTEXT)),
                KeyBinding::new("escape", app::CancelQuery, Some("QueryWorkbench")),
                KeyBinding::new("cmd-c", app::CopyQuerySelection, Some("QueryResult")),
                KeyBinding::new("ctrl-c", app::CopyQuerySelection, Some("QueryResult")),
                KeyBinding::new("left", app::DiagramPanLeft, Some("DbxDiagram")),
                KeyBinding::new("right", app::DiagramPanRight, Some("DbxDiagram")),
                KeyBinding::new("up", app::DiagramPanUp, Some("DbxDiagram")),
                KeyBinding::new("down", app::DiagramPanDown, Some("DbxDiagram")),
                KeyBinding::new("shift-left", app::DiagramPanLeftLarge, Some("DbxDiagram")),
                KeyBinding::new("shift-right", app::DiagramPanRightLarge, Some("DbxDiagram")),
                KeyBinding::new("shift-up", app::DiagramPanUpLarge, Some("DbxDiagram")),
                KeyBinding::new("shift-down", app::DiagramPanDownLarge, Some("DbxDiagram")),
                KeyBinding::new("=", app::DiagramZoomIn, Some("DbxDiagram")),
                KeyBinding::new("shift-=", app::DiagramZoomIn, Some("DbxDiagram")),
                KeyBinding::new("-", app::DiagramZoomOut, Some("DbxDiagram")),
                KeyBinding::new("0", app::DiagramResetView, Some("DbxDiagram")),
                KeyBinding::new("f", app::DiagramFit, Some("DbxDiagram")),
                KeyBinding::new("r", app::DiagramRefresh, Some("DbxDiagram")),
                KeyBinding::new("cmd-r", app::RefreshData, None),
                KeyBinding::new("ctrl-r", app::RefreshData, None),
                KeyBinding::new(
                    "shift-cmd-f",
                    app::FormatQuery,
                    Some(editor::SQL_EDITOR_CONTEXT),
                ),
                KeyBinding::new(
                    "ctrl-shift-f",
                    app::FormatQuery,
                    Some(editor::SQL_EDITOR_CONTEXT),
                ),
                KeyBinding::new("up", app::CompletionUp, Some(editor::SQL_EDITOR_CONTEXT)),
                KeyBinding::new(
                    "down",
                    app::CompletionDown,
                    Some(editor::SQL_EDITOR_CONTEXT),
                ),
                KeyBinding::new(
                    "enter",
                    app::CompletionEnter,
                    Some(editor::SQL_EDITOR_CONTEXT),
                ),
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
                // Workspace navigation follows each platform's conventions:
                // ⌘ on macOS, Ctrl elsewhere.
                KeyBinding::new("cmd-n", app::NewConnection, None),
                KeyBinding::new("ctrl-n", app::NewConnection, None),
                KeyBinding::new("cmd-t", app::NewQuery, None),
                KeyBinding::new("ctrl-t", app::NewQuery, None),
                KeyBinding::new("cmd-w", app::CloseTab, None),
                KeyBinding::new("ctrl-w", app::CloseTab, None),
                KeyBinding::new("ctrl-tab", app::NextConnection, None),
                KeyBinding::new("ctrl-shift-tab", app::PreviousConnection, None),
                KeyBinding::new("cmd-shift-]", app::NextConnection, None),
                KeyBinding::new("cmd-shift-[", app::PreviousConnection, None),
                KeyBinding::new("ctrl-pagedown", app::NextConnection, None),
                KeyBinding::new("ctrl-pageup", app::PreviousConnection, None),
                KeyBinding::new("cmd-b", app::ToggleSidebar, None),
                KeyBinding::new("ctrl-b", app::ToggleSidebar, None),
                KeyBinding::new("f5", app::RefreshData, None),
                KeyBinding::new("cmd-m", Minimize, None),
                KeyBinding::new("ctrl-cmd-f", Zoom, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.on_action(|_: &Minimize, cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, _| window.minimize_window());
                }
            });
            cx.on_action(|_: &Zoom, cx| {
                if let Some(window) = cx.active_window() {
                    let _ = window.update(cx, |_, window, _| window.zoom_window());
                }
            });
            // DBX is a single-window app: closing it (titlebar, compositor, or
            // ⌘W-style window close) ends the process, as the old control did.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.set_menus(app_menus());
            cx.open_window(dbx_window_options(), |window, cx| {
                window.set_window_title(APP_NAME);
                cx.new(|cx| DbxApp::new(window, cx))
            })
            .expect("open DBX window");
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_window_preserves_native_move_and_resize_contract() {
        let options = dbx_window_options();

        assert!(options.is_movable);
        assert!(options.is_resizable);
        assert!(options.app_owns_titlebar_drag);
        let titlebar = options
            .titlebar
            .expect("macOS requires a titlebar style mask for native edge resizing");
        assert!(titlebar.appears_transparent);
        assert!(
            titlebar
                .traffic_light_position
                .is_some_and(|position| position.x >= px(0.0)),
            "Native traffic lights remain inside the window"
        );
    }
}
