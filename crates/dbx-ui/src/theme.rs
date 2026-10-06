#![allow(dead_code)] // Primitives are adopted incrementally across DBX screens.

use std::sync::{
    LazyLock,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use gpui::prelude::FluentBuilder;
use gpui::{
    BoxShadow, Div, ElementId, InteractiveElement, ParentElement, Rgba, SharedString, Styled, Svg,
    WindowAppearance, WindowBackgroundAppearance, div, point, px, rgb, rgba, svg,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

use crate::assets;
use dbx_core::DatabaseKind;

/// The DBX shell uses a deliberately restrained density: controls align to a
/// four-pixel rhythm and panels earn their separation with a single border.
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const RADIUS_CONTROL: f32 = 7.0;
pub const RADIUS_PANEL: f32 = 12.0;
/// Floating glass panes (sidebars, popovers, dialogs) use a softer, larger
/// corner so they read as layers above the content sheet.
pub const RADIUS_GLASS: f32 = 16.0;
/// Gap between floating glass panes and the window edge or content sheet.
pub const GLASS_INSET: f32 = 8.0;

const _: () = {
    assert!(SPACE_1 < SPACE_2 && SPACE_2 < SPACE_3 && SPACE_3 < SPACE_4);
    assert!(RADIUS_CONTROL < RADIUS_PANEL && RADIUS_PANEL < RADIUS_GLASS);
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
    /// Tint laid over the blurred desktop backdrop at the window root.
    pub window: Rgba,
    /// Fill for chrome that floats on the backdrop: sidebars and toolbars.
    pub glass: Rgba,
    /// Denser glass for transient layers: menus, popovers, and dialogs.
    pub glass_raised: Rgba,
    /// Hover wash for controls sitting on glass.
    pub glass_hover: Rgba,
    /// Selected pill on glass (tabs, segmented controls, list rows).
    pub glass_selected: Rgba,
    /// Specular rim along the top edge of glass: the light-catching lip.
    pub rim: Rgba,
    /// Translucent separator that stays legible over any backdrop.
    pub hairline: Rgba,
    /// Ambient drop shadow under floating layers.
    pub shadow: Rgba,
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
    window: rgba(if cfg!(target_os = "macos") {
        0x0b0d12b8
    } else {
        0x0b0d12f2
    }),
    glass: rgba(0xffffff0b),
    glass_raised: rgba(0x1f2430fa),
    glass_hover: rgba(0xffffff12),
    glass_selected: rgba(0xffffff1f),
    rim: rgba(0xffffff29),
    hairline: rgba(0xffffff17),
    shadow: rgba(0x00000080),
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
    window: rgba(if cfg!(target_os = "macos") {
        0xeef1f6c2
    } else {
        0xeef1f6f4
    }),
    glass: rgba(0xffffff8c),
    glass_raised: rgba(0xfcfdfef9),
    glass_hover: rgba(0x0f172a0d),
    glass_selected: rgba(0xffffffe0),
    rim: rgba(0xffffffe6),
    hairline: rgba(0x0f172a1a),
    shadow: rgba(0x0f172a2e),
});

// The backdrop tint is denser off macOS: AppKit always frosts the backdrop,
// but many Linux compositors (and Windows without Mica) show it unblurred, so
// the desktop may only ghost through faintly without hurting legibility.

/// Opaque equivalents of the glass materials, used when the user asks the OS
/// or DBX to reduce transparency, or the platform cannot blur the backdrop.
static DARK_SOLID_THEME: LazyLock<Theme> = LazyLock::new(|| Theme {
    window: DARK_THEME.rail,
    glass: DARK_THEME.panel,
    glass_raised: DARK_THEME.panel_raised,
    glass_hover: rgb(0x1b1f27),
    glass_selected: rgb(0x232834),
    rim: rgba(0xffffff0f),
    hairline: DARK_THEME.border,
    ..*DARK_THEME
});

static LIGHT_SOLID_THEME: LazyLock<Theme> = LazyLock::new(|| Theme {
    window: LIGHT_THEME.rail,
    glass: rgb(0xf6f8fb),
    glass_raised: LIGHT_THEME.panel,
    glass_hover: rgb(0xe2e8f0),
    glass_selected: rgb(0xffffff),
    hairline: LIGHT_THEME.border,
    ..*LIGHT_THEME
});

/// The application appearance selected by the user. `System` follows the
/// operating system's light/dark setting live, which is the native default.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Appearance {
    #[default]
    System = 2,
    Light = 1,
    Dark = 0,
}

