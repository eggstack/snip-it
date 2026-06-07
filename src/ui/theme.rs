//! Theme system: Halloy-compatible TOML themes with a 10-color projection.
//!
//! Themes live as `*.toml` files in `~/.config/snp/themes/`. The bundled
//! release ships with ~50 themes (see `scripts/build_themes.py`) that are
//! extracted to the user's themes directory on first launch. The active
//! theme is recorded in `~/.config/snp/themes.toml` and loaded into a
//! process-global `RwLock<Theme>` so the TUI can re-style itself on demand.
//!
//! The original two built-in themes (`dark`, `bright`) remain reachable
//! via the `SNP_THEME` environment variable for backward compatibility.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

use ratatui::style::{Color, Style};
use serde::{Deserialize, Serialize};

use crate::error::{SnipError, SnipResult};
use crate::utils::config::get_config_dir;

/// The 10 colors that drive the entire TUI chrome and syntax highlighter.
///
/// `Clone + Copy` so it can be cheaply snapshotted into the draw closure.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub background: Color,
    pub text: Color,
    pub border: Color,
    pub selected_bg: Color,
    pub muted: Color,
    pub string_color: Color,
    pub escape_color: Color,
}

/// Built-in dark theme, used as a last-resort fallback and for the
/// `SNP_THEME=dark` / `COLORFGBG=…;…` legacy paths.
const DARK_THEME: Theme = Theme {
    primary: Color::Blue,
    secondary: Color::Cyan,
    accent: Color::Yellow,
    background: Color::Black,
    text: Color::White,
    border: Color::Cyan,
    selected_bg: Color::Blue,
    muted: Color::Gray,
    string_color: Color::Green,
    escape_color: Color::Magenta,
};

/// Built-in bright theme, used for `SNP_THEME=bright|light`.
const BRIGHT_THEME: Theme = Theme {
    primary: Color::Blue,
    secondary: Color::Blue,
    accent: Color::Magenta,
    background: Color::White,
    text: Color::Black,
    border: Color::Blue,
    selected_bg: Color::LightBlue,
    muted: Color::Gray,
    string_color: Color::DarkGray,
    escape_color: Color::DarkGray,
};

