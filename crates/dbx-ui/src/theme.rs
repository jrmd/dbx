#![allow(dead_code)] // Primitives are adopted incrementally across DBX screens.

use std::sync::{
    LazyLock,
    atomic::{AtomicU8, Ordering},
};

use gpui::prelude::FluentBuilder;
use gpui::{
    Div, ElementId, ParentElement, Rgba, SharedString, Styled, Svg, div, px, rgb, rgba, svg,
};
use gpui_component::button::{Button, ButtonVariants as _};

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
    pub accent_foreground: Rgba,
    pub accent_soft: Rgba,
    pub success: Rgba,
    pub danger: Rgba,
    pub warning: Rgba,
    pub grid_alternate: Rgba,
    pub rail: Rgba,
    pub focus_ring: Rgba,
    pub overlay: Rgba,
    pub selection: Rgba,
    pub sql_keyword: Rgba,
    pub sql_string: Rgba,
    pub sql_comment: Rgba,
    pub sql_number: Rgba,
    pub sql_parameter: Rgba,
    pub sql_identifier: Rgba,
    pub sql_type: Rgba,
}

/// DBX's low-glare operational palette.
pub static DARK_THEME: LazyLock<Theme> = LazyLock::new(|| Theme {
    canvas: rgb(0x0a0c10),
    panel: rgb(0x111318),
    panel_raised: rgb(0x171a20),
    border: rgb(0x1f232b),
    border_strong: rgb(0x343b47),
    text: rgb(0xf1f5f9),
    text_muted: rgb(0x94a3b8),
    accent: rgb(0x2563eb),
    accent_foreground: rgb(0xffffff),
    accent_soft: rgb(0x10294d),
    success: rgb(0x22c55e),
    danger: rgb(0xef4444),
    warning: rgb(0xf59e0b),
    grid_alternate: rgb(0x0e1116),
    rail: rgb(0x0d1016),
    focus_ring: rgb(0x60a5fa),
    overlay: rgba(0x00000088),
    selection: rgba(0x3311ff30),
    sql_keyword: rgb(0xc792ea),
    sql_string: rgb(0xc3e88d),
    sql_comment: rgb(0x737e8c),
    sql_number: rgb(0xf78c6c),
    sql_parameter: rgb(0xffcb6b),
    sql_identifier: rgb(0x82aaff),
    sql_type: rgb(0x89ddff),
});

/// A composed light palette for well-lit working environments. It keeps DBX's
/// blue action language and strong pane boundaries rather than inverting the
/// dark palette mechanically.
pub static LIGHT_THEME: LazyLock<Theme> = LazyLock::new(|| Theme {
    canvas: rgb(0xf7f9fc),
    panel: rgb(0xffffff),
    panel_raised: rgb(0xf0f4f8),
    border: rgb(0xd8dee8),
    border_strong: rgb(0xb6c2d1),
    text: rgb(0x16202f),
    text_muted: rgb(0x52657b),
    accent: rgb(0x1d5fd1),
    accent_foreground: rgb(0xffffff),
    accent_soft: rgb(0xe5f0ff),
    success: rgb(0x16803c),
    danger: rgb(0xc3333f),
    warning: rgb(0xa85e00),
    grid_alternate: rgb(0xf1f5f9),
    rail: rgb(0xebf0f6),
    focus_ring: rgb(0x0b63ce),
    overlay: rgba(0x0f172a4d),
    selection: rgba(0x1d5fd133),
    sql_keyword: rgb(0x7c3aed),
    sql_string: rgb(0x16794b),
    sql_comment: rgb(0x64748b),
    sql_number: rgb(0xc2410c),
    sql_parameter: rgb(0xa16207),
    sql_identifier: rgb(0x1d4ed8),
    sql_type: rgb(0x0369a1),
});

/// The application appearance selected by the user.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Appearance {
    Light = 1,
    #[default]
    Dark = 0,
}

impl Appearance {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

static CURRENT_APPEARANCE: AtomicU8 = AtomicU8::new(Appearance::Dark as u8);

/// Set the appearance used by DBX's semantic primitives.
pub fn set_appearance(appearance: Appearance) {
    CURRENT_APPEARANCE.store(appearance as u8, Ordering::Release);
}

/// Return the appearance currently used by DBX's semantic primitives.
pub fn appearance() -> Appearance {
    match CURRENT_APPEARANCE.load(Ordering::Acquire) {
        value if value == Appearance::Light as u8 => Appearance::Light,
        _ => Appearance::Dark,
    }
}

/// Return the semantic palette for the current appearance.
pub fn theme() -> &'static Theme {
    match appearance() {
        Appearance::Light => &LIGHT_THEME,
        Appearance::Dark => &DARK_THEME,
    }
}

/// A compact, consistent line-icon vocabulary for navigation and actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Icon {
    Database,
    Table,
    Query,
    Structure,
    Diagram,
    Search,
    Refresh,
    Settings,
    Sun,
    Moon,
    Add,
    Close,
    More,
    ArrowRight,
}