impl Appearance {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Light,
            2 => Self::System,
            _ => Self::Dark,
        }
    }
}

static CURRENT_APPEARANCE: AtomicU8 = AtomicU8::new(Appearance::System as u8);
static SYSTEM_IS_DARK: AtomicBool = AtomicBool::new(true);
static REDUCE_TRANSPARENCY: AtomicBool = AtomicBool::new(false);

/// Set the appearance preference used by DBX's semantic primitives.
pub fn set_appearance(appearance: Appearance) {
    CURRENT_APPEARANCE.store(appearance as u8, Ordering::Release);
}

/// Return the appearance preference currently selected.
pub fn appearance() -> Appearance {
    Appearance::from_u8(CURRENT_APPEARANCE.load(Ordering::Acquire))
}

/// Record the operating system's current appearance for `System` mode.
pub fn set_system_appearance(appearance: WindowAppearance) {
    let dark = matches!(
        appearance,
        WindowAppearance::Dark | WindowAppearance::VibrantDark
    );
    SYSTEM_IS_DARK.store(dark, Ordering::Release);
}

/// Whether the palette currently in effect is the dark one.
pub fn is_dark() -> bool {
    match appearance() {
        Appearance::Dark => true,
        Appearance::Light => false,
        Appearance::System => SYSTEM_IS_DARK.load(Ordering::Acquire),
    }
}

/// Replace glass materials with opaque equivalents.
pub fn set_reduce_transparency(reduce: bool) {
    REDUCE_TRANSPARENCY.store(reduce, Ordering::Release);
}

pub fn reduce_transparency() -> bool {
    // Keep AppKit's whole-window vibrancy out of the data workbench. The
    // translucent GPUI layers do not map cleanly to native sidebar materials.
    cfg!(target_os = "macos") || REDUCE_TRANSPARENCY.load(Ordering::Acquire)
}

/// The window backdrop that matches the current material setting. Blurred
/// maps to NSVisualEffectView on macOS and compositor blur on Wayland.
pub fn window_background() -> WindowBackgroundAppearance {
    if reduce_transparency() {
        WindowBackgroundAppearance::Opaque
    } else {
        WindowBackgroundAppearance::Blurred
    }
}

/// Return the semantic palette for the current appearance and material.
pub fn theme() -> &'static Theme {
    match (is_dark(), reduce_transparency()) {
        (true, false) => &DARK_THEME,
        (true, true) => &DARK_SOLID_THEME,
        (false, false) => &LIGHT_THEME,
        (false, true) => &LIGHT_SOLID_THEME,
    }
}

