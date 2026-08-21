mod app;
mod assets;
mod connection_fields;
mod editor;
#[allow(dead_code)]
mod filters;
mod profiles;
#[allow(dead_code)]
mod row_drafts;
mod theme;

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
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            cx.bind_keys(editor::default_key_bindings());
            cx.bind_keys([
                KeyBinding::new("cmd-enter", app::RunQuery, None),
                KeyBinding::new("ctrl-enter", app::RunQuery, None),
                KeyBinding::new("cmd-r", app::RefreshData, None),
                KeyBinding::new("ctrl-r", app::RefreshData, None),
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
                    // same compact dark language as the rest of the shell.
                    // The custom controls are rendered by `DbxApp`.
                    titlebar: None,
                    app_owns_titlebar_drag: true,
                    window_decorations: Some(WindowDecorations::Client),
                    app_id: Some("dbx.jrmd.app".into()),
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
