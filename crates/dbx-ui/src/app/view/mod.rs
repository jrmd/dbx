mod chrome;
mod connection;
mod data;
mod diagram;
mod overlays;
mod query;

use super::*;

impl Render for DbxApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.compact_layout = window.bounds().size.width < px(900.);
        self.narrow_workspace = window.bounds().size.width < px(1180.);
        let content = if self.connection_picker_open || self.active_session().is_none() {
            self.render_connection(cx).into_any_element()
        } else {
            self.render_workspace(window, cx).into_any_element()
        };
        // The pane rail only makes sense once a connection is live.
        let connected = self
            .active_session()
            .is_some_and(|session| session.engine.is_some());
        div()
            .size_full()
            .flex()
            .bg(theme().canvas)
            .text_color(theme().text)
            .capture_key_down(cx.listener(|this, event, window, cx| {
                this.dismiss_overlay_on_escape(event, window, cx)
            }))
            .on_action(cx.listener(Self::refresh_action))
            .when(connected, |view| view.child(self.render_app_rail(cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_topbar(cx))
                    .child(content),
            )
    }
}
