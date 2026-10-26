//! Portable DBX palettes and the shared token catalogue used by import and Settings.
use std::{cell::RefCell, collections::BTreeMap};

use gpui::{Rgba, rgb, rgba};
use serde::{Deserialize, Serialize};

use super::{Appearance, DARK_SOLID_THEME, DARK_THEME, LIGHT_SOLID_THEME, LIGHT_THEME, Theme};

pub type Colors = BTreeMap<String, String>;
pub const MAX_THEME_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    #[default]
    Default,
    Ocean,
    Forest,
    Plum,
    Sand,
    Graphite,
    Dracula,
    Nord,
    Solarized,
    Catppuccin,
}

impl Preset {
    pub const ALL: [Self; 10] = [
        Self::Default,
        Self::Ocean,
        Self::Forest,
        Self::Plum,
        Self::Sand,
        Self::Graphite,
        Self::Dracula,
        Self::Nord,
        Self::Solarized,
        Self::Catppuccin,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "DBX",
            Self::Ocean => "Ocean",
            Self::Forest => "Forest",
            Self::Plum => "Plum",
            Self::Sand => "Sand",
            Self::Graphite => "Graphite",
            Self::Dracula => "Dracula",
            Self::Nord => "Nord",
            Self::Solarized => "Solarized",
            Self::Catppuccin => "Catppuccin",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomTheme {
    pub name: String,
    #[serde(default)]
    pub light: Colors,
    #[serde(default)]
    pub dark: Colors,
}

impl CustomTheme {
    /// The appearance a single-variant theme was made for. VS Code themes
    /// declare light or dark and import into that variant only; DBX exports
    /// and edited presets define both and suit either.
    pub fn appearance(&self) -> Option<Appearance> {
        match (self.light.is_empty(), self.dark.is_empty()) {
            (false, true) => Some(Appearance::Light),
            (true, false) => Some(Appearance::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredPreferences")]
pub struct ThemePreferences {
    /// Base palette; also fills roles a custom theme leaves unset.
    pub preset: Preset,
    pub custom_themes: Vec<CustomTheme>,
    /// Index into `custom_themes`; `None` uses the preset.
    pub active_custom: Option<usize>,
}

/// Accepts the current shape and the earlier single custom slot.
#[derive(Deserialize)]
struct StoredPreferences {
    #[serde(default)]
    preset: Preset,
    #[serde(default)]
    custom_themes: Vec<CustomTheme>,
    #[serde(default)]
    active_custom: Option<usize>,
    #[serde(default)]
    custom: Option<CustomTheme>,
    #[serde(default)]
    use_custom: bool,
}

impl From<StoredPreferences> for ThemePreferences {
    fn from(stored: StoredPreferences) -> Self {
        let mut custom_themes = stored.custom_themes;
        let mut active_custom = stored.active_custom;
        if let Some(legacy) = stored.custom {
            custom_themes.push(legacy);
            if stored.use_custom {
                active_custom = Some(custom_themes.len() - 1);
            }
        }
        Self {
            preset: stored.preset,
            active_custom: active_custom.filter(|&index| index < custom_themes.len()),
            custom_themes,
        }
    }
}

impl ThemePreferences {
    pub fn active(&self) -> Option<&CustomTheme> {
        self.active_custom.and_then(|i| self.custom_themes.get(i))
    }

    /// The palette custom theme `index` would produce, for previews.
    pub fn resolve_custom(&self, index: usize, dark: bool, solid: bool) -> Theme {
        Self {
            active_custom: Some(index),
            ..self.clone()
        }
        .resolve(dark, solid)
    }

    pub fn resolve(&self, dark: bool, solid: bool) -> Theme {
        let mut palette = preset_theme(self.preset, dark, solid);
        if let Some(custom) = self.active() {
            let colors = if dark { &custom.dark } else { &custom.light };
            for token in TOKENS {
                if let Some(color) = colors.get(token.key).and_then(|v| parse_color(v).ok()) {
                    (token.set)(&mut palette, color);
                }
            }
            ensure_readable(&mut palette, dark);
        }
        // Respect reduced transparency even for imported glass materials.
        if solid {
            palette.window = composite(palette.window, palette.rail);
            palette.glass = composite(palette.glass, palette.window);
            palette.glass_raised = composite(palette.glass_raised, palette.window);
        }
        palette
    }

    pub fn edit(&mut self, key: &str, value: &str, dark: bool) -> Result<(), String> {
        if !TOKENS.iter().any(|token| token.key == key) {
            return Err(format!("Unknown colour: {key}"));
        }
        let value = color_hex(parse_color(value)?);
        let index = match self.active_custom {
            Some(index) => index,
            // Snapshot both variants so switching away from a preset never loses its colours.
            None => {
                let base = format!("Custom {}", self.preset.label());
                let name = (1..)
                    .map(|n| {
                        if n == 1 {
                            base.clone()
                        } else {
                            format!("{base} {n}")
                        }
                    })
                    .find(|name| self.custom_themes.iter().all(|t| &t.name != name))
                    .unwrap();
                self.add(self.snapshot(name))
            }
        };
        let custom = &mut self.custom_themes[index];
        let colors = if dark {
            &mut custom.dark
        } else {
            &mut custom.light
        };
        colors.insert(key.into(), value);
        Ok(())
    }

    /// Adds and activates `theme`. A theme with the same name is replaced,
    /// so re-importing a file updates it rather than duplicating it.
    pub fn add(&mut self, theme: CustomTheme) -> usize {
        let index = match self.custom_themes.iter().position(|t| t.name == theme.name) {
            Some(index) => {
                self.custom_themes[index] = theme;
                index
            }
            None => {
                self.custom_themes.push(theme);
                self.custom_themes.len() - 1
            }
        };
        self.active_custom = Some(index);
        index
    }

    /// Removes custom theme `index`; if it was active, the preset takes over.
    pub fn remove(&mut self, index: usize) {
        if index >= self.custom_themes.len() {
            return;
        }
        self.custom_themes.remove(index);
        self.active_custom = match self.active_custom {
            Some(active) if active == index => None,
            Some(active) if active > index => Some(active - 1),
            active => active,
        };
    }

    pub fn snapshot(&self, name: String) -> CustomTheme {
        let colors = |dark| {
            let palette = self.resolve(dark, false);
            TOKENS
                .iter()
                .map(|token| (token.key.to_string(), color_hex((token.get)(&palette))))
                .collect()
        };
        CustomTheme {
            name,
            light: colors(false),
            dark: colors(true),
        }
    }
}

thread_local! {
    static PALETTES: RefCell<Option<[Theme; 4]>> = const { RefCell::new(None) };
}

pub fn set_preferences(preferences: &ThemePreferences) {
    PALETTES.with_borrow_mut(|palettes| {
        *palettes = Some([
            preferences.resolve(false, false),
            preferences.resolve(false, true),
            preferences.resolve(true, false),
            preferences.resolve(true, true),
        ]);
    });
}

pub(super) fn resolved_theme(dark: bool, solid: bool) -> Theme {
    PALETTES.with_borrow(|palettes| {
        palettes.as_ref().map_or_else(
            || preset_theme(Preset::Default, dark, solid),
            |palettes| palettes[usize::from(dark) * 2 + usize::from(solid)],
        )
    })
}

pub fn composite(fg: Rgba, bg: Rgba) -> Rgba {
    Rgba {
        r: fg.r * fg.a + bg.r * (1. - fg.a),
        g: fg.g * fg.a + bg.g * (1. - fg.a),
        b: fg.b * fg.a + bg.b * (1. - fg.a),
        a: 1.,
    }
}

/// WCAG contrast ratio between two opaque colours.
fn contrast(a: Rgba, b: Rgba) -> f32 {
    let luminance = |c: Rgba| {
        let channel = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Flattens `color` onto `surfaces[0]` and moves it toward `toward` until it
/// reaches `minimum` contrast on every surface. VS Code themes often use
/// translucent or deliberately faint foregrounds (line numbers, comments)
/// that are unreadable as DBX labels.
fn readable(color: Rgba, toward: Rgba, surfaces: &[Rgba], minimum: f32) -> Rgba {
    let base = composite(color, surfaces[0]);
    (0..=20)
        .map(|step| mix(base, toward, step as f32 / 20.))
        .find(|c| surfaces.iter().all(|&s| contrast(*c, s) >= minimum))
        .unwrap_or(toward)
}

fn mix(a: Rgba, b: Rgba, amount: f32) -> Rgba {
    let mut overlay = b;
    overlay.a = amount;
    composite(overlay, a)
}

/// Text roles must stay legible whatever a custom or imported theme says;
/// saved themes from older imports are corrected here when they load.
pub(super) fn ensure_readable(p: &mut Theme, dark: bool) {
    let surfaces = [
        p.canvas,
        composite(p.panel, p.canvas),
        composite(p.panel_raised, p.canvas),
    ];
    ensure_readable_on(p, dark, &surfaces, &surfaces[..1], 3.);
}

/// Resolve ink against the composited material, including the least favorable
/// desktop brightness. Opaque-palette contrast is insufficient for glass.
pub(super) fn ensure_glass_readable(p: &mut Theme, dark: bool) {
    let backdrop = if dark { rgb(0xffffff) } else { rgb(0) };
    let window = composite(p.window, backdrop);
    let canvas = composite(p.canvas, window);
    let glass = composite(p.glass, window);
    // The clear window is never a text surface: content always sits on canvas.
    let surfaces = [
        canvas,
        glass,
        composite(p.glass_hover, glass),
        composite(p.glass_selected, glass),
        composite(p.panel, canvas),
        composite(p.panel_raised, canvas),
        composite(p.glass_raised, window),
    ];
    // Selected rows are tinted with the accent itself, so accent-coloured ink
    // can never clear them without being bleached toward black or white. Only
    // body text is held to them.
    let selected = [
        canvas,
        composite(p.grid_active, canvas),
        composite(p.grid_selected, canvas),
    ];
    // SQL belongs to the stable content layer, not the glass navigation layer.
    let syntax_surfaces = [canvas, composite(p.panel, canvas)];
    ensure_readable_on(p, dark, &surfaces, &syntax_surfaces, 4.5);
    let ink = if dark { rgb(0xffffff) } else { rgb(0) };
    p.text = readable(p.text, ink, &selected, 4.5);
    p.text_muted = readable(p.text_muted, p.text, &selected, 4.5);
}

fn ensure_readable_on(
    p: &mut Theme,
    dark: bool,
    surfaces: &[Rgba],
    syntax_surfaces: &[Rgba],
    syntax_minimum: f32,
) {
    let ink = if dark { rgb(0xffffff) } else { rgb(0) };
    p.text = readable(p.text, ink, surfaces, 4.5);
    for color in [
        &mut p.text_muted,
        &mut p.accent_text,
        &mut p.danger_text,
        &mut p.warning_text,
        &mut p.success_text,
    ] {
        *color = readable(*color, p.text, surfaces, 4.5);
    }
    // Control edges are non-text UI: 3:1, as for the presets. Themes that
    // mark inputs with a fill and a transparent border still get an edge.
    for color in [&mut p.input_border, &mut p.focus_ring] {
        *color = readable(*color, p.text, surfaces, 3.);
    }
    for color in [
        &mut p.sql_keyword,
        &mut p.sql_string,
        &mut p.sql_comment,
        &mut p.sql_number,
        &mut p.sql_parameter,
        &mut p.sql_identifier,
        &mut p.sql_type,
    ] {
        *color = readable(*color, p.text, syntax_surfaces, syntax_minimum);
    }
}

pub fn preset_theme(preset: Preset, dark: bool, solid: bool) -> Theme {
    let mut p = match (dark, solid) {
        (true, true) => *DARK_SOLID_THEME,
        (true, false) => *DARK_THEME,
        (false, true) => *LIGHT_SOLID_THEME,
        (false, false) => *LIGHT_THEME,
    };
    if preset == Preset::Default {
        return p;
    }
    if matches!(
        preset,
        Preset::Dracula | Preset::Nord | Preset::Solarized | Preset::Catppuccin
    ) {
        return familiar_palette(p, preset, dark);
    }
    let (tint, accent, bright) = match preset {
        Preset::Ocean => (0x146c94, 0x086b87, 0x78d4ea),
        Preset::Forest => (0x37765b, 0x24734d, 0x92d9ac),
        Preset::Plum => (0x765386, 0x8147b3, 0xd7a7f4),
        Preset::Sand => (0xb08a50, 0x976121, 0xe7c38c),
        Preset::Graphite => (0x777777, 0x536176, 0xb9c9df),
        _ => unreachable!(),
    };
    for color in [
        &mut p.canvas,
        &mut p.panel,
        &mut p.panel_raised,
        &mut p.rail,
        &mut p.grid_alternate,
        &mut p.border,
        &mut p.border_strong,
        &mut p.window,
        &mut p.glass,
        &mut p.glass_raised,
        &mut p.glass_hover,
        &mut p.glass_selected,
    ] {
        let alpha = color.a;
        *color = mix(*color, rgb(tint), if dark { 0.12 } else { 0.045 });
        color.a = alpha;
    }
    p.text_muted = mix(p.text_muted, p.text, 0.18);
    p.accent = rgb(accent);
    p.accent_hover = mix(p.accent, rgb(0), 0.12);
    p.accent_active = mix(p.accent, rgb(0), 0.24);
    p.accent_text = rgb(if dark { bright } else { accent });
    p.focus_ring = p.accent_text;
    p.accent_soft = mix(p.canvas, p.accent_text, if dark { 0.18 } else { 0.12 });
    p.grid_active = mix(p.canvas, p.accent_text, if dark { 0.22 } else { 0.28 });
    p.grid_selected = mix(p.canvas, p.accent_text, if dark { 0.17 } else { 0.15 });
    p.grid_hover = mix(p.canvas, p.accent_text, 0.07);
    p.selection = p.accent_text;
    p.selection.a = 0.3;
    p.sql_keyword = p.accent_text;
    p.sql_identifier = p.accent_text;
    p
}

/// Established palette colours adapted to DBX's semantic roles. See
/// docs/themes.md for the original palette references and light variants.
fn familiar_palette(mut p: Theme, preset: Preset, dark: bool) -> Theme {
    let (background, foreground, muted, accent, keyword, string, number, comment) =
        match (preset, dark) {
            (Preset::Dracula, true) => (
                0x282a36, 0xf8f8f2, 0xb3b8d5, 0xbd93f9, 0xff79c6, 0xf1fa8c, 0xbd93f9, 0x6272a4,
            ),
            (Preset::Dracula, false) => (
                0xfffbeb, 0x1f1f1f, 0x6c664b, 0x644ac9, 0xa3144d, 0x6a6b09, 0x644ac9, 0x6c664b,
            ),
            (Preset::Nord, true) => (
                0x2e3440, 0xeceff4, 0xb8c2d2, 0x88c0d0, 0x81a1c1, 0xa3be8c, 0xb48ead, 0x788ba8,
            ),
            (Preset::Nord, false) => (
                0xeceff4, 0x2e3440, 0x4c566a, 0x3b6580, 0x5e81ac, 0x4f6c39, 0x825b7e, 0x616e88,
            ),
            (Preset::Solarized, true) => (
                0x002b36, 0x93a1a1, 0x839496, 0x268bd2, 0x859900, 0x2aa198, 0xd33682, 0x657b83,
            ),
            (Preset::Solarized, false) => (
                0xfdf6e3, 0x586e75, 0x657b83, 0x268bd2, 0x859900, 0x2aa198, 0xd33682, 0x839496,
            ),
            (Preset::Catppuccin, true) => (
                0x1e1e2e, 0xcdd6f4, 0xa6adc8, 0xcba6f7, 0xcba6f7, 0xa6e3a1, 0xfab387, 0x7f849c,
            ),
            (Preset::Catppuccin, false) => (
                0xeff1f5, 0x4c4f69, 0x5c5f77, 0x8839ef, 0x8839ef, 0x40a02b, 0xfe640b, 0x7c7f93,
            ),
            _ => unreachable!(),
        };
    let edge = if dark { rgb(0xffffff) } else { rgb(0) };
    p.canvas = rgb(background);
    p.text = rgb(foreground);
    p.text_muted = rgb(muted);
    p.panel = mix(p.canvas, edge, 0.025);
    p.panel_raised = mix(p.canvas, edge, 0.05);
    p.window = p.panel;
    p.rail = p.panel;
    p.glass = p.panel;
    p.glass_raised = p.panel_raised;
    p.border = mix(p.canvas, edge, 0.16);
    p.hairline = p.border;
    p.border_strong = mix(p.canvas, edge, 0.3);
    p.input_border = mix(p.canvas, p.text, 0.6);
    p.accent = rgb(accent);
    p.accent_hover = mix(p.accent, edge, 0.12);
    p.accent_active = mix(p.accent, edge, 0.24);
    p.accent_text = readable(p.accent, p.text, &[p.canvas, p.panel, p.panel_raised], 4.5);
    p.accent_foreground = if contrast(rgb(0), p.accent) >= contrast(rgb(0xffffff), p.accent) {
        rgb(0)
    } else {
        rgb(0xffffff)
    };
    p.focus_ring = p.accent_text;
    p.accent_soft = mix(p.canvas, p.accent_text, 0.12);
    p.grid_alternate = mix(p.canvas, edge, 0.02);
    p.grid_active = mix(p.canvas, p.accent_text, if dark { 0.22 } else { 0.28 });
    p.grid_selected = mix(p.canvas, p.accent_text, 0.1);
    p.grid_hover = mix(p.canvas, p.accent_text, 0.05);
    p.glass_hover = p.grid_hover;
    p.glass_selected = p.grid_selected;
    p.selection = p.accent_text;
    p.selection.a = 0.3;
    p.sql_keyword = rgb(keyword);
    p.sql_string = rgb(string);
    p.sql_number = rgb(number);
    p.sql_comment = rgb(comment);
    p.sql_parameter = p.accent_text;
    p.sql_identifier = p.text;
    p.sql_type = p.accent_text;
    ensure_readable(&mut p, dark);
    // Grid washes must leave labels readable in both appearances.
    p.text = readable(
        p.text,
        edge,
        &[
            p.canvas,
            p.panel,
            p.panel_raised,
            p.grid_active,
            p.grid_selected,
        ],
        4.5,
    );
    p.text_muted = readable(
        p.text_muted,
        p.text,
        &[
            p.canvas,
            p.panel,
            p.panel_raised,
            p.grid_active,
            p.grid_selected,
        ],
        4.5,
    );
    p
}

pub struct ColorToken {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub get: fn(&Theme) -> Rgba,
    pub set: fn(&mut Theme, Rgba),
}

macro_rules! tokens {
    ($(($field:ident, $label:literal, $group:literal)),* $(,)?) => {
        pub const TOKENS: &[ColorToken] = &[$(ColorToken {
            key: stringify!($field), label: $label, group: $group,
            get: |p| p.$field, set: |p, color| p.$field = color,
        }),*];
    };
}

tokens! {
    (canvas, "Canvas", "Interface"),
    (panel, "Panel", "Interface"),
    (panel_raised, "Panel raised", "Interface"),
    (border, "Border", "Interface"),
    (border_strong, "Border strong", "Interface"),
    (input_border, "Input border", "Interface"),
    (text, "Primary text", "Interface"),
    (text_muted, "Secondary text", "Interface"),
    (accent, "Accent", "Interface"),
    (accent_hover, "Accent hover", "Interface"),
    (accent_active, "Accent active", "Interface"),
    (accent_text, "Accent text", "Interface"),
    (accent_foreground, "Accent foreground", "Interface"),
    (accent_soft, "Accent soft", "Interface"),
    (success, "Success", "Interface"),
    (success_text, "Success text", "Interface"),
    (danger, "Danger", "Interface"),
    (danger_text, "Danger text", "Interface"),
    (warning, "Warning", "Interface"),
    (warning_text, "Warning text", "Interface"),
    (grid_alternate, "Alternate row", "Data grid"),
    (grid_active, "Active row", "Data grid"),
    (grid_selected, "Selected rows", "Data grid"),
    (grid_hover, "Hovered row", "Data grid"),
    (rail, "Rail", "Interface"),
    (focus_ring, "Focus ring", "Interface"),
    (overlay, "Overlay", "Materials"),
    (selection, "Text selection", "Interface"),
    (sql_keyword, "Keyword", "SQL syntax"),
    (sql_string, "String", "SQL syntax"),
    (sql_comment, "Comment", "SQL syntax"),
    (sql_number, "Number", "SQL syntax"),
    (sql_parameter, "Parameter", "SQL syntax"),
    (sql_identifier, "Identifier", "SQL syntax"),
    (sql_type, "Type", "SQL syntax"),
    (window, "Window", "Materials"),
    (glass, "Glass", "Materials"),
    (glass_raised, "Glass raised", "Materials"),
    (glass_hover, "Glass hover", "Materials"),
    (glass_selected, "Glass selected", "Materials"),
    (rim, "Rim", "Materials"),
    (hairline, "Hairline", "Materials"),
    (shadow, "Shadow", "Materials"),
}

pub fn parse_color(value: &str) -> Result<Rgba, String> {
    let hex = value
        .trim()
        .strip_prefix('#')
        .ok_or("Use a hex colour such as #2563eb or #2563eb80.")?;
    if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("Invalid hex colour: {value}"));
    }
    let expanded = match hex.len() {
        3 | 4 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 | 8 => hex.to_string(),
        _ => return Err("Colours must have 3, 4, 6, or 8 hex digits.".into()),
    };
    let value = u32::from_str_radix(&expanded, 16).map_err(|e| e.to_string())?;
    Ok(if expanded.len() == 6 {
        rgb(value)
    } else {
        rgba(value)
    })
}

pub fn color_hex(color: Rgba) -> String {
    let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    let mut value = format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    );
    if channel(color.a) != 255 {
        value.push_str(&format!("{:02x}", channel(color.a)));
    }
    value
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    dbx_theme: u32,
    #[serde(flatten)]
    theme: CustomTheme,
}

pub fn export_theme(preferences: &ThemePreferences) -> Result<String, String> {
    let name = preferences
        .active()
        .map_or_else(|| preferences.preset.label().into(), |t| t.name.clone());
    serde_json::to_string_pretty(&ThemeFile {
        dbx_theme: 1,
        theme: preferences.snapshot(name),
    })
    .map_err(|e| e.to_string())
}

pub struct ImportedTheme {
    pub theme: CustomTheme,
    pub appearance: Option<Appearance>,
}

/// Reads JSONC without interpreting strings as comments or evaluating any code.
fn jsonc(source: &str) -> Result<String, String> {
    let mut bytes = source.trim_start_matches('\u{feff}').as_bytes().to_vec();
    let mut i = 0;
    let mut string = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if string => {
                i += 2;
                continue;
            }
            b'"' => string = !string,
            b'/' if !string && bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    bytes[i] = b' ';
                    i += 1;
                }
                continue;
            }
            b'/' if !string && bytes.get(i + 1) == Some(&b'*') => {
                bytes[i] = b' ';
                bytes[i + 1] = b' ';
                i += 2;
                loop {
                    if i + 1 >= bytes.len() {
                        return Err("Unterminated JSON comment.".into());
                    }
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        bytes[i] = b' ';
                        bytes[i + 1] = b' ';
                        i += 2;
                        break;
                    }
                    if bytes[i] != b'\n' && bytes[i] != b'\r' {
                        bytes[i] = b' ';
                    }
                    i += 1;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    i = 0;
    string = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if string => {
                i += 2;
                continue;
            }
            b'"' => string = !string,
            b',' if !string
                && bytes[i + 1..]
                    .iter()
                    .find(|c| !c.is_ascii_whitespace())
                    .is_some_and(|c| matches!(c, b'}' | b']')) =>
            {
                bytes[i] = b' ';
            }
            _ => {}
        }
        i += 1;
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

type SyntaxRole = (
    fn(&mut Theme, Rgba),
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
);

/// DBX syntax role ← TextMate scopes: exact generic scopes in priority order,
/// then prefixes for themes that only style specific scopes. Themes are full
/// of language-specific overrides (`string.regexp`, `keyword.operator.php`),
/// so the most generic matching rule represents the role.
const SYNTAX_ROLES: [SyntaxRole; 7] = [
    (
        |p, c| p.sql_keyword = c,
        &[
            "keyword.other.DML.sql",
            "keyword",
            "keyword.control",
            "storage",
            "keyword.other",
        ],
        &["keyword.", "storage."],
        &["keyword.operator"],
    ),
    (
        |p, c| p.sql_string = c,
        &["string", "string.quoted", "string.quoted.single"],
        &["string."],
        &["string.regexp", "string.other"],
    ),
    (|p, c| p.sql_comment = c, &["comment"], &["comment."], &[]),
    (
        |p, c| p.sql_number = c,
        &["constant.numeric"],
        &["constant.numeric."],
        &[],
    ),
    (
        |p, c| p.sql_parameter = c,
        &["variable.parameter"],
        &["variable.parameter."],
        &[],
    ),
    (
        |p, c| p.sql_type = c,
        &["support.type", "entity.name.type", "storage.type"],
        &["support.type.", "entity.name.type."],
        &["support.type.property-name"],
    ),
    (
        |p, c| p.sql_identifier = c,
        &["variable", "variable.other", "entity.name"],
        &["variable.other.", "entity.name."],
        &[
            "entity.name.function",
            "entity.name.type",
            "entity.name.tag",
        ],
    ),
];

/// Later rules override earlier ones, as in VS Code.
fn syntax_color(
    scoped: &[(&str, Rgba)],
    exact: &[&str],
    prefixes: &[&str],
    excluded: &[&str],
) -> Option<Rgba> {
    let last = |wanted: &str| {
        scoped
            .iter()
            .rev()
            .find(|(scope, _)| *scope == wanted)
            .map(|(_, c)| *c)
    };
    exact.iter().find_map(|scope| last(scope)).or_else(|| {
        scoped
            .iter()
            .enumerate()
            .filter(|(_, (scope, _))| {
                prefixes.iter().any(|p| scope.starts_with(p))
                    && !excluded.iter().any(|e| scope.starts_with(e))
            })
            .min_by_key(|(index, (scope, _))| (scope.matches('.').count(), usize::MAX - index))
            .map(|(_, (_, c))| *c)
    })
}

pub fn import_theme(source: &str, fallback_name: &str) -> Result<ImportedTheme, String> {
    import_themes(source, fallback_name)?
        .into_iter()
        .next()
        .ok_or("The theme family is empty.".into())
}

/// Validate the whole file before changing preferences. Zed families can
/// contain several independent variants; retain every one in Your themes.
pub fn import_themes(source: &str, fallback_name: &str) -> Result<Vec<ImportedTheme>, String> {
    if source.len() as u64 > MAX_THEME_BYTES {
        return Err("Theme files must be smaller than 2 MB.".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(&jsonc(source)?).map_err(|e| format!("Invalid theme JSON: {e}"))?;
    if value.get("dbx_theme").is_some() {
        let file: ThemeFile =
            serde_json::from_value(value).map_err(|e| format!("Invalid DBX theme: {e}"))?;
        if file.dbx_theme != 1 {
            return Err("Unsupported DBX theme version.".into());
        }
        for colors in [&file.theme.light, &file.theme.dark] {
            for (key, value) in colors {
                if !TOKENS.iter().any(|t| t.key == key) {
                    return Err(format!("Unknown theme colour: {key}"));
                }
                parse_color(value).map_err(|e| format!("{key}: {e}"))?;
            }
        }
        if file.theme.light.is_empty() && file.theme.dark.is_empty() {
            return Err("The theme has no colours.".into());
        }
        return Ok(vec![ImportedTheme {
            theme: file.theme,
            appearance: None,
        }]);
    }
    if let Some(themes) = value.get("themes") {
        let themes = themes
            .as_array()
            .filter(|themes| !themes.is_empty())
            .ok_or("A Zed theme family needs a nonempty ‘themes’ array.")?;
        return themes
            .iter()
            .map(|theme| import_zed(theme, fallback_name))
            .collect();
    }
    if value.get("style").is_some() {
        return import_zed(&value, fallback_name).map(|theme| vec![theme]);
    }
    import_vscode(&value, fallback_name).map(|theme| vec![theme])
}

fn import_zed(value: &serde_json::Value, fallback_name: &str) -> Result<ImportedTheme, String> {
    use serde_json::{Value, json};
    let style = value
        .get("style")
        .and_then(Value::as_object)
        .ok_or("A Zed theme needs a ‘style’ object.")?;
    let appearance = value
        .get("appearance")
        .and_then(Value::as_str)
        .ok_or("A Zed theme needs a light or dark ‘appearance’.")?;
    if !matches!(appearance, "light" | "dark") {
        return Err("Zed theme appearance must be light or dark.".into());
    }
    let mut colors = serde_json::Map::new();
    // Reuse the material derivation, alpha handling and contrast correction
    // of the VS Code adapter after mapping Zed's semantic field names.
    for (from, to) in [
        ("background", "editor.background"),
        ("editor.background", "editor.background"),
        ("text", "editor.foreground"),
        ("editor.foreground", "editor.foreground"),
        ("text.muted", "descriptionForeground"),
        ("surface.background", "panel.background"),
        ("panel.background", "sideBar.background"),
        ("toolbar.background", "editorGroupHeader.tabsBackground"),
        ("title_bar.background", "titleBar.activeBackground"),
        ("elevated_surface.background", "editorWidget.background"),
        ("border", "panel.border"),
        ("border.variant", "sideBar.border"),
        ("border.focused", "focusBorder"),
        ("text.accent", "textLink.foreground"),
        ("element.background", "button.background"),
        ("element.hover", "button.hoverBackground"),
        ("element.selected", "list.activeSelectionBackground"),
        ("ghost_element.hover", "list.hoverBackground"),
        ("ghost_element.selected", "list.inactiveSelectionBackground"),
        ("editor.selection_background", "editor.selectionBackground"),
        ("error", "editorError.foreground"),
        ("warning", "editorWarning.foreground"),
        ("success", "testing.iconPassed"),
    ] {
        if let Some(color) = style.get(from).filter(|value| !value.is_null()) {
            colors.insert(to.into(), color.clone());
        }
    }
    if let Some(cursor) = style
        .get("players")
        .and_then(Value::as_array)
        .and_then(|players| players.first())
        .and_then(|player| player.get("cursor"))
    {
        colors
            .entry("editorCursor.foreground")
            .or_insert_with(|| cursor.clone());
    }
    let mut rules = Vec::new();
    if let Some(syntax) = style.get("syntax") {
        let syntax = syntax.as_object().ok_or("Zed syntax must be an object.")?;
        for (capture, scope) in [
            ("keyword", "keyword"),
            ("string", "string"),
            ("comment", "comment"),
            ("comment.doc", "comment.block.documentation"),
            ("number", "constant.numeric"),
            ("constant", "constant"),
            ("type", "storage.type"),
            ("variable", "variable"),
            ("variable.special", "variable.parameter"),
            ("property", "entity.name"),
        ] {
            if let Some(color) = syntax
                .get(capture)
                .and_then(|style| style.get("color"))
                .filter(|color| !color.is_null())
            {
                rules.push(json!({"scope": scope, "settings": {"foreground": color}}));
            }
        }
    }
    import_vscode(
        &json!({
            "name": value.get("name").and_then(Value::as_str).unwrap_or(fallback_name),
            "type": appearance,
            "colors": colors,
            "tokenColors": rules,
        }),
        fallback_name,
    )
}

fn import_vscode(value: &serde_json::Value, fallback_name: &str) -> Result<ImportedTheme, String> {
    use serde_json::Value;
    if value.get("include").is_some() {
        return Err("This theme includes another file. In VS Code, run ‘Developer: Generate Color Theme From Current Settings’ and import the generated JSON.".into());
    }
    let colors = value.get("colors").and_then(Value::as_object).ok_or(
        "Choose a VS Code colour theme JSON with a ‘colors’ object, or a DBX theme export.",
    )?;
    let mut parsed = BTreeMap::new();
    for (key, color) in colors {
        if color.is_null() {
            continue;
        }
        let color = color
            .as_str()
            .ok_or_else(|| format!("{key} must be a hex colour."))?;
        parsed.insert(
            key.as_str(),
            parse_color(color).map_err(|e| format!("{key}: {e}"))?,
        );
    }
    let dark = match value.get("type").and_then(Value::as_str) {
        Some("light" | "hcLight") => false,
        Some("dark" | "hc" | "hcDark") => true,
        _ => parsed
            .get("editor.background")
            .is_none_or(|c| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b < 0.5),
    };
    let mut p = preset_theme(Preset::Default, dark, false);
    // Derive every material from the imported editor surface first, then apply specific UI colours.
    if let Some(&background) = parsed.get("editor.background") {
        p.canvas = composite(background, p.canvas);
        let edge = if dark { rgb(0xffffff) } else { rgb(0) };
        p.panel = mix(p.canvas, edge, 0.025);
        p.panel_raised = mix(p.canvas, edge, 0.055);
        p.rail = p.panel;
        p.window = p.panel;
        p.glass = p.panel;
        p.glass_raised = p.panel_raised;
        p.grid_alternate = mix(p.canvas, edge, 0.02);
        p.border = mix(p.canvas, edge, 0.15);
        p.hairline = p.border;
        p.border_strong = mix(p.canvas, edge, 0.3);
        p.glass_hover = mix(p.canvas, edge, 0.07);
        p.glass_selected = mix(p.canvas, edge, 0.12);
        p.grid_hover = p.glass_hover;
    }
    if let Some(&fg) = parsed
        .get("editor.foreground")
        .or_else(|| parsed.get("foreground"))
    {
        p.text = fg;
        p.text_muted = mix(p.canvas, fg, 0.65);
        p.input_border = mix(p.canvas, fg, 0.6);
    }
    macro_rules! map {
        ($field:ident, $($key:literal),+) => {
            if let Some(color) = [$($key),+].iter().find_map(|key| parsed.get(key)) { p.$field = *color; }
        };
    }
    map!(text_muted, "descriptionForeground");
    map!(panel, "panel.background", "sideBar.background");
    map!(rail, "sideBar.background", "activityBar.background");
    map!(window, "titleBar.activeBackground", "sideBar.background");
    map!(glass, "sideBar.background");
    map!(glass_raised, "editorWidget.background", "menu.background");
    map!(
        panel_raised,
        "tab.inactiveBackground",
        "editorGroupHeader.tabsBackground"
    );
    map!(border, "panel.border", "editorGroup.border");
    map!(hairline, "sideBar.border", "panel.border");
    map!(input_border, "input.border", "contrastBorder");
    map!(accent, "button.background", "focusBorder");
    p.accent_hover = mix(p.accent, rgb(0), 0.12);
    p.accent_active = mix(p.accent, rgb(0), 0.24);
    map!(accent_hover, "button.hoverBackground");
    map!(accent_foreground, "button.foreground");
    p.accent_text = if dark {
        mix(p.accent, rgb(0xffffff), 0.5)
    } else {
        p.accent
    };
    map!(accent_text, "textLink.foreground");
    p.focus_ring = p.accent_text;
    map!(focus_ring, "focusBorder", "editorCursor.foreground");
    p.accent_soft = mix(p.canvas, p.accent_text, 0.15);
    map!(selection, "editor.selectionBackground");
    p.grid_active = mix(p.canvas, p.accent_text, 0.28);
    p.grid_selected = mix(p.canvas, p.accent_text, 0.15);
    // DBX grid values retain semantic text colours: a translucent wash works with those colours.
    if let Some(&color) = parsed.get("list.activeSelectionBackground") {
        p.grid_active = mix(p.canvas, composite(color, p.canvas), 0.55);
        p.grid_selected = mix(p.canvas, composite(color, p.canvas), 0.35);
    }
    map!(grid_hover, "list.hoverBackground");
    map!(
        glass_hover,
        "toolbar.hoverBackground",
        "list.hoverBackground"
    );
    map!(glass_selected, "list.inactiveSelectionBackground");
    map!(danger, "editorError.foreground", "errorForeground");
    p.danger_text = p.danger;
    map!(warning, "editorWarning.foreground");
    p.warning_text = p.warning;
    map!(
        success,
        "testing.iconPassed",
        "gitDecoration.addedResourceForeground"
    );
    p.success_text = p.success;
    if let Some(rules) = value.get("tokenColors") {
        let rules = rules.as_array().ok_or("‘tokenColors’ must be an array; external token files are unsupported. Export the current theme from VS Code first.")?;
        let mut scoped = Vec::new();
        for rule in rules {
            let Some(color) = rule.pointer("/settings/foreground") else {
                continue;
            };
            let color = parse_color(
                color
                    .as_str()
                    .ok_or("Token foreground must be a hex colour.")?,
            )?;
            let scopes: Vec<&str> = match rule.get("scope") {
                Some(Value::String(s)) => s.split(',').map(str::trim).collect(),
                Some(Value::Array(scopes)) => scopes.iter().filter_map(Value::as_str).collect(),
                _ => Vec::new(),
            };
            // Descendant selectors ("comment.block keyword") describe nested
            // tokens, not the role itself.
            scoped.extend(
                scopes
                    .into_iter()
                    .filter(|scope| !scope.contains(char::is_whitespace))
                    .map(|scope| (scope, color)),
            );
        }
        for (field, exact, prefixes, excluded) in SYNTAX_ROLES {
            if let Some(color) = syntax_color(&scoped, exact, prefixes, excluded) {
                field(&mut p, color);
            }
        }
    }
    if parsed.is_empty() {
        return Err("The theme has no UI colours to import.".into());
    }
    ensure_readable(&mut p, dark);
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback_name)
        .to_string();
    let colors = TOKENS
        .iter()
        .map(|token| (token.key.into(), color_hex((token.get)(&p))))
        .collect();
    let theme = CustomTheme {
        name,
        light: if dark { Colors::new() } else { colors },
        dark: Colors::new(),
    };
    let theme = if dark {
        CustomTheme {
            dark: TOKENS
                .iter()
                .map(|t| (t.key.into(), color_hex((t.get)(&p))))
                .collect(),
            ..theme
        }
    } else {
        theme
    };
    Ok(ImportedTheme {
        theme,
        appearance: Some(if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        }),
    })
}

#[cfg(test)]
mod glass_contrast_tests {
    use super::*;

    #[test]
    fn secondary_and_syntax_ink_survives_changing_glass_backdrops() {
        for dark in [false, true] {
            let palette = super::super::native_material_palette(
                preset_theme(Preset::Default, dark, false),
                dark,
            );
            for backdrop in [rgb(0), rgb(0x808080), rgb(0xffffff)] {
                let window = composite(palette.window, backdrop);
                let canvas = composite(palette.canvas, window);
                let glass = composite(palette.glass, window);
                for surface in [canvas, glass, composite(palette.glass_selected, glass)] {
                    for ink in [
                        palette.text,
                        palette.text_muted,
                        palette.accent_text,
                        palette.danger_text,
                        palette.success_text,
                        palette.warning_text,
                    ] {
                        assert!(contrast(ink, surface) >= 4.5, "dark={dark}");
                    }
                    for edge in [palette.input_border, palette.focus_ring] {
                        assert!(contrast(edge, surface) >= 3., "dark={dark}");
                    }
                }
                for ink in [palette.text, palette.text_muted] {
                    assert!(
                        contrast(ink, composite(palette.grid_active, canvas)) >= 4.5,
                        "dark={dark}"
                    );
                }
                for surface in [canvas, palette.panel] {
                    for ink in [
                        palette.sql_comment,
                        palette.sql_keyword,
                        palette.sql_string,
                        palette.sql_number,
                    ] {
                        assert!(contrast(ink, surface) >= 4.5, "dark={dark}");
                    }
                }
            }
        }
    }

    #[test]
    fn native_content_is_translucent_but_dense_and_data_stays_opaque() {
        for preset in Preset::ALL {
            for dark in [false, true] {
                let mut source = preset_theme(preset, dark, false);
                // Imported themes may supply translucent content tokens too.
                source.canvas.a = 0.15;
                source.panel.a = 0.2;
                source.grid_alternate.a = 0.1;
                let palette = super::super::native_material_palette(source, dark);
                assert_eq!(palette.canvas.a, super::super::NATIVE_CANVAS_ALPHA);
                assert_eq!(palette.panel.a, super::super::NATIVE_PANEL_ALPHA);
                for surface in [
                    palette.panel_raised,
                    palette.grid_alternate,
                    palette.grid_active,
                    palette.grid_hover,
                    palette.grid_selected,
                    palette.glass_raised,
                ] {
                    assert_eq!(surface.a, 1., "{preset:?}, dark={dark}");
                    assert_eq!(
                        composite(surface, rgb(0)),
                        composite(surface, rgb(0xffffff))
                    );
                }
            }
        }
    }
}