// ============================================================================
// Halloy TOML schema (private)
// ============================================================================
//
// Mirrors the post-2024.11 Halloy `Styles` struct. Every field is
// `#[serde(default)]` so unknown fields and missing sections parse
// gracefully — the on-disk themes in the wild are inconsistent.
//
// We deserialize colors as `String` (or `Option<String>`) and convert
// to `ratatui::style::Color` after parsing, because `Color` does not
// implement `serde::Deserialize`.

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyTheme {
    general: HalloyGeneral,
    text: HalloyText,
    buffer: HalloyBuffer,
    buttons: HalloyButtons,
    formatting: HalloyFormatting,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyGeneral {
    background: String,
    border: String,
    horizontal_rule: String,
    horizontal_rule_text: Option<String>,
    scrollbar: Option<String>,
    unread_indicator: String,
    highlight_indicator: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyText {
    primary: HalloyTextStyle,
    secondary: HalloyTextStyle,
    tertiary: HalloyTextStyle,
    success: HalloyTextStyle,
    error: HalloyTextStyle,
    warning: HalloyOptionalTextStyle,
    info: HalloyOptionalTextStyle,
    debug: HalloyOptionalTextStyle,
    trace: HalloyOptionalTextStyle,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyBuffer {
    action: HalloyTextStyle,
    background: String,
    background_text_input: String,
    background_title_bar: String,
    border: String,
    border_selected: String,
    code: HalloyTextStyle,
    highlight: String,
    nickname: HalloyTextStyle,
    selection: String,
    server_messages: HalloyServerMessages,
    timestamp: HalloyTextStyle,
    topic: HalloyTextStyle,
    url: HalloyTextStyle,
    nickname_offline: HalloyOptionalTextStyle,
    backlog_rule: Option<String>,
    backlog_rule_text: Option<String>,
    date_rule: Option<String>,
    date_rule_text: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyServerMessages {
    default: HalloyTextStyle,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyButtons {
    primary: HalloyButton,
    secondary: HalloyButton,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyButton {
    background: String,
    background_hover: String,
    background_selected: String,
    background_selected_hover: String,
    border_active: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct HalloyFormatting {
    white: Option<String>,
    black: Option<String>,
    blue: Option<String>,
    green: Option<String>,
    red: Option<String>,
    brown: Option<String>,
    magenta: Option<String>,
    orange: Option<String>,
    yellow: Option<String>,
    lightgreen: Option<String>,
    cyan: Option<String>,
    lightcyan: Option<String>,
    lightblue: Option<String>,
    pink: Option<String>,
    grey: Option<String>,
    lightgrey: Option<String>,
}

/// Halloy `TextStyle`: a color plus an optional font_style modifier.
/// We accept both `"#RRGGBB"` (basic) and `{ color = "#RRGGBB", font_style = "bold" }` (extended).
#[derive(Debug, Clone, Default)]
struct HalloyTextStyle {
    color: String,
    #[allow(dead_code)]
    font_style: Option<FontStyle>,
}

impl<'de> Deserialize<'de> for HalloyTextStyle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Basic(String),
            Extended {
                color: String,
                font_style: Option<FontStyle>,
            },
        }
        let repr = Repr::deserialize(deserializer)?;
        let (color, font_style) = match repr {
            Repr::Basic(c) => (c, None),
            Repr::Extended { color, font_style } => (color, font_style),
        };
        Ok(HalloyTextStyle { color, font_style })
    }
}

impl Serialize for HalloyTextStyle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.font_style.is_some() {
            use serde::ser::SerializeStruct;
            let mut s = serializer.serialize_struct("TextStyle", 2)?;
            s.serialize_field("color", &self.color)?;
            s.serialize_field("font_style", &self.font_style)?;
            s.end()
        } else {
            self.color.serialize(serializer)
        }
    }
}

/// Same as `HalloyTextStyle` but the color is `Option<String>` and the
/// missing form is `null` rather than a default.
#[derive(Debug, Clone, Default)]
struct HalloyOptionalTextStyle {
    color: Option<String>,
    #[allow(dead_code)]
    font_style: Option<FontStyle>,
}

impl<'de> Deserialize<'de> for HalloyOptionalTextStyle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Basic(Option<String>),
            Extended {
                color: Option<String>,
                font_style: Option<FontStyle>,
            },
        }
        let repr = Repr::deserialize(deserializer)?;
        let (color, font_style) = match repr {
            Repr::Basic(c) => (c, None),
            Repr::Extended { color, font_style } => (color, font_style),
        };
        Ok(HalloyOptionalTextStyle { color, font_style })
    }
}

