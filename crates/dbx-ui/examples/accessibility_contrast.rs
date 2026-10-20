//! Render DBX's actual controls on glass over controlled black/white pixels.
//! This harness has no profiles, vault, database connections or user settings.
use dbx_ui_components::{assets, theme};

use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
    WindowOptions, div, px, rgb,
};
use gpui_component::{
    Sizable, Size,
    checkbox::Checkbox,
    select::{SearchableVec, Select, SelectState},
};

struct ContrastQa {
    selections: [Entity<SelectState<SearchableVec<gpui::SharedString>>>; 2],
}

impl Render for ContrastQa {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::theme();
        div().size_full().flex().bg(palette.canvas).children(
            [
                ("Black desktop pixel", rgb(0x000000)),
                ("White desktop pixel", rgb(0xffffff)),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (title, desktop))| {
                div().flex_1().size_full().bg(desktop).child(
                    div()
                        .size_full()
                        .bg(palette.window)
                        .p(px(24.))
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .text_color(palette.text)
                        .text_size(px(12.))
                        .child(div().text_size(px(18.)).child(title))
                        .child(
                            theme::glass(div(), 16., 0.)
                                .p(px(20.))
                                .flex()
                                .flex_col()
                                .gap(px(16.))
                                .child("Glass pane: normal text")
                                .child(
                                    div()
                                        .text_color(palette.text_muted)
                                        .child("Muted metadata remains readable"),
                                )
                                .child(
                                    div()
                                        .text_color(palette.accent_text)
                                        .child("Blue help text"),
                                )
                                .child(
                                    div()
                                        .text_color(palette.danger_text)
                                        .child("Could not open the database"),
                                )
                                .child(
                                    div()
                                        .text_color(palette.success_text)
                                        .child("Connection test succeeded"),
                                )
                                .child(
                                    div()
                                        .text_color(palette.warning_text)
                                        .child("Results were limited"),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(12.))
                                        .child(
                                            theme::button(
                                                gpui::SharedString::from(format!(
                                                    "{title}-primary"
                                                )),
                                                "Connect",
                                                theme::ButtonKind::Primary,
                                            )
                                            .on_click(|_, _, _| {}),
                                        )
                                        .child(
                                            theme::button(
                                                gpui::SharedString::from(format!("{title}-danger")),
                                                "Delete…",
                                                theme::ButtonKind::Danger,
                                            )
                                            .on_click(|_, _, _| {}),
                                        ),
                                )
                                .child(theme::settings_switch(
                                    gpui::SharedString::from(format!("{title}-switch")),
                                    "Reduce motion",
                                    false,
                                ))
                                .child(
                                    Checkbox::new(gpui::SharedString::from(format!(
                                        "{title}-checkbox"
                                    )))
                                    .label("Remember this device"),
                                ),
                        )
                        .child(
                            theme::glass_raised(div(), 16.)
                                .p(px(20.))
                                .flex()
                                .flex_col()
                                .gap(px(16.))
                                .child("Raised surface: normal text")
                                .child(
                                    div()
                                        .text_color(palette.text_muted)
                                        .child("Optional filter value"),
                                )
                                .child(
                                    div()
                                        .text_color(palette.danger_text)
                                        .child("Import could not be previewed"),
                                )
                                .child(
                                    Select::new(&self.selections[index])
                                        .with_size(Size::Small)
                                        .accessibility_label("Filter column")
                                        .placeholder("Choose a column"),
                                ),
                        ),
                )
            }),
        )
    }
}

fn main() {
    let appearance = if std::env::args().nth(1).as_deref() == Some("light") {
        theme::Appearance::Light
    } else {
        theme::Appearance::Dark
    };
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            theme::set_appearance(appearance);
            theme::set_text_scales(100, 100);
            theme::set_reduce_transparency(false);
            theme::sync_component_theme(None, cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::new(
                        gpui::point(px(100.), px(100.)),
                        gpui::size(px(1200.), px(760.)),
                    ))),
                    window_background: gpui::WindowBackgroundAppearance::Opaque,
                    app_id: Some("dev.jrmd.dbx.contrast-qa".into()),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("DBX contrast QA");
                    cx.new(|cx| ContrastQa {
                        selections: std::array::from_fn(|_| {
                            cx.new(|cx| {
                                SelectState::new(
                                    SearchableVec::new(vec!["id".into(), "name".into()]),
                                    None,
                                    window,
                                    cx,
                                )
                            })
                        }),
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
}
