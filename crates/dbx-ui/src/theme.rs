#![allow(dead_code)] // Primitives are adopted incrementally across DBX screens.

use std::sync::LazyLock;

use gpui::prelude::FluentBuilder;
use gpui::{Div, ParentElement, Rgba, SharedString, Styled, Svg, div, px, rgb, svg};

use crate::assets;
use dbx_core::DatabaseKind;

/// The DBX shell uses a deliberately restrained density: controls align to a
/// four-pixel rhythm and panels earn their separation with a single border.
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const RADIUS_CONTROL: f32 = 6.0;
pub const RADIUS_PANEL: f32 = 10.0;

const _: () = {
    assert!(SPACE_1 < SPACE_2 && SPACE_2 < SPACE_3 && SPACE_3 < SPACE_4);
    assert!(RADIUS_CONTROL < RADIUS_PANEL);
};

#[derive(Clone, Copy)]
pub struct Theme {
    pub canvas: Rgba,
    pub panel: Rgba,
    pub panel_raised: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub accent_soft: Rgba,
    pub success: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub grid_alternate: Rgba,
    pub rail: Rgba,
    pub focus_ring: Rgba,
    pub window_close: Rgba,
    pub window_minimize: Rgba,
    pub window_maximize: Rgba,
}

pub static THEME: LazyLock<Theme> = LazyLock::new(|| Theme {
    canvas: rgb(0x0a0c10),
    panel: rgb(0x111318),
    panel_raised: rgb(0x171a20),
    border: rgb(0x1f232b),
    border_strong: rgb(0x343b47),
    text: rgb(0xf1f5f9),
    text_muted: rgb(0x94a3b8),
    accent: rgb(0x3b82f6),
    accent_soft: rgb(0x10294d),
    success: rgb(0x22c55e),
    danger: rgb(0xef4444),
    warning: rgb(0xf59e0b),
    grid_alternate: rgb(0x0e1116),
    rail: rgb(0x0d1016),
    focus_ring: rgb(0x60a5fa),
    // Familiar traffic-light colors, used by DBX's app-owned titlebar on
    // every desktop platform rather than relying on platform defaults.
    window_close: rgb(0xff5f57),
    window_minimize: rgb(0xfebc2e),
    window_maximize: rgb(0x28c840),
});

/// A compact, consistent line-icon vocabulary for navigation and actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icon {
    Database,
    Table,
    Query,
    Structure,
    Search,
    Refresh,
    Settings,
    Add,
    Close,
    More,
}

/// Draw a 16px icon from the embedded SVG set. Consumers provide the color so
/// active rail items and quiet secondary actions stay in the same grammar.
pub fn icon(kind: Icon, color: Rgba) -> Svg {
    let path = match kind {
        Icon::Database => assets::ICON_DATABASE,
        Icon::Table => assets::ICON_TABLE,
        Icon::Query => assets::ICON_QUERY,
        Icon::Structure => assets::ICON_STRUCTURE,
        Icon::Search => assets::ICON_SEARCH,
        Icon::Refresh => assets::ICON_REFRESH,
        Icon::Settings => assets::ICON_SETTINGS,
        Icon::Add => assets::ICON_ADD,
        Icon::Close => assets::ICON_CLOSE,
        Icon::More => assets::ICON_MORE,
    };

    svg().path(path).size(px(16.)).text_color(color)
}

/// Draw the brand mark for a database engine. Like [`icon`], consumers provide
/// the color so the logo follows the active/inactive treatment of its host.
pub fn database_logo(kind: DatabaseKind, color: Rgba) -> Svg {
    let path = match kind {
        DatabaseKind::PostgreSQL => assets::LOGO_POSTGRESQL,
        DatabaseKind::MySQL => assets::LOGO_MYSQL,
        DatabaseKind::SQLite => assets::LOGO_SQLITE,
        DatabaseKind::Redis => assets::LOGO_REDIS,
    };

    svg().path(path).size(px(16.)).text_color(color)
}

/// Compact, label-first panel title treatment for panes and inspectors.
pub fn panel_header(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(SPACE_2))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(THEME.text)
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(THEME.text_muted)
                .child(detail.into()),
        )
}

/// A primary-shell connection tab. Add an id and interaction handler at the
/// call site to keep this visual primitive usable in any screen state.
pub fn connection_tab(kind: DatabaseKind, label: impl Into<SharedString>, active: bool) -> Div {
    div()
        .relative()
        .h(px(32.))
        .px(px(SPACE_3))
        .rounded_t(px(RADIUS_CONTROL))
        .border_1()
        .border_color(if active {
            THEME.border_strong
        } else {
            THEME.border
        })
        .bg(if active { THEME.panel } else { THEME.rail })
        .text_size(px(12.))
        .text_color(if active { THEME.text } else { THEME.text_muted })
        .flex()
        .items_center()
        .gap(px(SPACE_2))
        .child(database_logo(
            kind,
            if active {
                THEME.accent
            } else {
                THEME.text_muted
            },
        ))
        .child(label.into())
        .when(active, |tab| {
            tab.child(
                div()
                    .absolute()
                    .left(px(0.))
                    .right(px(0.))
                    .bottom(px(0.))
                    .h(px(2.))
                    .bg(THEME.accent),
            )
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonKind {
    Primary,
    Quiet,
    Danger,
}

pub fn button(label: impl Into<SharedString>, kind: ButtonKind) -> Div {
    let (background, border, text) = match kind {
        ButtonKind::Primary => (THEME.accent, THEME.accent, THEME.text),
        ButtonKind::Quiet => (THEME.panel_raised, THEME.border, THEME.text),
        ButtonKind::Danger => (THEME.panel_raised, THEME.danger, THEME.danger),
    };

    div()
        .h(px(30.))
        .px(px(SPACE_3))
        .rounded(px(RADIUS_CONTROL))
        .border_1()
        .border_color(border)
        .bg(background)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(text)
        .flex()
        .items_center()
        .justify_center()
        .child(label.into())
}

pub fn badge(label: impl Into<SharedString>, color: Rgba) -> Div {
    div()
        .px(px(SPACE_2))
        .py(px(SPACE_1))
        .rounded_full()
        .bg(THEME.panel_raised)
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(color)
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_palette_is_preserved() {
        assert_eq!(THEME.canvas, rgb(0x0a0c10));
        assert_eq!(THEME.panel, rgb(0x111318));
        assert_eq!(THEME.border, rgb(0x1f232b));
        assert_eq!(THEME.accent, rgb(0x3b82f6));
        assert_eq!(THEME.success, rgb(0x22c55e));
    }
}