/// Push DBX's resolved palette into gpui-component so its menus, popovers,
/// tooltips, and inputs share the same glass materials and radii.
pub fn sync_component_theme(window: Option<&mut gpui::Window>, cx: &mut gpui::App) {
    gpui_component::Theme::change(
        if is_dark() {
            gpui_component::ThemeMode::Dark
        } else {
            gpui_component::ThemeMode::Light
        },
        window,
        cx,
    );
    let palette = theme();
    let component = gpui_component::Theme::global_mut(cx);
    component.radius = px(RADIUS_CONTROL);
    component.radius_lg = px(RADIUS_PANEL);
    component.shadow = true;
    component.colors.popover = palette.glass_raised.into();
    component.colors.popover_foreground = palette.text.into();
    component.colors.border = palette.hairline.into();
    component.colors.accent = palette.glass_selected.into();
    component.colors.accent_foreground = palette.text.into();
    component.colors.primary = palette.accent.into();
    component.colors.primary_foreground = palette.accent_foreground.into();
    component.colors.primary_hover = gpui::Hsla::from(palette.accent).opacity(0.9);
    component.colors.primary_active = gpui::Hsla::from(palette.accent).opacity(0.8);
    component.colors.secondary_hover = palette.glass_hover.into();
    component.colors.muted_foreground = palette.text_muted.into();
    component.colors.ring = palette.focus_ring.into();
    component.colors.caret = palette.focus_ring.into();
    component.colors.selection = palette.selection.into();
    component.colors.list_active = palette.accent_soft.into();
    component.colors.list_active_border = palette.accent.into();
    component.colors.button_primary = palette.accent.into();
    component.colors.button_primary_foreground = palette.accent_foreground.into();
    component.colors.button_primary_hover = component.colors.primary_hover;
    component.colors.button_primary_active = component.colors.primary_active;

    // Components read the resolved token table rather than `colors`, and
    // `Theme::change` resolved it from the stock palette. Re-resolve every
    // token DBX overrides so hover and active states match the rest state
    // (otherwise a primary button turns the stock near-white on hover).
    let colors = component.colors;
    let tokens = &mut component.tokens;
    tokens.popover = colors.popover.into();
    tokens.popover_foreground = colors.popover_foreground.into();
    tokens.border = colors.border.into();
    tokens.accent = colors.accent.into();
    tokens.accent_foreground = colors.accent_foreground.into();
    tokens.primary = colors.primary.into();
    tokens.primary_foreground = colors.primary_foreground.into();
    tokens.primary_hover = colors.primary_hover.into();
    tokens.primary_active = colors.primary_active.into();
    tokens.secondary_hover = colors.secondary_hover.into();
    tokens.muted_foreground = colors.muted_foreground.into();
    tokens.ring = colors.ring.into();
    tokens.caret = colors.caret.into();
    tokens.selection = colors.selection.into();
    tokens.list_active = colors.list_active.into();
    tokens.list_active_border = colors.list_active_border.into();
    tokens.button_primary = colors.button_primary.into();
    tokens.button_primary_foreground = colors.button_primary_foreground.into();
    tokens.button_primary_hover = colors.button_primary_hover.into();
    tokens.button_primary_active = colors.button_primary_active.into();
}

/// A keyboard shortcut label in the platform's own notation, e.g.
/// `shortcut("↵", "Enter")` → "⌘↵" on macOS and "Ctrl+Enter" elsewhere.
pub fn shortcut(mac_key: &str, key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("⌘{mac_key}")
    } else {
        format!("Ctrl+{key}")
    }
}

/// Tooltip builder for plain gpui elements.
pub fn tip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    let text = text.into();
    move |window, cx| gpui_component::tooltip::Tooltip::new(text.clone()).build(window, cx)
}

/// Soft ambient shadow plus a one-pixel specular rim on the top edge: the two
/// cues that make a translucent layer read as a physical sheet of glass.
pub fn glass_shadow(elevation: f32) -> Vec<BoxShadow> {
    let theme = theme();
    vec![
        BoxShadow {
            color: theme.shadow.into(),
            offset: point(px(0.), px(elevation * 0.5)),
            blur_radius: px(elevation * 2.),
            spread_radius: px(0.),
            inset: false,
        },
        BoxShadow {
            color: theme.rim.into(),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: true,
        },
    ]
}

/// GPUI clips `overflow_hidden` to the bounding rectangle, not the rounded
/// rect, so a child with its own background (a tab strip, header row or
/// footer) paints square over its parent's curved corners. Children that touch
/// a rounded container's corners take the container's radius, less its
/// one-pixel border, on the corners they share.
pub trait FollowCorners: Styled + Sized {
    /// The child's top edge sits on the container's top corners.
    fn follow_top_corners(self, outer_radius: f32) -> Self {
        let radius = px((outer_radius - 1.).max(0.));
        self.rounded_tl(radius).rounded_tr(radius)
    }

    /// The child's bottom edge sits on the container's bottom corners.
    fn follow_bottom_corners(self, outer_radius: f32) -> Self {
        let radius = px((outer_radius - 1.).max(0.));
        self.rounded_bl(radius).rounded_br(radius)
    }
}

impl<E: Styled + Sized> FollowCorners for E {}

/// Style any element as a floating pane of glass.
pub fn glass<E: Styled>(element: E, radius: f32, elevation: f32) -> E {
    let theme = theme();
    element
        .bg(theme.glass)
        .border_1()
        .border_color(theme.hairline)
        .rounded(px(radius))
        .shadow(glass_shadow(elevation))
}

