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
        if window.focused(cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
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
            .relative()
            .flex()
            .flex_col()
            .bg(theme().window)
            .text_color(theme().text)
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(|this, event, window, cx| {
                this.dismiss_overlay_on_escape(event, window, cx)
            }))
            .on_action(cx.listener(Self::refresh_action))
            .on_action(cx.listener(Self::new_connection_action))
            .on_action(cx.listener(Self::new_query_action))
            .on_action(cx.listener(Self::close_tab_action))
            .on_action(cx.listener(Self::next_connection_action))
            .on_action(cx.listener(Self::previous_connection_action))
            .on_action(cx.listener(Self::toggle_sidebar_action))
            .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| this.check_for_updates(cx)))
            .child(self.render_topbar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .flex()
                    .when(connected, |view| view.child(self.render_app_rail(cx)))
                    .when(!connected, |view| view.pl(px(GLASS_INSET)))
                    .child(content),
            )
            .child(self.render_toasts(cx))
            .children(resize_edges(window))
    }
}

impl DbxApp {
    /// Glass capsules stacked above the status bar; click to dismiss early.
    fn render_toasts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(76.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .children(self.toasts.iter().map(|toast| {
                let id = toast.id;
                let accent = match toast.kind {
                    ToastKind::Info => theme().accent,
                    ToastKind::Success => theme().success,
                    ToastKind::Error => theme().danger,
                };
                glass_raised(div(), RADIUS_GLASS + 4.)
                    .id(SharedString::from(format!("toast-{id}")))
                    .rounded_full()
                    .max_w(px(520.))
                    .h(px(36.))
                    .pl(px(14.))
                    .pr(px(18.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .occlude()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().size(px(8.)).flex_none().rounded_full().bg(accent))
                    .child(div().truncate().child(toast.message.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| this.dismiss_toast(id, cx)))
            }))
    }
}

/// Invisible edge handles for client-decorated windows. Without these, a
/// floating DBX window on a compositor that leaves decorations to the client
/// (GNOME on Wayland, for example) could not be resized from its edges.
fn resize_edges(window: &Window) -> Vec<Stateful<Div>> {
    const EDGE: f32 = 5.;
    const CORNER: f32 = 12.;
    let Decorations::Client { tiling } = window.window_decorations() else {
        return Vec::new();
    };
    if window.is_maximized() || window.is_fullscreen() {
        return Vec::new();
    }
    let handle = |id: &'static str, edge: ResizeEdge, cursor: CursorStyle| {
        div()
            .id(id)
            .absolute()
            .occlude()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                window.start_window_resize(edge);
            })
    };
    let mut edges = Vec::new();
    if !tiling.top {
        edges.push(
            handle("resize-top", ResizeEdge::Top, CursorStyle::ResizeUpDown)
                .top_0()
                .left(px(CORNER))
                .right(px(CORNER))
                .h(px(EDGE)),
        );
    }
    if !tiling.bottom {
        edges.push(
            handle(
                "resize-bottom",
                ResizeEdge::Bottom,
                CursorStyle::ResizeUpDown,
            )
            .bottom_0()
            .left(px(CORNER))
            .right(px(CORNER))
            .h(px(EDGE)),
        );
    }
    if !tiling.left {
        edges.push(
            handle(
                "resize-left",
                ResizeEdge::Left,
                CursorStyle::ResizeLeftRight,
            )
            .left_0()
            .top(px(CORNER))
            .bottom(px(CORNER))
            .w(px(EDGE)),
        );
    }
    if !tiling.right {
        edges.push(
            handle(
                "resize-right",
                ResizeEdge::Right,
                CursorStyle::ResizeLeftRight,
            )
            .right_0()
            .top(px(CORNER))
            .bottom(px(CORNER))
            .w(px(EDGE)),
        );
    }
    let corners = [
        (
            "resize-top-left",
            ResizeEdge::TopLeft,
            CursorStyle::ResizeUpLeftDownRight,
            !tiling.top && !tiling.left,
        ),
        (
            "resize-top-right",
            ResizeEdge::TopRight,
            CursorStyle::ResizeUpRightDownLeft,
            !tiling.top && !tiling.right,
        ),
        (
            "resize-bottom-left",
            ResizeEdge::BottomLeft,
            CursorStyle::ResizeUpRightDownLeft,
            !tiling.bottom && !tiling.left,
        ),
        (
            "resize-bottom-right",
            ResizeEdge::BottomRight,
            CursorStyle::ResizeUpLeftDownRight,
            !tiling.bottom && !tiling.right,
        ),
    ];
    for (id, edge, cursor, enabled) in corners {
        if !enabled {
            continue;
        }
        let corner = handle(id, edge, cursor).size(px(CORNER));
        edges.push(match edge {
            ResizeEdge::TopLeft => corner.top_0().left_0(),
            ResizeEdge::TopRight => corner.top_0().right_0(),
            ResizeEdge::BottomLeft => corner.bottom_0().left_0(),
            _ => corner.bottom_0().right_0(),
        });
    }
    edges
}
