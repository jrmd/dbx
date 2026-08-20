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
    App, AppContext, Application, Bounds, KeyBinding, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions, point, px, size,
};

const APP_NAME: &str = "DBX";

fn main() {
    Application::new()
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            cx.bind_keys(editor::default_key_bindings());
            cx.bind_keys([
                KeyBinding::new("cmd-enter", app::RunQuery, None),
                KeyBinding::new("ctrl-enter", app::RunQuery, None),
                KeyBinding::new("cmd-r", app::RefreshData, None),
                KeyBinding::new("ctrl-r", app::RefreshData, None),
            ]);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(80.0), px(80.0)),
                        size(px(1440.0), px(900.0)),
                    ))),
                    window_min_size: Some(size(px(960.0), px(640.0))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(SharedString::from(APP_NAME)),
                        ..Default::default()
                    }),
                    app_id: Some("dev.dbx.app".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| DbxApp::new(window, cx)),
            )
            .expect("open DBX window");
            cx.activate(true);
        });
}
