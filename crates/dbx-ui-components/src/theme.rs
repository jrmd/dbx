#![allow(dead_code)] // Primitives are adopted incrementally across DBX screens.

use std::sync::{
    LazyLock,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use gpui::prelude::FluentBuilder;
use gpui::{
    BoxShadow, Div, ElementId, InteractiveElement, ParentElement, Rgba, SharedString,
    StatefulInteractiveElement, Styled, Svg, WindowAppearance, WindowBackgroundAppearance, div,
    point, px, rgb, rgba, svg,
};
use gpui_component::{
    Sizable as _, Size,
    button::{Button, ButtonVariants as _},
};

#[path = "theme/catalog.rs"]
pub mod catalog;

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

#[derive(Clone, Copy, PartialEq)]
pub struct Theme {
    pub canvas: Rgba,
    pub panel: Rgba,
    pub panel_raised: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    /// Essential input edges, independent of decorative separators.
    pub input_border: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_active: Rgba,
    /// Small blue text needs more contrast than a filled primary button.
    pub accent_text: Rgba,
    pub accent_foreground: Rgba,
    pub accent_soft: Rgba,
    pub success: Rgba,
    pub success_text: Rgba,
    pub danger: Rgba,
    /// Error text can need a brighter red than a status dot or filled control.
    pub danger_text: Rgba,
    pub warning: Rgba,
    pub warning_text: Rgba,
    pub grid_alternate: Rgba,
    pub grid_active: Rgba,
    pub grid_selected: Rgba,
    pub grid_hover: Rgba,
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
    input_border: rgb(0x7b879b),
    text: rgb(0xf1f5f9),
    text_muted: rgb(0x94a3b8),
    accent: rgb(0x2563eb),
    accent_hover: rgb(0x1d4ed8),
    accent_active: rgb(0x1e40af),
    accent_text: rgb(0x82b1ff),
    accent_foreground: rgb(0xffffff),
    accent_soft: rgb(0x10294d),
    success: rgb(0x22c55e),
    success_text: rgb(0x22c55e),
    danger: rgb(0xef4444),
    danger_text: rgb(0xff818a),
    warning: rgb(0xf59e0b),
    warning_text: rgb(0xf59e0b),
    grid_alternate: rgb(0x0e1116),
    grid_active: rgb(0x19355a),
    grid_selected: rgb(0x182c49),
    grid_hover: rgb(0x192230),
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
    input_border: rgb(0x657287),
    text: rgb(0x16202f),
    text_muted: rgb(0x465b73),
    accent: rgb(0x1d5fd1),
    accent_hover: rgb(0x174fb3),
    accent_active: rgb(0x124293),
    accent_text: rgb(0x174fb3),
    accent_foreground: rgb(0xffffff),
    accent_soft: rgb(0xe5f0ff),
    success: rgb(0x16803c),
    success_text: rgb(0x106b31),
    danger: rgb(0xc3333f),
    danger_text: rgb(0xac2532),
    warning: rgb(0xa85e00),
    warning_text: rgb(0x884900),
    grid_alternate: rgb(0xf1f5f9),
    grid_active: rgb(0xb9d6fa),
    grid_selected: rgb(0xdce8fa),
    grid_hover: rgb(0xe7eef8),
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

thread_local! {
    static PRODUCTION_CONTEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PRODUCTION_PALETTE: std::cell::RefCell<Option<(Theme, bool, Theme)>> = const { std::cell::RefCell::new(None) };
    static NATIVE_PALETTE: std::cell::RefCell<Option<(Theme, bool, Theme)>> = const { std::cell::RefCell::new(None) };
    static MONO_FONT_SIZE: std::cell::Cell<u16> = const { std::cell::Cell::new(12) };
    static ROW_FONT_SIZE: std::cell::Cell<u16> = const { std::cell::Cell::new(12) };
    static FONT_SIZE: std::cell::Cell<u16> = const { std::cell::Cell::new(12) };
    static UI_SCALE: std::cell::Cell<u16> = const { std::cell::Cell::new(100) };
    static EDITOR_SCALE: std::cell::Cell<u16> = const { std::cell::Cell::new(100) };
    static DEFAULT_ROW_FONT_FAMILY: SharedString = gpui_component::Theme::default().mono_font_family;
    static ROW_FONT_FAMILY: std::cell::RefCell<Option<SharedString>> = const { std::cell::RefCell::new(None) };
}

/// Row values have their own font, independent of interface and editor fonts.
pub fn set_row_font_family(family: Option<&str>) {
    ROW_FONT_FAMILY.with_borrow_mut(|current| {
        if current.as_deref() != family {
            *current = family.map(|family| SharedString::from(family.to_owned()));
        }
    });
}

pub fn row_font_family() -> SharedString {
    ROW_FONT_FAMILY.with_borrow(|family| {
        family
            .clone()
            .unwrap_or_else(|| DEFAULT_ROW_FONT_FAMILY.with(Clone::clone))
    })
}

pub fn set_text_scales(ui: u16, editor: u16) {
    UI_SCALE.set(ui.clamp(100, 200));
    EDITOR_SCALE.set(editor.clamp(100, 200));
}

pub fn set_font_sizes(normal: u16, mono: u16, row: u16) {
    FONT_SIZE.set(normal.clamp(10, 20));
    MONO_FONT_SIZE.set(mono.clamp(10, 20));
    ROW_FONT_SIZE.set(row.clamp(10, 20));
}

pub fn row_text_px(value: f32) -> gpui::Pixels {
    ui_px(value * ROW_FONT_SIZE.get() as f32 / 12.)
}

pub fn mono_text_px(value: f32) -> gpui::Pixels {
    ui_px(value * MONO_FONT_SIZE.get() as f32 / 12.)
}

/// Text sizing is independent of spacing, icons, and window zoom.
pub fn text_px(value: f32) -> gpui::Pixels {
    ui_px(value * FONT_SIZE.get() as f32 / 12.)
}

/// Grow text-bearing controls by the extra line height, leaving spacing and icons unchanged.
pub fn control_height(base: f32) -> gpui::Pixels {
    ui_px(base + 18. * (font_scale().max(1.) - 1.))
}

pub fn font_scale() -> f32 {
    FONT_SIZE.get() as f32 / 12.
}

pub fn grid_row_height() -> gpui::Pixels {
    ui_px(30. + 18. * ((FONT_SIZE.get().max(ROW_FONT_SIZE.get()) as f32 / 12.).max(1.) - 1.))
}

pub fn ui_scale() -> f32 {
    UI_SCALE.get() as f32 / 100.
}
pub fn editor_scale() -> f32 {
    EDITOR_SCALE.get() as f32 / 100.
}

/// Design-space measurements scale together; measured screen coordinates stay
/// ordinary Pixels, avoiding double scaling in hit testing and dragging.
pub fn ui_px(value: f32) -> gpui::Pixels {
    gpui::px(value * ui_scale())
}

/// Set the appearance preference used by DBX's semantic primitives.
/// Serialises tests that change or depend on the process-wide appearance.
#[cfg(any(test, feature = "test-support"))]
pub fn appearance_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

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
    REDUCE_TRANSPARENCY.load(Ordering::Acquire)
        || (cfg!(target_os = "macos") && !native_glass_active())
}

static NATIVE_GLASS_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn set_native_glass_active(active: bool) {
    NATIVE_GLASS_ACTIVE.store(active, Ordering::Release);
}

pub fn native_glass_active() -> bool {
    NATIVE_GLASS_ACTIVE.load(Ordering::Acquire)
}

/// macOS 26 rounds toolbar windows to roughly 26pt. Content sheets inset
/// `GLASS_INSET` from that corner take the remainder so the curves stay
/// concentric; the solid shell keeps the standard panel radius.
pub const RADIUS_SHEET_GLASS: f32 = 18.0;

/// Corner radius for the main content sheets that sit in the window corners.
pub fn sheet_radius() -> f32 {
    if native_glass_active() {
        RADIUS_SHEET_GLASS
    } else {
        RADIUS_PANEL
    }
}

/// Corner radius of the floating navigation panels (explorer, connection list)
/// that sit beside the content sheet. The solid shell uses the sheet's own
/// radius so neighbouring panels match; Liquid Glass keeps the floating-glass
/// radius.
pub fn navigation_radius() -> f32 {
    if native_glass_active() {
        RADIUS_GLASS
    } else {
        sheet_radius()
    }
}

/// AppKit supplies Liquid Glass itself; GPUI must leave its Metal surface
/// transparent without installing the older NSVisualEffectView blur material.
pub fn window_background() -> WindowBackgroundAppearance {
    if reduce_transparency() {
        WindowBackgroundAppearance::Opaque
    } else if native_glass_active() {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Blurred
    }
}

/// Return the semantic palette for the current appearance and material.
pub fn theme() -> Theme {
    // Native glass leaves GPUI content opaque, so start from the active theme's
    // solid variant: flattening the translucent one onto the default canvas
    // would wash out every preset and custom theme.
    let mut palette =
        catalog::resolved_theme(is_dark(), reduce_transparency() || native_glass_active());
    if native_glass_active() && !reduce_transparency() {
        let original = palette;
        let dark = is_dark();
        palette = NATIVE_PALETTE.with_borrow_mut(|cache| {
            if let Some((base, was_dark, resolved)) = *cache
                && base == original
                && was_dark == dark
            {
                return resolved;
            }
            palette = native_material_palette(palette, dark);
            *cache = Some((original, dark, palette));
            palette
        });
    }
    if PRODUCTION_CONTEXT.get() {
        let original = palette;
        let dark = is_dark();
        if let Some(cached) = PRODUCTION_PALETTE.with_borrow(|cache| {
            cache
                .as_ref()
                .filter(|(base, was_dark, _)| *base == original && *was_dark == dark)
                .map(|(_, _, tinted)| *tinted)
        }) {
            return cached;
        }
        // Tint surfaces, never text or data-state colors. The connection badge
        // also names the environment so this cue does not rely on color alone.
        let tint = palette.danger;
        for surface in [
            &mut palette.window,
            &mut palette.canvas,
            &mut palette.panel,
            &mut palette.panel_raised,
            &mut palette.rail,
            &mut palette.glass,
            &mut palette.glass_raised,
        ] {
            let alpha = surface.a;
            *surface = Rgba {
                r: surface.r * 0.985 + tint.r * 0.015,
                g: surface.g * 0.985 + tint.g * 0.015,
                b: surface.b * 0.985 + tint.b * 0.015,
                a: alpha,
            };
        }
        catalog::ensure_readable(&mut palette, dark);
        if native_glass_active() && !reduce_transparency() {
            catalog::ensure_glass_readable(&mut palette, dark);
        }
        PRODUCTION_PALETTE.with_borrow_mut(|cache| *cache = Some((original, dark, palette)));
    }
    palette
}

/// Content surfaces are translucent enough for the blurred native backdrop to
/// show through, yet dense enough that ink contrast holds on any desktop.
const NATIVE_CANVAS_ALPHA: f32 = 0.90;
const NATIVE_PANEL_ALPHA: f32 = 0.97;

/// Keep this independent of global state for palette and contrast regression tests.
fn native_material_palette(mut palette: Theme, dark: bool) -> Theme {
    // AppKit blurs the desktop under a clear window; the content surfaces
    // below are the only wash on top of it.
    palette.window.a = 0.;
    let base = if dark {
        DARK_THEME.canvas
    } else {
        LIGHT_THEME.canvas
    };
    let opaque_canvas = catalog::composite(palette.canvas, base);
    palette.canvas = opaque_canvas;
    palette.canvas.a = NATIVE_CANVAS_ALPHA;
    for surface in [
        &mut palette.panel,
        &mut palette.panel_raised,
        &mut palette.grid_alternate,
        &mut palette.grid_active,
        &mut palette.grid_selected,
        &mut palette.grid_hover,
        &mut palette.glass_raised,
        &mut palette.glass,
        &mut palette.glass_hover,
        &mut palette.glass_selected,
    ] {
        *surface = catalog::composite(*surface, opaque_canvas);
    }
    palette.panel.a = NATIVE_PANEL_ALPHA;
    // AppKit owns the native glass highlights. Content keeps its own subtle
    // separators; focus and essential input edges retain their contrast.
    palette.rim = rgba(0);
    palette.hairline = rgba(0x00000024);
    palette.border = rgba(0x00000024);
    palette.border_strong = rgba(0x00000040);
    catalog::ensure_glass_readable(&mut palette, dark);
    palette
}

/// GPUI navigation inside documents is a stable content surface; AppKit owns glass.
pub fn navigation_surface() -> Rgba {
    let palette = theme();
    palette.panel
}

/// Data stays on stable content colours in every material mode.
pub fn table_row_background(alternate: bool) -> Rgba {
    let palette = theme();
    if alternate {
        palette.grid_alternate
    } else if native_glass_active() && !reduce_transparency() {
        // Keep text-bearing rows stable over every desktop backdrop.
        Rgba {
            a: 1.,
            ..palette.canvas
        }
    } else {
        palette.canvas
    }
}

/// Returns whether the active environment changed, for component theme sync.
pub fn set_production_context(production: bool) -> bool {
    PRODUCTION_CONTEXT.replace(production) != production
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
    component.radius = crate::theme::ui_px(RADIUS_CONTROL);
    // Menus, popovers, and dialogs are floating glass layers, so they take the
    // glass radius rather than the panel radius.
    component.radius_lg = crate::theme::ui_px(RADIUS_GLASS);
    component.shadow = true;
    component.native_glass_menus = native_glass_active() && !reduce_transparency();
    component.colors.background = palette.canvas.into();
    component.colors.foreground = palette.text.into();
    component.colors.input = palette.input_border.into();
    component.input_background_override = Some(palette.canvas.into());
    // DBX uses outlined destructive buttons. Keep their text red while the
    // interaction wash moves toward canvas instead of tinting red behind red.
    component.colors.danger = palette.danger_text.into();
    component.colors.popover = palette.glass_raised.into();
    component.colors.popover_foreground = palette.text.into();
    component.colors.border = palette.hairline.into();
    component.colors.accent = palette.glass_selected.into();
    component.colors.accent_foreground = palette.text.into();
    component.colors.primary = palette.accent.into();
    component.colors.primary_foreground = palette.accent_foreground.into();
    component.colors.primary_hover = palette.accent_hover.into();
    component.colors.primary_active = palette.accent_active.into();
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

    // The workspace already paints the translucent canvas behind the grid;
    // a second canvas fill would make that material effectively opaque.
    component.colors.table = if native_glass_active() && !reduce_transparency() {
        gpui::transparent_black().into()
    } else {
        palette.canvas.into()
    };
    component.colors.table_active = palette.grid_active.into();
    component.colors.table_active_border = palette.focus_ring.into();
    component.colors.table_even = table_row_background(true).into();
    component.colors.table_head = palette.panel_raised.into();
    component.colors.table_head_foreground = palette.text.into();
    component.colors.table_foot = palette.panel.into();
    component.colors.table_foot_foreground = palette.text_muted.into();
    component.colors.table_hover = palette.grid_hover.into();
    component.colors.table_row_border = palette.border.into();
    component.colors.list = palette.panel.into();
    component.colors.list_even = palette.grid_alternate.into();
    component.colors.list_head = palette.panel_raised.into();
    component.colors.list_hover = palette.glass_hover.into();
    component.colors.secondary = palette.panel_raised.into();
    component.colors.secondary_foreground = palette.text.into();
    component.colors.secondary_active = palette.glass_selected.into();
    component.colors.muted = palette.panel_raised.into();
    component.colors.scrollbar = palette.panel.into();
    component.colors.scrollbar_thumb = palette.border_strong.into();
    component.colors.scrollbar_thumb_hover = palette.text_muted.into();
    component.colors.tab = palette.panel.into();
    component.colors.tab_active = palette.canvas.into();
    component.colors.tab_foreground = palette.text_muted.into();
    component.colors.tab_active_foreground = palette.text.into();
    component.colors.tab_bar = palette.rail.into();
    component.colors.tab_bar_segmented = palette.panel_raised.into();
    component.colors.title_bar = palette.rail.into();
    component.colors.title_bar_border = palette.border.into();
    component.colors.sidebar = palette.glass.into();
    component.colors.sidebar_foreground = palette.text.into();
    component.colors.sidebar_border = palette.hairline.into();
    component.colors.sidebar_accent = palette.glass_selected.into();
    component.colors.sidebar_accent_foreground = palette.text.into();
    component.colors.sidebar_primary = palette.accent.into();
    component.colors.sidebar_primary_foreground = palette.accent_foreground.into();
    component.colors.status_bar = palette.panel.into();
    component.colors.status_bar_border = palette.border.into();
    component.colors.overlay = palette.overlay.into();
    component.colors.link = palette.accent_text.into();
    component.colors.link_hover = palette.accent_text.into();
    component.colors.link_active = palette.accent_text.into();
    component.colors.success = palette.success.into();
    component.colors.warning = palette.warning.into();
    component.colors.info = palette.accent.into();
    component.colors.progress_bar = palette.accent.into();
    component.colors.switch = palette.border_strong.into();
    component.colors.switch_thumb = palette.panel.into();
    component.colors.button = palette.panel_raised.into();
    component.colors.button_foreground = palette.text.into();
    component.colors.button_hover = palette.glass_hover.into();
    component.colors.button_active = palette.glass_selected.into();
    component.colors.accordion = palette.panel.into();
    component.colors.group_box = palette.panel.into();
    component.colors.group_box_foreground = palette.text.into();
    component.colors.description_list_label = palette.panel_raised.into();
    component.colors.description_list_label_foreground = palette.text_muted.into();
    component.colors.drag_border = palette.focus_ring.into();
    component.colors.drop_target = palette.accent_soft.into();
    component.colors.skeleton = palette.panel_raised.into();
    component.colors.slider_bar = palette.accent.into();
    component.colors.slider_thumb = palette.accent.into();
    component.colors.tiles = palette.panel.into();
    component.colors.window_border = palette.border.into();

    // Components read the resolved token table rather than `colors`, and
    // `Theme::change` resolved it from the stock palette. Re-resolve every
    // token DBX overrides so hover and active states match the rest state
    // (otherwise a primary button turns the stock near-white on hover).
    let colors = component.colors;
    let tokens = &mut component.tokens;
    tokens.popover = colors.popover.into();
    tokens.background = colors.background.into();
    tokens.input = colors.input.into();
    tokens.danger = gpui::Hsla::from(palette.canvas).into();
    tokens.danger_hover = gpui::Hsla::from(palette.canvas).into();
    tokens.danger_active = gpui::Hsla::from(palette.canvas).into();
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
    tokens.table = colors.table.into();
    tokens.table_active = colors.table_active.into();
    tokens.table_active_border = colors.table_active_border.into();
    tokens.table_even = colors.table_even.into();
    tokens.table_head = colors.table_head.into();
    tokens.table_head_foreground = colors.table_head_foreground.into();
    tokens.table_foot = colors.table_foot.into();
    tokens.table_foot_foreground = colors.table_foot_foreground.into();
    tokens.table_hover = colors.table_hover.into();
    tokens.table_row_border = colors.table_row_border.into();
    tokens.list = colors.list.into();
    tokens.list_even = colors.list_even.into();
    tokens.list_head = colors.list_head.into();
    tokens.list_hover = colors.list_hover.into();
    tokens.secondary = colors.secondary.into();
    tokens.secondary_foreground = colors.secondary_foreground.into();
    tokens.secondary_active = colors.secondary_active.into();
    tokens.muted = colors.muted.into();
    tokens.scrollbar = colors.scrollbar.into();
    tokens.scrollbar_thumb = colors.scrollbar_thumb.into();
    tokens.scrollbar_thumb_hover = colors.scrollbar_thumb_hover.into();
    tokens.tab = colors.tab.into();
    tokens.tab_active = colors.tab_active.into();
    tokens.tab_foreground = colors.tab_foreground.into();
    tokens.tab_active_foreground = colors.tab_active_foreground.into();
    tokens.tab_bar = colors.tab_bar.into();
    tokens.tab_bar_segmented = colors.tab_bar_segmented.into();
    tokens.title_bar = colors.title_bar.into();
    tokens.title_bar_border = colors.title_bar_border.into();
    tokens.sidebar = colors.sidebar.into();
    tokens.sidebar_foreground = colors.sidebar_foreground.into();
    tokens.sidebar_border = colors.sidebar_border.into();
    tokens.sidebar_accent = colors.sidebar_accent.into();
    tokens.sidebar_accent_foreground = colors.sidebar_accent_foreground.into();
    tokens.sidebar_primary = colors.sidebar_primary.into();
    tokens.sidebar_primary_foreground = colors.sidebar_primary_foreground.into();
    tokens.status_bar = colors.status_bar.into();
    tokens.status_bar_border = colors.status_bar_border.into();
    tokens.overlay = colors.overlay.into();
    tokens.link = colors.link.into();
    tokens.link_hover = colors.link_hover.into();
    tokens.link_active = colors.link_active.into();
    tokens.success = colors.success.into();
    tokens.warning = colors.warning.into();
    tokens.info = colors.info.into();
    tokens.progress_bar = colors.progress_bar.into();
    tokens.switch = colors.switch.into();
    tokens.switch_thumb = colors.switch_thumb.into();
    tokens.button = colors.button.into();
    tokens.button_foreground = colors.button_foreground.into();
    tokens.button_hover = colors.button_hover.into();
    tokens.button_active = colors.button_active.into();
    tokens.accordion = colors.accordion.into();
    tokens.group_box = colors.group_box.into();
    tokens.group_box_foreground = colors.group_box_foreground.into();
    tokens.description_list_label = colors.description_list_label.into();
    tokens.description_list_label_foreground = colors.description_list_label_foreground.into();
    tokens.drag_border = colors.drag_border.into();
    tokens.drop_target = colors.drop_target.into();
    tokens.skeleton = colors.skeleton.into();
    tokens.slider_bar = colors.slider_bar.into();
    tokens.slider_thumb = colors.slider_thumb.into();
    tokens.tiles = colors.tiles.into();
    tokens.window_border = colors.window_border.into();
    tokens.foreground = colors.foreground.into();
    // Base owns a separate snapshot. Publishing only styled-component colors
    // leaves its focus rings and unstyled controls on the stock palette.
    gpui_component::Theme::sync_base(cx);
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
    // GPUI's outer shadow also paints underneath a pane. That dark fill shows
    // through translucent content and masks the native backdrop. AppKit owns
    // the window shadow and edge highlights; avoid a second painted rim.
    if native_glass_active() && !reduce_transparency() {
        return vec![];
    }
    painted_glass_shadow(elevation, theme().rim)
}

/// Shadow for pills that sit on the opaque content sheet (document tabs and
/// tab-group sub-tabs). Unlike [`glass_shadow`] this paints even under native
/// Liquid Glass: AppKit owns the highlights of chrome floating on the window's
/// own glass, but the sheet is opaque and a pill on it must draw its own lift.
/// The rim uses the stronger border ink, which stays visible where the native
/// palette zeroes `rim`.
pub fn content_shadow(elevation: f32) -> Vec<BoxShadow> {
    painted_glass_shadow(elevation, theme().border_strong)
}

/// Soft ambient shadow plus a one-pixel specular rim on the top edge: the two
/// cues that make a translucent layer read as a physical sheet of glass.
fn painted_glass_shadow(elevation: f32, rim: Rgba) -> Vec<BoxShadow> {
    let theme = theme();
    vec![
        BoxShadow {
            color: theme.shadow.into(),
            offset: point(crate::theme::ui_px(0.), px(elevation * 0.5)),
            blur_radius: px(elevation * 2.),
            spread_radius: crate::theme::ui_px(0.),
            inset: false,
        },
        BoxShadow {
            color: rim.into(),
            offset: point(crate::theme::ui_px(0.), crate::theme::ui_px(1.)),
            blur_radius: crate::theme::ui_px(0.),
            spread_radius: crate::theme::ui_px(0.),
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
/// These float on the dimmed content, not on the window's own glass, so they
/// paint their own lift even under native Liquid Glass.
pub fn glass_raised<E: Styled>(element: E, radius: f32) -> E {
    let theme = theme();
    element
        .bg(theme.glass_raised)
        .border_1()
        .border_color(theme.hairline)
        .rounded(px(radius))
        .shadow(content_shadow(24.))
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
    Fingerprint,
    Expand,
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
        Icon::Fingerprint => assets::ICON_FINGERPRINT,
        Icon::Expand => assets::ICON_EXPAND,
        Icon::Sparkles => assets::ICON_SPARKLES,
        Icon::Trash => assets::ICON_TRASH,
        Icon::Pencil => assets::ICON_PENCIL,
        Icon::Tag => assets::ICON_TAG,
        Icon::Download => assets::ICON_DOWNLOAD,
        Icon::ChevronUp => assets::ICON_CHEVRON_UP,
        Icon::ChevronDown => assets::ICON_CHEVRON_DOWN,
    };

    svg()
        .path(path)
        .size(crate::theme::ui_px(16.))
        .text_color(color)
}

/// Draw the brand mark for a database engine. Like [`icon`], consumers provide
/// the color so the logo follows the active/inactive treatment of its host.
pub fn database_logo_path(kind: DatabaseKind) -> &'static str {
    match kind {
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
    }
}

pub fn database_logo(kind: DatabaseKind, color: Rgba) -> Svg {
    let path = database_logo_path(kind);
    svg()
        .path(path)
        .size(crate::theme::ui_px(16.))
        .text_color(color)
}

/// Compact, label-first panel title treatment for panes and inspectors.
pub fn panel_header(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Div {
    let theme = theme();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(crate::theme::ui_px(SPACE_2))
        .child(
            div()
                .text_size(crate::theme::text_px(13.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title.into()),
        )
        .child(
            div()
                .text_size(crate::theme::text_px(11.))
                .text_color(theme.text_muted)
                .child(detail.into()),
        )
}

/// The shared liquid-glass tab capsule. Connection tabs in the titlebar and
/// document tabs on the content sheet are the same pill, so both build on
/// this. The resting state keeps a transparent one-pixel border, so the
/// hairline that appears when a tab is selected never shifts the strip.
pub fn glass_tab(selected: bool) -> Div {
    let theme = theme();
    div()
        .h(crate::theme::control_height(28.))
        .pl(crate::theme::ui_px(10.))
        .pr(crate::theme::ui_px(4.))
        .rounded_full()
        .border_1()
        .border_color(gpui::transparent_black())
        .text_size(crate::theme::text_px(12.))
        .flex_none()
        .flex()
        .items_center()
        .gap(crate::theme::ui_px(6.))
        .cursor_pointer()
        .when(selected, |tab| {
            tab.bg(theme.glass_selected)
                .border_color(theme.hairline)
                // Content-sheet pills draw their own lift even under native
                // Liquid Glass, where AppKit only owns the window's chrome.
                .shadow(content_shadow(4.))
                .text_color(theme.text)
                .font_weight(gpui::FontWeight::MEDIUM)
                .hover(|style| style.border_color(theme.border_strong))
        })
        .when(!selected, |tab| {
            tab.text_color(theme.text_muted).hover(|style| {
                style
                    .bg(theme.glass_hover)
                    .border_color(theme.hairline)
                    .text_color(theme.text)
            })
        })
}

/// A smaller pill for a document tab nested inside an expanded tab-group
/// strip. Same glass language as [`glass_tab`], scaled down to sit in the
/// group's track.
pub fn glass_sub_tab(selected: bool) -> Div {
    let theme = theme();
    div()
        .h(crate::theme::control_height(24.))
        .px(crate::theme::ui_px(8.))
        .rounded_full()
        .border_1()
        .border_color(gpui::transparent_black())
        .text_size(crate::theme::text_px(11.))
        .flex_none()
        .flex()
        .items_center()
        .gap(crate::theme::ui_px(5.))
        .cursor_pointer()
        .when(selected, |tab| {
            tab.bg(theme.glass_selected)
                .border_color(theme.hairline)
                .shadow(content_shadow(3.))
                .text_color(theme.text)
                .font_weight(gpui::FontWeight::MEDIUM)
                .hover(|style| style.border_color(theme.border_strong))
        })
        .when(!selected, |tab| {
            tab.text_color(theme.text_muted).hover(|style| {
                style
                    .bg(theme.glass_hover)
                    .border_color(theme.hairline)
                    .text_color(theme.text)
            })
        })
}

/// A primary-shell connection tab: a capsule that lifts into a glass pill
/// when selected. Add an id and interaction handler at the call site.
pub fn connection_tab(kind: DatabaseKind, label: impl Into<SharedString>, active: bool) -> Div {
    let theme = theme();
    glass_tab(active)
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
        .h(crate::theme::control_height(26.))
        .px(crate::theme::ui_px(11.))
        .rounded_full()
        .border_1()
        .border_color(gpui::transparent_black())
        .font_weight(gpui::FontWeight::MEDIUM)
        .flex()
        .items_center()
        .gap(crate::theme::ui_px(6.))
        .text_size(crate::theme::text_px(12.))
        .cursor_pointer()
        .when(selected, |view| {
            view.bg(theme.glass_selected)
                .border_color(theme.hairline)
                .shadow(glass_shadow(3.))
                .text_color(theme.text)
                .font_weight(gpui::FontWeight::MEDIUM)
                .hover(|style| style.border_color(theme.border_strong))
        })
        .when(!selected, |view| {
            view.text_color(theme.text_muted).hover(|style| {
                style
                    .bg(theme.glass_hover)
                    .border_color(theme.hairline)
                    .text_color(theme.text)
            })
        })
        .child(label.into())
}

/// The recessed track that holds a row of [`segment`]s.
pub fn segmented_track() -> Div {
    let theme = theme();
    div()
        .p(crate::theme::ui_px(2.))
        .rounded_full()
        .bg(theme.glass_hover)
        .border_1()
        .border_color(theme.hairline)
        .flex()
        .items_center()
        .gap(crate::theme::ui_px(2.))
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
        .pressable(kind.label())
        .size(crate::theme::ui_px(28.))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(gpui::transparent_black())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(active, |view| {
            view.bg(theme.glass_selected)
                .border_color(theme.hairline)
                .shadow(glass_shadow(3.))
                .hover(|style| style.border_color(theme.border_strong))
        })
        .when(!active, |view| {
            view.hover(|style| style.bg(theme.glass_hover).border_color(theme.hairline))
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

/// Keyboard access for a clickable element that isn't a `Button`.
///
/// Tab reaches it, Enter and Space fire its `on_click` handler (GPUI turns
/// them into a click on a focused element), and a ring marks it while the
/// keyboard has focus. The ring is a shadow rather than a border, so focusing
/// never changes a control's size.
pub trait Pressable: StatefulInteractiveElement + Styled + Sized {
    fn pressable(self, label: impl Into<SharedString>) -> Self {
        self.role(gpui::Role::Button)
            .aria_label(label)
            .tab_index(0)
            .focus_visible(|style| style.shadow(focus_ring()))
    }
}

impl<T: StatefulInteractiveElement + Styled> Pressable for T {}

impl Icon {
    pub fn label(self) -> &'static str {
        match self {
            Self::Database => "Connections",
            Self::Table => "Data",
            Self::Query => "Query",
            Self::Structure => "Structure",
            Self::Diagram => "Diagram",
            Self::Search => "Search",
            Self::Refresh => "Refresh",
            Self::Settings => "Settings",
            Self::Sun => "Light theme",
            Self::Moon => "Dark theme",
            Self::Add => "Add",
            Self::Close => "Close",
            Self::More => "More actions",
            Self::ArrowRight => "Open",
            Self::Minimize => "Minimize window",
            Self::Maximize => "Maximize window",
            Self::Restore => "Restore window",
            Self::Sidebar => "Toggle explorer",
            Self::Appearance => "Appearance",
            Self::Lock => "Lock vault",
            Self::Fingerprint => "Touch ID",
            Self::Expand => "Expand",
            Self::Sparkles => "Query assistant",
            Self::Trash => "Delete",
            Self::Pencil => "Edit",
            Self::Tag => "Tags",
            Self::Download => "Export",
            Self::ChevronUp => "Previous",
            Self::ChevronDown => "Next",
        }
    }
}

/// Add metadata to the library button's existing interactive node. Keeping
/// the same node preserves its disabled, focus, and click behavior.
pub trait ButtonAccessibility {
    fn accessible_name(self, label: impl Into<SharedString>) -> Self;
}

struct ButtonSemantics<'a>(&'a mut Button);
impl InteractiveElement for ButtonSemantics<'_> {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.0.interactivity()
    }
}
impl StatefulInteractiveElement for ButtonSemantics<'_> {}
impl ButtonAccessibility for Button {
    fn accessible_name(mut self, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        ButtonSemantics(&mut self)
            .aria_label(label.clone())
            .a11y_synthetic_children(move |builder| {
                builder.parent_node().set_label(label.to_string());
            });
        self
    }
}

/// A labelled switch for settings rows that already render their own title.
pub fn settings_switch(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .pressable(label)
        .role(gpui::Role::Switch)
        .aria_toggled(if checked {
            gpui::accesskit::Toggled::True
        } else {
            gpui::accesskit::Toggled::False
        })
        .w(crate::theme::ui_px(36.))
        .h(crate::theme::control_height(28.))
        .flex()
        .items_center()
        .cursor_pointer()
        .child(
            div()
                .w(crate::theme::ui_px(36.))
                .h(crate::theme::ui_px(20.))
                .rounded_full()
                .border_1()
                .border_color(theme().input_border)
                .bg(if checked {
                    theme().accent
                } else {
                    theme().canvas
                })
                .flex()
                .items_center()
                .px(crate::theme::ui_px(2.))
                .when(checked, |view| view.justify_end())
                .child(
                    div()
                        .size(crate::theme::ui_px(14.))
                        .rounded_full()
                        .bg(if checked {
                            theme().accent_foreground
                        } else {
                            theme().text_muted
                        }),
                ),
        )
}

/// The ring drawn around keyboard-focused controls that aren't `Button`s.
pub fn focus_ring() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: theme().focus_ring.into(),
        offset: point(crate::theme::ui_px(0.), crate::theme::ui_px(0.)),
        blur_radius: crate::theme::ui_px(0.),
        spread_radius: crate::theme::ui_px(2.),
        // Inset: a plain shadow paints under the whole element, which fills a
        // transparent control solid instead of outlining it.
        inset: true,
    }]
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
        ButtonKind::Danger => (theme.glass_hover, theme.hairline, theme.danger_text),
    };

    // gpui-component sizes the label from the button's `Size`, not from the
    // container's text style; XSmall is the 12px label DBX specifies.
    let button = Button::new(id)
        .label(label)
        .with_size(Size::XSmall)
        .flex_shrink_0()
        .text_size(crate::theme::text_px(12.))
        .h(crate::theme::control_height(28.))
        .px(px(SPACE_3 + 2.))
        .rounded_full()
        .border_1()
        .border_color(border)
        .bg(background)
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(text);

    match kind {
        ButtonKind::Primary => button.primary().shadow(vec![BoxShadow {
            color: if native_glass_active() && !reduce_transparency() {
                rgba(0).into()
            } else {
                rgba(0xffffff33).into()
            },
            offset: point(crate::theme::ui_px(0.), crate::theme::ui_px(1.)),
            blur_radius: crate::theme::ui_px(0.),
            spread_radius: crate::theme::ui_px(0.),
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
        .rounded(crate::theme::ui_px(RADIUS_PANEL))
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
        .min_h(crate::theme::ui_px(48.))
        .px(crate::theme::ui_px(SPACE_4))
        .py(crate::theme::ui_px(SPACE_2))
        .flex()
        .items_center()
        .gap(crate::theme::ui_px(SPACE_4))
        .when(ui_scale() * font_scale() > 1.25, |view| {
            view.flex_col().items_start()
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(crate::theme::ui_px(2.))
                .child(
                    div()
                        .text_size(crate::theme::text_px(13.))
                        .text_color(theme.text)
                        .child(label),
                )
                .when_some(detail, |view, detail| {
                    view.child(
                        div()
                            .text_size(crate::theme::text_px(11.))
                            .text_color(theme.text_muted)
                            .child(detail),
                    )
                }),
        )
        .child(
            div()
                .min_w_0()
                .when(ui_scale() * font_scale() > 1.25, |view| view.w_full())
                .flex_none()
                .flex()
                .items_center()
                .gap(crate::theme::ui_px(SPACE_2))
                .child(control),
        )
}

pub fn badge(label: impl Into<SharedString>, color: Rgba) -> Div {
    let theme = theme();
    div()
        .px(crate::theme::ui_px(SPACE_2))
        .py(crate::theme::ui_px(2.))
        .rounded_full()
        .bg(theme.glass_hover)
        .text_size(crate::theme::text_px(10.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(color)
        .child(label.into())
}