impl Serialize for HalloyOptionalTextStyle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.font_style.is_some() {
            use serde::ser::SerializeStruct;
            let mut s = serializer.serialize_struct("OptionalTextStyle", 2)?;
            s.serialize_field("color", &self.color)?;
            s.serialize_field("font_style", &self.font_style)?;
            s.end()
        } else {
            self.color.serialize(serializer)
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
enum FontStyle {
    #[default]
    Normal,
    Bold,
    Italic,
    #[serde(alias = "bold-italic")]
    ItalicBold,
}

/// Parses a Halloy hex color string.
///
/// Accepts:
/// - `"#RRGGBB"` (most common)
/// - `"#RRGGBBAA"` (alpha is discarded; ratatui has no alpha)
///
/// Returns `None` for any other format (including the empty string,
/// which we treat as "missing" so that missing fields fall through to
/// the chain).
fn hex_to_color(s: &str) -> Option<Color> {
    let hex = s.strip_prefix('#')?;
    let bytes = hex.as_bytes();
    let (r, g, b) = match bytes.len() {
        6 => (
            u8::from_str_radix(std::str::from_utf8(&bytes[0..2]).ok()?, 16).ok()?,
            u8::from_str_radix(std::str::from_utf8(&bytes[2..4]).ok()?, 16).ok()?,
            u8::from_str_radix(std::str::from_utf8(&bytes[4..6]).ok()?, 16).ok()?,
        ),
        8 => (
            // alpha discarded
            u8::from_str_radix(std::str::from_utf8(&bytes[0..2]).ok()?, 16).ok()?,
            u8::from_str_radix(std::str::from_utf8(&bytes[2..4]).ok()?, 16).ok()?,
            u8::from_str_radix(std::str::from_utf8(&bytes[4..6]).ok()?, 16).ok()?,
        ),
        _ => return None,
    };
    Some(Color::Rgb(r, g, b))
}

/// Convenience: convert an `Option<String>` to a usable `Color`, or
/// `Color::Reset` if the field is missing or malformed (so the
/// fallback chain in `into_snp_theme` can skip it).
fn opt_str_to_color(s: &Option<String>) -> Color {
    s.as_deref().and_then(hex_to_color).unwrap_or(Color::Reset)
}

impl HalloyTheme {
    /// Project the rich Halloy schema onto snp-it's 10-color `Theme`.
    ///
    /// See `plans/halloy-themes.md` §5c for the documented mapping table.
    /// Each field walks a short fallback chain so that a partial theme
    /// (e.g. one with no `[text]` section) still produces a coherent
    /// palette. `Color::Reset` is treated as "missing" and walks
    /// the chain.
    fn into_snp_theme(self) -> Theme {
        // Pre-convert the frequently-used fields once. Each `_c` is a
        // concrete `Color` (or `Color::Reset` if the field was
        // missing or malformed).
        let text_primary = hex_to_color(&self.text.primary.color).unwrap_or(Color::Reset);
        let text_secondary = hex_to_color(&self.text.secondary.color).unwrap_or(Color::Reset);
        let text_tertiary = hex_to_color(&self.text.tertiary.color).unwrap_or(Color::Reset);
        let text_success = hex_to_color(&self.text.success.color).unwrap_or(Color::Reset);
        let text_error = hex_to_color(&self.text.error.color).unwrap_or(Color::Reset);
        let general_bg = hex_to_color(&self.general.background).unwrap_or(Color::Reset);
        let general_border = hex_to_color(&self.general.border).unwrap_or(Color::Reset);
        let buffer_bg = hex_to_color(&self.buffer.background).unwrap_or(Color::Reset);
        let buffer_highlight = hex_to_color(&self.buffer.highlight).unwrap_or(Color::Reset);
        let buffer_selection = hex_to_color(&self.buffer.selection).unwrap_or(Color::Reset);
        let buffer_timestamp = hex_to_color(&self.buffer.timestamp.color).unwrap_or(Color::Reset);
        let formatting_green = opt_str_to_color(&self.formatting.green);
        let formatting_magenta = opt_str_to_color(&self.formatting.magenta);

        Theme {
            primary: first_some_color(&[text_primary, text_tertiary]),
            secondary: first_some_color(&[text_tertiary, text_secondary]),
            accent: first_some_color(&[text_tertiary, text_success]),
            background: first_some_color(&[general_bg, buffer_bg]),
            text: first_some_color(&[text_primary]),
            border: first_some_color(&[general_border, text_tertiary]),
            selected_bg: first_some_color(&[buffer_highlight, buffer_selection]),
            muted: first_some_color(&[buffer_timestamp, text_secondary]),
            string_color: first_some_color(&[formatting_green, text_success]),
            escape_color: first_some_color(&[formatting_magenta, text_error]),
        }
    }
}

/// Returns the first non-transparent color in the slice, or `Color::White`
/// as the last-resort safety net. Always returns a usable color.
fn first_some_color(candidates: &[Color]) -> Color {
    for &c in candidates {
        if c != Color::Reset {
            return c;
        }
    }
    Color::White
}

// ============================================================================
// ThemeManager
// ============================================================================

/// Configuration file for the theme system. Records the user's
/// currently-active theme. Lives at `~/.config/snp/themes.toml`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ThemesConfig {
    /// Basename of the active theme file (no `.toml` extension).
    #[serde(default)]
    pub active: Option<String>,
}

/// One entry in the theme directory listing.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ThemeInfo {
    /// Basename without `.toml`. Used as the persistent identifier
    /// (in `themes.toml`) and as the lookup key in `load_theme`.
    pub name: String,
    /// Absolute path to the on-disk `.toml` file.
    pub path: PathBuf,
    /// True if this theme was extracted from the bundled binary
    /// (vs. authored by the user).
    pub is_bundled: bool,
}

/// Manages the `~/.config/snp/themes/` directory and `themes.toml` index.
///
/// One instance per process is typical. Methods are non-thread-safe
/// (mutating methods take `&mut self`); the active theme is published
/// to a separate process-global `RwLock<Theme>` for cheap reads from
/// the TUI draw loop.
#[allow(dead_code)]
pub struct ThemeManager {
    #[allow(dead_code)]
    config_dir: PathBuf,
    #[allow(dead_code)]
    themes_dir: PathBuf,
    #[allow(dead_code)]
    config_path: PathBuf,
    #[allow(dead_code)]
    config: ThemesConfig,
}