/// Style an element as a transient glass layer (menu, popover, dialog).
pub fn glass_raised<E: Styled>(element: E, radius: f32) -> E {
    let theme = theme();
    element
        .bg(theme.glass_raised)
        .border_1()
        .border_color(theme.hairline)
        .rounded(px(radius))
        .shadow(glass_shadow(24.))
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
    Minimize,
    Maximize,
    Restore,
    Sidebar,
    Appearance,
    Lock,
    Sparkles,
    Trash,
    Pencil,
    Tag,
    Download,
    ChevronUp,
    ChevronDown,
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
        Icon::Minimize => assets::ICON_MINIMIZE,
        Icon::Maximize => assets::ICON_MAXIMIZE,
        Icon::Restore => assets::ICON_RESTORE,
        Icon::Sidebar => assets::ICON_SIDEBAR,
        Icon::Appearance => assets::ICON_APPEARANCE,
        Icon::Lock => assets::ICON_LOCK,
        Icon::Sparkles => assets::ICON_SPARKLES,
        Icon::Trash => assets::ICON_TRASH,
        Icon::Pencil => assets::ICON_PENCIL,
        Icon::Tag => assets::ICON_TAG,
        Icon::Download => assets::ICON_DOWNLOAD,
        Icon::ChevronUp => assets::ICON_CHEVRON_UP,
        Icon::ChevronDown => assets::ICON_CHEVRON_DOWN,
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
        DatabaseKind::MongoDB => assets::LOGO_MONGODB,
        DatabaseKind::CockroachDB => assets::LOGO_COCKROACHDB,
        DatabaseKind::DuckDB => assets::LOGO_DUCKDB,
        DatabaseKind::Elasticsearch => assets::LOGO_ELASTICSEARCH,
        DatabaseKind::BigQuery => assets::LOGO_BIGQUERY,
        DatabaseKind::Kafka => assets::LOGO_KAFKA,
        DatabaseKind::Turso => assets::LOGO_TURSO,
        DatabaseKind::CloudflareD1 => assets::LOGO_CLOUDFLARE_D1,
        DatabaseKind::ClickHouse => assets::LOGO_CLICKHOUSE,
        DatabaseKind::SqlServer => assets::LOGO_SQLSERVER,
        DatabaseKind::Snowflake => assets::LOGO_SNOWFLAKE,
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

/// A primary-shell connection tab: a capsule that lifts into a glass pill
/// when selected. Add an id and interaction handler at the call site.
pub fn connection_tab(kind: DatabaseKind, label: impl Into<SharedString>, active: bool) -> Div {
    let theme = theme();
    div()
        .h(px(28.))
        .pl(px(10.))
        .pr(px(5.))
        .rounded_full()
        .text_size(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .when(active, |tab| {
            tab.bg(theme.glass_selected)
                .border_1()
                .border_color(theme.hairline)
                .shadow(glass_shadow(4.))
                .text_color(theme.text)
                .font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!active, |tab| {
            tab.text_color(theme.text_muted)
                .hover(|style| style.bg(theme.glass_hover).text_color(theme.text))
        })
        .child(database_logo(
            kind,
            if active {
                theme.accent
            } else {
                theme.text_muted
            },
        ))
        .child(label.into())
}

/// A segmented-control option: a pill that becomes the selected glass lens.
pub fn segment(label: impl Into<SharedString>, selected: bool) -> Div {
    let theme = theme();
    div()
        .h(px(26.))
        .px(px(11.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .cursor_pointer()
        .when(selected, |view| {
            view.bg(theme.glass_selected)
                .border_1()
                .border_color(theme.hairline)
                .shadow(glass_shadow(3.))
                .text_color(theme.text)
                .font_weight(gpui::FontWeight::MEDIUM)
        })
        .when(!selected, |view| {
            view.text_color(theme.text_muted)
                .hover(|style| style.bg(theme.glass_hover).text_color(theme.text))
        })
        .child(label.into())
}

/// The recessed track that holds a row of [`segment`]s.
pub fn segmented_track() -> Div {
    let theme = theme();
    div()
        .p(px(2.))
        .rounded_full()
        .bg(theme.glass_hover)
        .border_1()
        .border_color(theme.hairline)
        .flex()
        .items_center()
        .gap(px(2.))
}

/// A compact icon-only control that sits on glass.
pub fn glass_icon_button(
    id: impl Into<ElementId>,
    kind: Icon,
    active: bool,
) -> gpui::Stateful<Div> {
    let theme = theme();
    div()
        .id(id)
        .size(px(28.))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(active, |view| {
            view.bg(theme.glass_selected)
                .border_1()
                .border_color(theme.hairline)
                .shadow(glass_shadow(3.))
        })
        .when(!active, |view| {
            view.hover(|style| style.bg(theme.glass_hover))
        })
        .child(icon(
            kind,
            if active {
                theme.accent
            } else {
                theme.text_muted
            },
        ))
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
        ButtonKind::Quiet => (theme.glass_hover, theme.hairline, theme.text),
        ButtonKind::Danger => (theme.glass_hover, theme.hairline, theme.danger),
    };

    // gpui-component sizes the label from the button's `Size`, not from the
    // container's text style; XSmall is the 12px label DBX specifies.
    let button = Button::new(id)
        .label(label)
        .with_size(Size::XSmall)
        .h(px(28.))
        .px(px(SPACE_3 + 2.))
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(background)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(text);

    match kind {
        ButtonKind::Primary => button.primary().shadow(vec![BoxShadow {
            color: rgba(0xffffff33).into(),
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: true,
        }]),
        ButtonKind::Quiet => button.outline(),
        ButtonKind::Danger => button.danger().outline(),
    }
}

/// An inset group of settings rows, separated by hairlines, in the style of
/// the platform's own preference panes.
pub fn settings_group(rows: impl IntoIterator<Item = gpui::AnyElement>) -> Div {
    let theme = theme();
    div()
        .flex()
        .flex_col()
        .rounded(px(RADIUS_PANEL))
        .border_1()
        .border_color(theme.hairline)
        .bg(theme.panel)
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(|(index, row)| {
            div()
                .when(index > 0, |view| {
                    view.border_t_1().border_color(theme.hairline)
                })
                .child(row)
        }))
}

/// A settings row: a label and optional detail on the left, its control on
/// the right.
pub fn settings_row(
    label: impl gpui::IntoElement,
    detail: Option<SharedString>,
    control: impl gpui::IntoElement,
) -> Div {
    let theme = theme();
    div()
        .min_h(px(48.))
        .px(px(SPACE_4))
        .py(px(SPACE_2))
        .flex()
        .items_center()
        .gap(px(SPACE_4))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(13.)).text_color(theme.text).child(label))
                .when_some(detail, |view, detail| {
                    view.child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child(detail),
                    )
                }),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(SPACE_2))
                .child(control),
        )
}

