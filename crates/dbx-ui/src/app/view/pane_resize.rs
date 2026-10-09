//! Drag handles that resize the explorer sidebar and the row inspector.

use super::super::*;
use gpui::{ClickEvent, DragMoveEvent, MouseUpEvent};

const EXPLORER_MIN_WIDTH: f32 = 160.;
const EXPLORER_MAX_WIDTH: f32 = 560.;
pub(super) const INSPECTOR_MIN_WIDTH: f32 = 280.;
const INSPECTOR_DEFAULT_WIDTH: f32 = 330.;
/// How much of the grid stays visible beside a dragged-out inspector.
pub(super) const GRID_MIN_WIDTH: f32 = 320.;
const APP_RAIL_WIDTH: f32 = 48.;
const HANDLE_WIDTH: f32 = 8.;

/// The drag payload for a pane resize, naming the pane being resized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneResize {
    Explorer,
    Inspector,
}

impl Render for PaneResize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

impl DbxApp {
    pub(super) fn explorer_width(&self) -> f32 {
        self.explorer_width
            .unwrap_or(if self.compact_layout { 188. } else { 236. })
    }

    pub(super) fn inspector_width(&self) -> f32 {
        self.inspector_width.unwrap_or(INSPECTOR_DEFAULT_WIDTH)
    }

    /// A grab strip on the pane's inner edge: drag to resize, double-click to
    /// restore the default width. Its parent must be relatively positioned
    /// and resize through [`Self::resize_pane`].
    pub(super) fn pane_resize_handle(
        &self,
        pane: PaneResize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let resizing = self.pane_resizing == Some(pane);
        let (id, group) = match pane {
            PaneResize::Explorer => ("resize-explorer", "dbx-resize-explorer"),
            PaneResize::Inspector => ("resize-inspector", "dbx-resize-inspector"),
        };
        div()
            .id(id)
            .group(group)
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(HANDLE_WIDTH))
            // The explorer's handle fills the gap beside the glass; the
            // inspector's straddles its border with the grid.
            .map(|handle| match pane {
                PaneResize::Explorer => handle.right(px(-HANDLE_WIDTH)),
                PaneResize::Inspector => handle.left(px(-HANDLE_WIDTH / 2.)),
            })
            .flex()
            .justify_center()
            .cursor_col_resize()
            .occlude()
            .child(
                div()
                    .h_full()
                    .w(px(2.))
                    .when(resizing, |line| line.bg(theme().accent))
                    .when(!resizing, |line| {
                        line.group_hover(group, |line| line.bg(theme().border_strong))
                    }),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if event.click_count() == 2 {
                    match pane {
                        PaneResize::Explorer => this.explorer_width = None,
                        PaneResize::Inspector => this.inspector_width = None,
                    }
                    this.persist_settings(cx);
                    cx.notify();
                }
            }))
            .on_drag(pane, |pane, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| *pane)
            })
            .on_mouse_up(MouseButton::Left, cx.listener(Self::finish_pane_resize))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::finish_pane_resize))
    }

    /// Follow a handle drag; `event.bounds` are the resized pane's bounds.
    pub(super) fn resize_pane(
        &mut self,
        pane: PaneResize,
        event: &DragMoveEvent<PaneResize>,
        cx: &mut Context<Self>,
    ) {
        if *event.drag(cx) != pane {
            return;
        }
        let x = f32::from(event.event.position.x);
        match pane {
            PaneResize::Explorer => {
                // The handle is centered in the gap after the sidebar.
                let width = (x - f32::from(event.bounds.left()) - HANDLE_WIDTH / 2.)
                    .clamp(EXPLORER_MIN_WIDTH, EXPLORER_MAX_WIDTH);
                self.explorer_width = Some(width);
            }
            PaneResize::Inspector => {
                let right = f32::from(event.bounds.right());
                let explorer = if self.sidebar_hidden {
                    0.
                } else {
                    self.explorer_width() + GLASS_INSET
                };
                let max =
                    (right - APP_RAIL_WIDTH - explorer - GRID_MIN_WIDTH).max(INSPECTOR_MIN_WIDTH);
                self.inspector_width = Some((right - x).clamp(INSPECTOR_MIN_WIDTH, max));
            }
        }
        self.pane_resizing = Some(pane);
        cx.notify();
    }

    fn finish_pane_resize(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.pane_resizing.take().is_some() {
            self.persist_settings(cx);
            cx.notify();
        }
    }
}