#[allow(dead_code)]
impl ThemeManager {
    /// Create a new `ThemeManager` rooted at the user's config dir.
    ///
    /// Reads `themes.toml` if present; falls back to defaults on a
    /// missing or unparseable file. Does not touch `themes/` — call
    /// `init_themes_dir` for that.
    pub fn new() -> SnipResult<Self> {
        let config_dir = get_config_dir();
        let themes_dir = config_dir.join("themes");
        let config_path = config_dir.join("themes.toml");

        let config = if config_path.exists() {
            match fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str::<ThemesConfig>(&content) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            config = %config_path.display(),
                            error = %e,
                            "Failed to parse themes.toml; using defaults"
                        );
                        ThemesConfig::default()
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        config = %config_path.display(),
                        error = %e,
                        "Failed to read themes.toml; using defaults"
                    );
                    ThemesConfig::default()
                }
            }
        } else {
            ThemesConfig::default()
        };

        Ok(Self {
            config_dir,
            themes_dir,
            config_path,
            config,
        })
    }

    /// Create the `themes/` directory and seed it with bundled themes.
    ///
    /// Idempotent: never overwrites an existing file, so user edits
    /// to seeded themes are preserved across upgrades.
    pub fn init_themes_dir(&mut self) -> SnipResult<()> {
        fs::create_dir_all(&self.themes_dir).map_err(|e| {
            SnipError::io_error("create themes directory", self.themes_dir.clone(), e)
        })?;

        // The default theme is embedded as a plain string — write it
        // first so the very first launch has Cyber Red on disk.
        write_theme_if_missing(
            &self.themes_dir,
            bundled_default_name(),
            super::_generated_bundled_themes::DEFAULT_BUNDLED,
        )?;

        for (name, toml_text) in super::_generated_bundled_themes::bundled_themes_decoded() {
            write_theme_if_missing(&self.themes_dir, &name, &toml_text)?;
        }
        Ok(())
    }

    /// List all themes in `themes/`, sorted alphabetically by name.
    pub fn list_themes(&self) -> SnipResult<Vec<ThemeInfo>> {
        if !self.themes_dir.exists() {
            return Ok(Vec::new());
        }
        let mut themes = Vec::new();
        for entry in fs::read_dir(&self.themes_dir)
            .map_err(|e| SnipError::io_error("read themes directory", self.themes_dir.clone(), e))?
        {
            let entry = entry
                .map_err(|e| SnipError::io_error("read theme entry", self.themes_dir.clone(), e))?;
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let name = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let is_bundled = is_bundled_theme_name(&name);
            themes.push(ThemeInfo {
                name,
                path,
                is_bundled,
            });
        }
        themes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(themes)
    }

    /// Parse a theme file and project it to a snp `Theme`.
    pub fn load_theme(&self, name: &str) -> SnipResult<Theme> {
        validate_theme_name(name)
            .map_err(|(msg, detail)| SnipError::runtime_error(msg, Some(detail)))?;
        let path = self.theme_path(name);
        let content = fs::read_to_string(&path)
            .map_err(|e| SnipError::io_error("read theme file", path.clone(), e))?;
        let halloy: HalloyTheme =
            toml::from_str(&content).map_err(|e| SnipError::toml_error("parse theme", e))?;
        Ok(halloy.into_snp_theme())
    }

    /// The currently-active theme name from `themes.toml`, if set.
    pub fn get_active_theme_name(&self) -> Option<String> {
        self.config.active.clone()
    }

    /// Persist `name` as the active theme.
    pub fn set_active_theme(&mut self, name: &str) -> SnipResult<()> {
        validate_theme_name(name)
            .map_err(|(msg, detail)| SnipError::runtime_error(msg, Some(detail)))?;
        self.config.active = Some(name.to_string());
        self.save_config()
    }

    /// Path to the `themes/` directory.
    pub fn themes_dir(&self) -> &Path {
        &self.themes_dir
    }

    fn theme_path(&self, name: &str) -> PathBuf {
        self.themes_dir.join(format!("{name}.toml"))
    }

    fn save_config(&self) -> SnipResult<()> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| SnipError::io_error("create config directory", parent, e))?;
        }
        let toml_str = toml::to_string_pretty(&self.config)
            .map_err(|e| SnipError::toml_error("serialize themes config", e))?;
        let tmp_path = self.config_path.with_extension("toml.tmp");
        fs::write(&tmp_path, toml_str)
            .map_err(|e| SnipError::io_error("write themes config", tmp_path.clone(), e))?;
        fs::rename(&tmp_path, &self.config_path).map_err(|e| {
            SnipError::io_error("rename themes config", self.config_path.clone(), e)
        })?;
        Ok(())
    }
}

