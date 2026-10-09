//! `config.toml` loading. Everything has a default; a broken file never
//! prevents the viewer from starting.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::input::command::parse_hex;

pub const APP_ID: &str = "io.github.notflask.Mizu";

pub fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("io.github", "notflask", "Mizu")
}

pub fn config_path() -> Option<PathBuf> {
    project_dirs().map(|d| d.config_dir().join("config.toml"))
}

/// Directory for the session file and the pipeline cache.
pub fn state_dir() -> Option<PathBuf> {
    let d = project_dirs()?;
    Some(
        d.state_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| d.data_local_dir().to_path_buf()),
    )
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawDark {
    background: Option<String>,
    foreground: Option<String>,
    separator: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawPen {
    width: Option<f32>,
    palette: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawKeys {
    normal: BTreeMap<String, String>,
    draw: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct RawConfig {
    dark_by_default: Option<bool>,
    statusbar: Option<bool>,
    scroll_step: Option<f32>,
    zoom_step: Option<f32>,
    tile_cache_mb: Option<u32>,
    dark: RawDark,
    pen: RawPen,
    keys: RawKeys,
}

/// Validated, ready-to-use settings.
#[derive(Debug, Clone)]
pub struct Settings {
    pub dark_by_default: bool,
    pub statusbar: bool,
    /// Logical pixels per `j` / `k`.
    pub scroll_step: f32,
    pub zoom_step: f32,
    pub tile_cache_mb: u32,
    pub dark_bg: [u8; 3],
    pub dark_fg: [u8; 3],
    pub dark_separator: Option<[u8; 3]>,
    pub pen_width: f32,
    pub palette: Vec<[u8; 3]>,
    pub keys_normal: BTreeMap<String, String>,
    pub keys_draw: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dark_by_default: false,
            statusbar: true,
            scroll_step: 60.0,
            zoom_step: 1.2,
            tile_cache_mb: 256,
            dark_bg: [0, 0, 0],
            dark_fg: [255, 255, 255],
            dark_separator: None,
            pen_width: 1.5,
            palette: default_palette(),
            keys_normal: BTreeMap::new(),
            keys_draw: BTreeMap::new(),
        }
    }
}

pub fn default_palette() -> Vec<[u8; 3]> {
    vec![
        [0x1a, 0x1a, 0x1a],
        [0xe0, 0x31, 0x31],
        [0x19, 0x71, 0xc2],
        [0x2f, 0x9e, 0x44],
        [0xf0, 0x8c, 0x00],
        [0x9c, 0x36, 0xb5],
    ]
}

impl Settings {
    /// Parse a TOML document. Invalid values fall back to defaults and are
    /// reported in the returned warnings. A syntax error rejects the file.
    pub fn parse(text: &str) -> Result<(Settings, Vec<String>), String> {
        let raw: RawConfig = toml::from_str(text).map_err(|e| e.message().to_string())?;
        let mut s = Settings::default();
        let mut warn = Vec::new();

        if let Some(v) = raw.dark_by_default {
            s.dark_by_default = v;
        }
        if let Some(v) = raw.statusbar {
            s.statusbar = v;
        }
        if let Some(v) = raw.scroll_step {
            if v.is_finite() && v > 0.0 {
                s.scroll_step = v;
            } else {
                warn.push("scroll_step must be positive".into());
            }
        }
        if let Some(v) = raw.zoom_step {
            if v.is_finite() && v > 1.0 {
                s.zoom_step = v;
            } else {
                warn.push("zoom_step must be greater than 1".into());
            }
        }
        if let Some(v) = raw.tile_cache_mb {
            s.tile_cache_mb = v.clamp(32, 4096);
        }
        let mut color = |name: &str, v: &Option<String>, dst: &mut [u8; 3]| {
            if let Some(text) = v {
                match parse_hex(text) {
                    Some(c) => *dst = c,
                    None => warn.push(format!("{name}: invalid colour {text:?}")),
                }
            }
        };
        color("dark.background", &raw.dark.background, &mut s.dark_bg);
        color("dark.foreground", &raw.dark.foreground, &mut s.dark_fg);
        if let Some(text) = &raw.dark.separator {
            match parse_hex(text) {
                Some(c) => s.dark_separator = Some(c),
                None => warn.push(format!("dark.separator: invalid colour {text:?}")),
            }
        }
        if let Some(w) = raw.pen.width {
            if w.is_finite() && (0.25..=20.0).contains(&w) {
                s.pen_width = w;
            } else {
                warn.push("pen.width must be between 0.25 and 20".into());
            }
        }
        if let Some(p) = &raw.pen.palette {
            let parsed: Vec<[u8; 3]> = p.iter().filter_map(|c| parse_hex(c)).collect();
            if parsed.is_empty() || parsed.len() != p.len() {
                warn.push("pen.palette: expected a list of #rrggbb colours".into());
            }
            if !parsed.is_empty() {
                s.palette = parsed.into_iter().take(9).collect();
            }
        }
        s.keys_normal = raw.keys.normal;
        s.keys_draw = raw.keys.draw;
        Ok((s, warn))
    }

    /// Load the user's config. Returns the settings and a message to show in
    /// the status line if something was wrong.
    pub fn load() -> (Settings, Option<String>) {
        let Some(path) = config_path() else {
            return (Settings::default(), None);
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Settings::default(), None)
            }
            Err(e) => {
                return (
                    Settings::default(),
                    Some(format!("config: cannot read {}: {e}", path.display())),
                )
            }
        };
        match Settings::parse(&text) {
            Ok((s, warnings)) => {
                let msg = (!warnings.is_empty()).then(|| format!("config: {}", warnings.join("; ")));
                (s, msg)
            }
            Err(e) => (Settings::default(), Some(format!("config: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_pure_black_and_white() {
        let s = Settings::default();
        assert_eq!(s.dark_bg, [0, 0, 0]);
        assert_eq!(s.dark_fg, [255, 255, 255]);
        assert_eq!(s.palette.len(), 6);
    }

    #[test]
    fn full_example_parses() {
        let text = r##"
            dark_by_default = true
            statusbar = false
            scroll_step = 80
            zoom_step = 1.25
            tile_cache_mb = 128
            [dark]
            background = "#101010"
            foreground = "#eeeeee"
            separator = "#222222"
            [pen]
            width = 2.0
            palette = ["#000000", "#ff0000"]
            [keys.normal]
            "<C-n>" = "toggle_dark"
            [keys.draw]
            "x" = "toggle_eraser"
        "##;
        let (s, w) = Settings::parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert!(s.dark_by_default && !s.statusbar);
        assert_eq!(s.scroll_step, 80.0);
        assert_eq!(s.dark_bg, [0x10, 0x10, 0x10]);
        assert_eq!(s.dark_separator, Some([0x22, 0x22, 0x22]));
        assert_eq!(s.palette.len(), 2);
        assert_eq!(s.keys_normal.get("<C-n>").map(String::as_str), Some("toggle_dark"));
    }

    #[test]
    fn bad_values_fall_back_with_warning() {
        let (s, w) = Settings::parse("scroll_step = -3\n[dark]\nbackground = \"nope\"").unwrap();
        assert_eq!(s.scroll_step, 60.0);
        assert_eq!(s.dark_bg, [0, 0, 0]);
        assert_eq!(w.len(), 2);
    }

    #[test]
    fn syntax_error_is_reported() {
        assert!(Settings::parse("this is = = not toml").is_err());
        // Unknown keys are rejected so typos do not go unnoticed.
        assert!(Settings::parse("scrol_step = 3").is_err());
    }

    #[test]
    fn empty_file_is_fine() {
        let (s, w) = Settings::parse("").unwrap();
        assert!(w.is_empty());
        assert_eq!(s.zoom_step, 1.2);
    }
}