/// Draw a 16px icon from the embedded SVG set. Consumers provide the color so
/// active rail items and quiet secondary actions stay in the same grammar.
pub fn icon(kind: Icon, color: Rgba) -> Svg {
    let path = match kind {
        Icon::Database => assets::ICON_DATABASE,
        Icon::Table => assets::ICON_TABLE,
        Icon::Query => assets::ICON_QUERY,
        Icon::Structure => assets::ICON_STRUCTURE,
        Icon::Diagram => assets::ICON_DIAGRAM,
        Icon::Search => assets::ICON_SEARCH,
        Icon::Refresh => assets::ICON_REFRESH,
        Icon::Settings => assets::ICON_SETTINGS,
        Icon::Sun => assets::ICON_SUN,
        Icon::Moon => assets::ICON_MOON,
        Icon::Add => assets::ICON_ADD,
        Icon::Close => assets::ICON_CLOSE,
        Icon::More => assets::ICON_MORE,
        Icon::ArrowRight => assets::ICON_ARROW_RIGHT,
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
    let theme = theme();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(SPACE_2))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(theme.text_muted)
                .child(detail.into()),
        )
}

/// A primary-shell connection tab. Add an id and interaction handler at the
/// call site to keep this visual primitive usable in any screen state.
pub fn connection_tab(kind: DatabaseKind, label: impl Into<SharedString>, active: bool) -> Div {
    let theme = theme();
    div()
        .relative()
        .h(px(32.))
        .px(px(SPACE_3))
        .rounded_t(px(RADIUS_CONTROL))
        .border_1()
        .border_color(if active {
            theme.border_strong
        } else {
            theme.border
        })
        .bg(if active { theme.panel } else { theme.rail })
        .text_size(px(12.))
        .text_color(if active { theme.text } else { theme.text_muted })
        .flex()
        .items_center()
        .gap(px(SPACE_2))
        .child(database_logo(
            kind,
            if active {
                theme.accent
            } else {
                theme.text_muted
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
                    .bg(theme.accent),
            )
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ButtonKind {
    Primary,
    Quiet,
    Danger,
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
) -> Button {
    let theme = theme();
    let (background, border, text) = match kind {
        ButtonKind::Primary => (theme.accent, theme.accent, theme.accent_foreground),
        ButtonKind::Quiet => (theme.panel_raised, theme.border, theme.text),
        ButtonKind::Danger => (theme.panel_raised, theme.danger, theme.danger),
    };

    let button = Button::new(id)
        .label(label)
        .h(px(30.))
        .px(px(SPACE_3))
        .rounded(px(RADIUS_CONTROL))
        .border_1()
        .border_color(border)
        .bg(background)
        .text_size(px(12.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(text);

    match kind {
        ButtonKind::Primary => button.primary(),
        ButtonKind::Quiet => button.outline(),
        ButtonKind::Danger => button.danger().outline(),
    }
}

pub fn badge(label: impl Into<SharedString>, color: Rgba) -> Div {
    let theme = theme();
    div()
        .px(px(SPACE_2))
        .py(px(SPACE_1))
        .rounded_full()
        .bg(theme.panel_raised)
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(color)
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(color: Rgba) -> f32 {
        fn channel(value: f32) -> f32 {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    fn contrast(foreground: Rgba, background: Rgba) -> f32 {
        let foreground = relative_luminance(foreground);
        let background = relative_luminance(background);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    #[test]
    fn dark_reference_palette_is_preserved() {
        assert_eq!(DARK_THEME.canvas, rgb(0x0a0c10));
        assert_eq!(DARK_THEME.panel, rgb(0x111318));
        assert_eq!(DARK_THEME.border, rgb(0x1f232b));
        assert_eq!(DARK_THEME.accent, rgb(0x2563eb));
        assert_eq!(DARK_THEME.success, rgb(0x22c55e));
    }

    #[test]
    fn palettes_keep_semantic_roles_distinct() {
        for palette in [&*DARK_THEME, &*LIGHT_THEME] {
            assert_ne!(palette.canvas, palette.panel);
            assert_ne!(palette.panel, palette.panel_raised);
            assert_ne!(palette.text, palette.canvas);
            assert_ne!(palette.text_muted, palette.canvas);
            assert_ne!(palette.accent, palette.canvas);
            assert_ne!(palette.focus_ring, palette.canvas);
            assert_ne!(palette.success, palette.danger);
            assert_ne!(palette.warning, palette.danger);
        }
    }

    #[test]
    fn both_appearances_keep_operational_text_at_body_contrast() {
        for palette in [&*DARK_THEME, &*LIGHT_THEME] {
            for foreground in [
                palette.text,
                palette.text_muted,
                palette.sql_keyword,
                palette.sql_string,
                palette.sql_comment,
                palette.sql_number,
                palette.sql_parameter,
                palette.sql_identifier,
                palette.sql_type,
            ] {
                assert!(contrast(foreground, palette.canvas) >= 4.5);
            }
            assert!(contrast(palette.accent_foreground, palette.accent) >= 4.5);
        }
    }

    #[test]
    fn current_palette_follows_selected_appearance() {
        set_appearance(Appearance::Light);
        assert_eq!(theme().canvas, LIGHT_THEME.canvas);
        set_appearance(Appearance::Dark);
        assert_eq!(theme().canvas, DARK_THEME.canvas);
    }
}