/// Returns true if `name` is one of the themes compiled into the binary.
#[allow(dead_code)]
fn is_bundled_theme_name(name: &str) -> bool {
    if name == bundled_default_name() {
        return true;
    }
    super::_generated_bundled_themes::BUNDLED
        .iter()
        .any(|b| b.name == name)
}

/// The basename (no extension) of `DEFAULT_BUNDLED`. The constant
/// string in the generated file is `themes/Cyber Red.toml`, so the
/// default theme name is `"Cyber Red"`.
#[allow(dead_code)]
fn bundled_default_name() -> &'static str {
    "Cyber Red"
}

#[allow(dead_code)]
fn write_theme_if_missing(themes_dir: &Path, name: &str, content: &str) -> SnipResult<()> {
    let path = themes_dir.join(format!("{name}.toml"));
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, content)
        .map_err(|e| SnipError::io_error("write bundled theme", path.clone(), e))
}

/// Validates a theme name supplied by the user. Rejects path traversal,
/// slashes, and NULs. Allows spaces, parens, dots, dashes, underscores —
/// mirroring the diversity of Halloy community theme names.
#[allow(dead_code)]
fn validate_theme_name(name: &str) -> Result<(), (&'static str, &'static str)> {
    if name.is_empty() {
        return Err(("Invalid theme name", "Theme name cannot be empty"));
    }
    if name.len() > 100 {
        return Err((
            "Invalid theme name",
            "Theme name cannot exceed 100 characters",
        ));
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err((
            "Invalid theme name",
            "Theme name cannot contain path separators or NUL bytes",
        ));
    }
    if name.contains("..") {
        return Err(("Invalid theme name", "Theme name cannot contain '..'"));
    }
    Ok(())
}

// ============================================================================
// Process-global active theme
// ============================================================================

static ACTIVE_THEME: LazyLock<RwLock<Theme>> = LazyLock::new(|| RwLock::new(load_initial_theme()));

/// Returns the current active theme. Cheap: one read-lock acquisition
/// and a `Copy` of the 10-color struct.
pub fn get_theme() -> Theme {
    *ACTIVE_THEME.read().unwrap_or_else(|e| e.into_inner())
}

/// Replace the active theme. The TUI's next draw frame will see the
/// new colors. Used by the theme picker when the user navigates
/// (preview) and when they commit a selection.
#[allow(dead_code)]
pub fn set_active_theme(theme: Theme) {
    *ACTIVE_THEME.write().unwrap_or_else(|e| e.into_inner()) = theme;
}

/// Initial theme resolution, in priority order:
///
/// 1. `~/.config/snp/themes.toml` → `[active] name`, loaded from `themes/`
/// 2. `SNP_THEME` env var — `"dark"`/`"bright"`/`"light"`/`"auto"` →
///    the built-in consts (backward compat). Any other value is
///    treated as a theme filename and looked up in `themes/`.
/// 3. The hardcoded bundled default (`DEFAULT_BUNDLED`).
/// 4. The built-in `DARK_THEME` (last-resort safety net).
fn load_initial_theme() -> Theme {
    // (1) themes.toml
    if let Some(theme) = load_from_themes_toml() {
        return theme;
    }
    // (2) SNP_THEME
    if let Ok(value) = std::env::var("SNP_THEME") {
        if let Some(theme) = resolve_legacy_or_filename(&value) {
            return theme;
        }
    }
    // (3) bundled default
    if let Some(theme) = parse_halloy_string(super::_generated_bundled_themes::DEFAULT_BUNDLED) {
        return theme;
    }
    // (4) safety net
    DARK_THEME
}

