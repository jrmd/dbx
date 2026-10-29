//! Window geometry for the vault gate and the main workspace.
use gpui::{App, Bounds, Pixels, Window, px, size};

const GATE_WIDTH: f32 = 520.;
const GATE_HEIGHT: f32 = 680.;
const WORKSPACE_WIDTH: f32 = 1440.;
const WORKSPACE_HEIGHT: f32 = 900.;

pub(crate) fn initial_bounds(cx: &App) -> Bounds<Pixels> {
    let desired = size(px(GATE_WIDTH), px(GATE_HEIGHT));
    cx.primary_display()
        .map(|display| {
            let available = display.visible_bounds();
            let fitted = size(
                desired.width.min(available.size.width),
                desired.height.min(available.size.height),
            );
            Bounds::centered_at(available.center(), fitted)
        })
        .unwrap_or_else(|| Bounds::centered(None, desired, cx))
}

pub(crate) fn apply(window: &mut Window, unlocked: bool) {
    let (width, height) = if unlocked {
        (WORKSPACE_WIDTH, WORKSPACE_HEIGHT)
    } else {
        (GATE_WIDTH, GATE_HEIGHT)
    };
    #[cfg(target_os = "macos")]
    if crate::native_glass::center_window(window, width, height) {
        return;
    }
    // Compositors such as Wayland control window placement themselves.
    window.resize(size(px(width), px(height)));
}
