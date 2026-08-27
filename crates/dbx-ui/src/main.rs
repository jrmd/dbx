mod app;
mod assets;
mod connection_fields;
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
    App, AppContext, Bounds, KeyBinding, WindowBounds, WindowDecorations, WindowOptions, point, px,
    size,
};

gpui::actions!(dbx_ui, [Quit]);

const APP_NAME: &str = "DBX";

fn main() {
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            let appearance = settings::SettingsStore::new()
                .and_then(|store| store.load())
                .map(|settings| settings.appearance)
                .unwrap_or_else(|error| {
                    eprintln!("DBX could not load appearance settings: {error}");
                    theme::Appearance::Dark
                });
            theme::set_appearance(appearance);
            let component_mode = match appearance {
                theme::Appearance::Light => gpui_component::ThemeMode::Light,
                theme::Appearance::Dark => gpui_component::ThemeMode::Dark,
            };
            gpui_component::Theme::change(component_mode, None, cx);
            cx.bind_keys(editor::default_key_bindings());
            cx.bind_keys([
                KeyBinding::new("tab", app::VaultFocusNext, Some("VaultGate")),
                KeyBinding::new("shift-tab", app::VaultFocusPrevious, Some("VaultGate")),
                KeyBinding::new("enter", app::SubmitVault, Some("VaultGate")),
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
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(80.0), px(80.0)),
                        size(px(1440.0), px(900.0)),
                    ))),
                    window_min_size: Some(size(px(960.0), px(640.0))),
                    // DBX owns the titlebar so the window chrome follows the
                    // same compact active appearance as the rest of the shell.
                    // The custom controls are rendered by `DbxApp`.
                    titlebar: None,
                    app_owns_titlebar_drag: true,
                    window_decorations: Some(WindowDecorations::Client),
                    app_id: Some("dev.jrmd.dbx".into()),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title(APP_NAME);
                    cx.new(|cx| DbxApp::new(window, cx))
                },
            )
            .expect("open DBX window");
            cx.activate(true);
        });
}