fn load_from_themes_toml() -> Option<Theme> {
    let config_dir = get_config_dir();
    let config_path = config_dir.join("themes.toml");
    if !config_path.exists() {
        return None;
    }
    let content = fs::read_to_string(&config_path).ok()?;
    let cfg: ThemesConfig = toml::from_str(&content).ok()?;
    let name = cfg.active?;
    let themes_dir = config_dir.join("themes");
    let theme_path = themes_dir.join(format!("{name}.toml"));
    let content = fs::read_to_string(&theme_path).ok()?;
    parse_halloy_string(&content)
}

fn resolve_legacy_or_filename(value: &str) -> Option<Theme> {
    match value {
        "dark" => Some(DARK_THEME),
        "bright" | "light" => Some(BRIGHT_THEME),
        "auto" => {
            let is_light = std::env::var("COLORFGBG")
                .map(|v| v.starts_with("15;") || v.starts_with("7;"))
                .unwrap_or(false);
            Some(if is_light { BRIGHT_THEME } else { DARK_THEME })
        }
        other => {
            // Validate theme name to prevent path traversal
            if other.contains('/') || other.contains('\\') || other.contains("..") {
                tracing::warn!(
                    "Invalid theme name '{}' contains path separators, ignoring",
                    other
                );
                return None;
            }
            // Treat as a theme filename relative to themes/.
            let themes_dir = get_config_dir().join("themes");
            let path = themes_dir.join(format!("{other}.toml"));
            let content = fs::read_to_string(&path).ok()?;
            parse_halloy_string(&content)
        }
    }
}

fn parse_halloy_string(s: &str) -> Option<Theme> {
    let halloy: HalloyTheme = toml::from_str(s).ok()?;
    Some(halloy.into_snp_theme())
}

// ============================================================================
// Style helpers (unchanged from the pre-PR-2 file)
// ============================================================================

pub(crate) fn style_fg(fg: Color) -> Style {
    Style::default().fg(fg)
}