pub fn badge(label: impl Into<SharedString>, color: Rgba) -> Div {
    let theme = theme();
    div()
        .px(px(SPACE_2))
        .py(px(2.))
        .rounded_full()
        .bg(theme.glass_hover)
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
        set_appearance(Appearance::System);
        set_system_appearance(WindowAppearance::Light);
        assert_eq!(theme().canvas, LIGHT_THEME.canvas);
        set_system_appearance(WindowAppearance::VibrantDark);
        assert_eq!(theme().canvas, DARK_THEME.canvas);
        set_appearance(Appearance::Dark);
    }

    #[test]
    fn reduced_transparency_makes_every_material_opaque() {
        for palette in [&*DARK_SOLID_THEME, &*LIGHT_SOLID_THEME] {
            for material in [palette.window, palette.glass, palette.glass_raised] {
                assert_eq!(material.a, 1.0);
            }
        }
        for palette in [&*DARK_THEME, &*LIGHT_THEME] {
            assert!(
                palette.window.a < 1.0,
                "glass needs a translucent backdrop tint"
            );
            // Content stays opaque so data never competes with the desktop.
            assert_eq!(palette.canvas.a, 1.0);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_workbench_always_uses_opaque_materials() {
        assert!(reduce_transparency());
        assert_eq!(window_background(), WindowBackgroundAppearance::Opaque);
        for material in [theme().window, theme().glass, theme().glass_raised] {
            assert_eq!(material.a, 1.0);
        }
    }

    #[test]
    fn glass_text_stays_legible_over_its_opaque_fallback() {
        for palette in [&*DARK_SOLID_THEME, &*LIGHT_SOLID_THEME] {
            for surface in [palette.window, palette.glass, palette.glass_raised] {
                assert!(contrast(palette.text, surface) >= 7.0);
                assert!(contrast(palette.text_muted, surface) >= 4.5);
            }
        }
    }
}