pub(crate) fn style_fg_bg(fg: Color, bg: Color) -> Style {
    Style::default().fg(fg).bg(bg)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------
    // Parser
    // ------------------------------------------------------------------

    #[test]
    fn parse_halloy_ferra_matches_expected_palette() {
        let toml = include_str!("../../themes/ferra.toml");
        let halloy: HalloyTheme = toml::from_str(toml).expect("parse ferra");
        let theme = halloy.into_snp_theme();
        // ferra has a warm-pink primary, cyan-ish text. Just sanity-check
        // a couple of values came from the file rather than the fallback.
        assert!(matches!(theme.primary, Color::Rgb(_, _, _)));
        assert!(matches!(theme.background, Color::Rgb(_, _, _)));
        assert_ne!(theme.background, Color::White);
    }

    #[test]
    fn parse_halloy_partial_only_general() {
        let toml = r##"
            [general]
            background = "#112233"
            border = "#445566"
        "##;
        let halloy: HalloyTheme = toml::from_str(toml).expect("parse partial");
        let theme = halloy.into_snp_theme();
        // background should come from general.background.
        assert_eq!(theme.background, Color::Rgb(0x11, 0x22, 0x33));
        // border should come from general.border (text.tertiary is empty).
        assert_eq!(theme.border, Color::Rgb(0x44, 0x55, 0x66));
        // text should fall back to Color::White since text.primary is empty.
        assert_eq!(theme.text, Color::White);
    }

    #[test]
    fn parse_halloy_with_alpha_hex() {
        assert_eq!(hex_to_color("#FF0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(hex_to_color("#FF0000AA"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(hex_to_color("#73000054"), Some(Color::Rgb(0x73, 0, 0)));
        // Not 7 chars, not 9 chars
        assert_eq!(hex_to_color("#FFF"), None);
        assert_eq!(hex_to_color(""), None);
        assert_eq!(hex_to_color("not a color"), None);
    }

    #[test]
    fn parse_halloy_unknown_section_ignored() {
        let toml = r##"
            [general]
            background = "#000000"
            unknown_field = "ignored"

            [some_unknown_section]
            foo = "bar"
        "##;
        let halloy: HalloyTheme = toml::from_str(toml).expect("parse with unknown");
        assert_eq!(halloy.general.background, "#000000");
        assert_eq!(
            hex_to_color(&halloy.general.background),
            Some(Color::Rgb(0, 0, 0))
        );
    }

    #[test]
    fn parse_halloy_extended_text_style() {
        // Extended form: { color = "#FF0000", font_style = "bold" }
        let toml = r##"
            [text]
            primary = { color = "#FF0000", font_style = "bold" }
        "##;
        let halloy: HalloyTheme = toml::from_str(toml).expect("parse extended");
        assert_eq!(halloy.text.primary.color, "#FF0000");
        assert_eq!(halloy.text.primary.font_style, Some(FontStyle::Bold));
        // And the conversion to Color works.
        assert_eq!(
            hex_to_color(&halloy.text.primary.color),
            Some(Color::Rgb(255, 0, 0))
        );
    }

    #[test]
    fn parse_halloy_kebab_font_style() {
        let toml = r##"
            [text]
            primary = { color = "#FF0000", font_style = "bold-italic" }
        "##;
        let halloy: HalloyTheme = toml::from_str(toml).expect("parse kebab font_style");
        assert_eq!(halloy.text.primary.font_style, Some(FontStyle::ItalicBold));
    }

    // ------------------------------------------------------------------
    // Mapping
    // ------------------------------------------------------------------

    #[test]
    fn mapping_chains_fallbacks() {
        // All empty except text.success → accent = success, primary = WHITE.
        let halloy = HalloyTheme {
            text: HalloyText {
                success: HalloyTextStyle {
                    color: "#010203".into(),
                    font_style: None,
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let theme = halloy.into_snp_theme();
        // primary chain: text.primary (empty) → text.secondary (empty) → WHITE
        assert_eq!(theme.primary, Color::White);
        // accent chain: text.tertiary (empty) → text.success
        assert_eq!(theme.accent, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn mapping_handles_transparent() {
        let halloy = HalloyTheme::default();
        let theme = halloy.into_snp_theme();
        // No fields set → all walk the chain to Color::White.
        assert_eq!(theme.primary, Color::White);
        assert_eq!(theme.text, Color::White);
    }

    // ------------------------------------------------------------------
    // ThemeManager
    // ------------------------------------------------------------------

    #[test]
    fn theme_manager_init_seeds_bundled() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = ThemeManager {
            config_dir: tmp.path().to_path_buf(),
            themes_dir: tmp.path().join("themes"),
            config_path: tmp.path().join("themes.toml"),
            config: ThemesConfig::default(),
        };
        mgr.init_themes_dir().expect("init");
        // All bundled themes plus the default should be on disk.
        let listed = mgr.list_themes().expect("list");
        assert!(
            listed.len() >= 40,
            "expected ~50 themes, got {}",
            listed.len()
        );
        // Cyber Red is the default and is marked bundled.
        let cyber = listed
            .iter()
            .find(|t| t.name == "Cyber Red")
            .expect("cyber-red");
        assert!(cyber.is_bundled);
        // ferra is bundled.
        let ferra = listed.iter().find(|t| t.name == "ferra").expect("ferra");
        assert!(ferra.is_bundled);
    }

    #[test]
    fn theme_manager_init_preserves_user_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = ThemeManager {
            config_dir: tmp.path().to_path_buf(),
            themes_dir: tmp.path().join("themes"),
            config_path: tmp.path().join("themes.toml"),
            config: ThemesConfig::default(),
        };
        mgr.init_themes_dir().expect("first init");
        // Overwrite a bundled theme with a sentinel.
        let path = mgr.themes_dir().join("ferra.toml");
        std::fs::write(&path, "# my custom ferra\n").unwrap();
        mgr.init_themes_dir().expect("second init");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("# my custom ferra"));
    }

    #[test]
    fn theme_manager_init_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = ThemeManager {
            config_dir: tmp.path().to_path_buf(),
            themes_dir: tmp.path().join("themes"),
            config_path: tmp.path().join("themes.toml"),
            config: ThemesConfig::default(),
        };
        mgr.init_themes_dir().expect("1");
        mgr.init_themes_dir().expect("2");
        mgr.init_themes_dir().expect("3");
    }

    #[test]
    fn theme_manager_load_active_persists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = ThemeManager {
            config_dir: tmp.path().to_path_buf(),
            themes_dir: tmp.path().join("themes"),
            config_path: tmp.path().join("themes.toml"),
            config: ThemesConfig::default(),
        };
        mgr.init_themes_dir().unwrap();
        mgr.set_active_theme("ferra").expect("set");
        // Simulate a process restart by re-reading from disk.
        let raw = std::fs::read_to_string(&mgr.config_path).unwrap();
        let cfg2: ThemesConfig = toml::from_str(&raw).expect("parse");
        assert_eq!(cfg2.active.as_deref(), Some("ferra"));
    }

    #[test]
    fn theme_manager_rejects_path_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = ThemeManager {
            config_dir: tmp.path().to_path_buf(),
            themes_dir: tmp.path().join("themes"),
            config_path: tmp.path().join("themes.toml"),
            config: ThemesConfig::default(),
        };
        mgr.init_themes_dir().unwrap();
        assert!(mgr.set_active_theme("../etc/passwd").is_err());
        assert!(mgr.set_active_theme("").is_err());
        assert!(mgr.set_active_theme("foo/bar").is_err());
        assert!(mgr.set_active_theme("a\\b").is_err());
        assert!(mgr.set_active_theme("..").is_err());
        assert!(mgr.load_theme("foo/../bar").is_err());
    }

    #[test]
    fn theme_manager_load_parses_real_theme() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = ThemeManager {
            config_dir: tmp.path().to_path_buf(),
            themes_dir: tmp.path().join("themes"),
            config_path: tmp.path().join("themes.toml"),
            config: ThemesConfig::default(),
        };
        mgr.init_themes_dir().unwrap();
        let ferra = mgr.load_theme("ferra").expect("load ferra");
        // ferra primary is #FECDB2.
        assert_eq!(ferra.primary, Color::Rgb(0xFE, 0xCD, 0xB2));
    }

    #[test]
    fn theme_manager_list_sorted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut mgr = ThemeManager {
            config_dir: tmp.path().to_path_buf(),
            themes_dir: tmp.path().join("themes"),
            config_path: tmp.path().join("themes.toml"),
            config: ThemesConfig::default(),
        };
        mgr.init_themes_dir().unwrap();
        let listed = mgr.list_themes().expect("list");
        let names: Vec<&str> = listed.iter().map(|t| t.name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort_by_key(|s| s.to_lowercase());
        assert_eq!(names, sorted, "themes should be sorted alphabetically");
    }

    // ------------------------------------------------------------------
    // Global state
    // ------------------------------------------------------------------

    #[test]
    fn set_active_theme_visible_to_get_theme() {
        let custom = Theme {
            primary: Color::Rgb(1, 2, 3),
            secondary: Color::Rgb(4, 5, 6),
            accent: Color::Rgb(7, 8, 9),
            background: Color::Rgb(10, 11, 12),
            text: Color::Rgb(13, 14, 15),
            border: Color::Rgb(16, 17, 18),
            selected_bg: Color::Rgb(19, 20, 21),
            muted: Color::Rgb(22, 23, 24),
            string_color: Color::Rgb(25, 26, 27),
            escape_color: Color::Rgb(28, 29, 30),
        };
        set_active_theme(custom);
        let got = get_theme();
        assert_eq!(got.primary, Color::Rgb(1, 2, 3));
        // Reset to a deterministic value so other tests aren't affected.
        set_active_theme(DARK_THEME);
        assert_eq!(get_theme().primary, DARK_THEME.primary);
    }

    // ------------------------------------------------------------------
    // Backward compat / fallback chain
    // ------------------------------------------------------------------

    #[test]
    fn default_bundled_is_valid() {
        let s = super::super::_generated_bundled_themes::DEFAULT_BUNDLED;
        let halloy: HalloyTheme = toml::from_str(s).expect("parse bundled default");
        let _ = halloy.into_snp_theme();
    }

    #[test]
    fn resolve_legacy_dark_and_bright() {
        // These functions don't read env vars directly so they're stable.
        assert_eq!(
            resolve_legacy_or_filename("dark").map(|t| t.background),
            Some(DARK_THEME.background),
        );
        assert_eq!(
            resolve_legacy_or_filename("bright").map(|t| t.background),
            Some(BRIGHT_THEME.background),
        );
        assert_eq!(
            resolve_legacy_or_filename("light").map(|t| t.background),
            Some(BRIGHT_THEME.background),
        );
        // auto depends on COLORFGBG; just verify it returns SOME theme.
        assert!(resolve_legacy_or_filename("auto").is_some());
    }
}
