//! Colour themes shared by the TUI and the GUI.
//!
//! Every theme is one JSON document in one format (`docs/themes.md`,
//! [`format`]): the built-in themes ship in the binary (`theme/builtin/`),
//! user themes live in `<app_home>/themes/*.json`. A built-in theme meets the
//! WCAG 2.2 contrast criteria (W3C Recommendation,
//! <https://www.w3.org/TR/WCAG22/>, read 2026-11-11): body text at level AAA
//! (7:1, SC 1.4.6), secondary, link and status text at AA (4.5:1, SC 1.4.3),
//! control borders and the focus colour at 3:1 (SC 1.4.11, 2.4.13). It also
//! meets the APCA Bronze levels of the WCAG 3 draft (|Lc| 75 for body text,
//! 60 for other text; APCA-W3 0.0.98G-4g, <https://github.com/Myndex/apca-w3>,
//! read 2026-11-11), at every greyscale level ([`greyscale`]). A user theme
//! that fails the check still loads with a note listing the failing pairs.
//! A front end never lets colour alone carry a meaning (SC 1.4.1): it pairs
//! a status colour with a word or a mark.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

mod contrast;
pub mod format;
mod grey;
mod palette;

pub use contrast::{apca_lc, check, contrast_ratio, luminance, Failure, Requirement, REQUIREMENTS};
pub use grey::{greyscale, level_of, parse_level, GREYSCALE_MAX};
pub use palette::{Palette, ROLES};

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// `#rrggbb` in lower case.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    /// A neutral grey: red, green and blue are equal.
    pub fn is_grey(self) -> bool {
        self.0 == self.1 && self.1 == self.2
    }

    /// `#rrggbb` (either case); `None` for anything else.
    pub fn parse(text: &str) -> Option<Rgb> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
        Some(Rgb(channel(0)?, channel(2)?, channel(4)?))
    }
}

/// A resolved theme: built-in or read from a user file.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Stable id, saved as `theme` in `config.json`.
    pub id: String,
    /// The name pickers show.
    pub name: String,
    /// `"kind": "dark"`: front ends pick the dark variant of native widgets.
    pub dark: bool,
    /// The theme it starts from, if any.
    pub extends: Option<String>,
    pub palette: Palette,
    /// The user file it came from; `None` for a built-in theme.
    pub file: Option<PathBuf>,
}

impl Theme {
    pub fn is_builtin(&self) -> bool {
        self.file.is_none()
    }

    /// The palette at greyscale `level` (0 to 100).
    pub fn shown(&self, level: u8) -> Palette {
        greyscale(&self.palette, level)
    }
}

/// The id of the theme used when none (or an unknown one) is configured.
pub const DEFAULT_THEME: &str = "dark";

/// The built-in theme files in picker order.
const BUILTIN_FILES: [(&str, &str); 8] = [
    ("dark.json", include_str!("theme/builtin/dark.json")),
    ("light.json", include_str!("theme/builtin/light.json")),
    ("aurora.json", include_str!("theme/builtin/aurora.json")),
    (
        "synthwave.json",
        include_str!("theme/builtin/synthwave.json"),
    ),
    ("sunset.json", include_str!("theme/builtin/sunset.json")),
    ("emerald.json", include_str!("theme/builtin/emerald.json")),
    ("sakura.json", include_str!("theme/builtin/sakura.json")),
    ("citrus.json", include_str!("theme/builtin/citrus.json")),
];

/// The built-in themes, parsed once with the user theme parser.
pub fn builtins() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        let docs: Vec<format::Doc> = BUILTIN_FILES
            .iter()
            .filter_map(|(file, text)| format::parse(text, &format!("built-in {file}")).ok())
            .collect();
        format::resolve(&docs, &[]).0
    })
}

/// The id a removed theme id now means: the greyscale themes became the
/// greyscale setting (`greyscale-dark` is `dark` at greyscale 100).
pub fn legacy_greyscale(id: &str) -> Option<&'static str> {
    match id.trim() {
        "greyscale-dark" => Some("dark"),
        "greyscale-light" => Some("light"),
        _ => None,
    }
}

/// The built-in theme with `id`, else the default theme.
pub fn by_id(id: &str) -> &'static Theme {
    let id = legacy_greyscale(id).unwrap_or(id.trim());
    let all = builtins();
    // theme_tests checks that every built-in parses, so `all` is not empty.
    all.iter()
        .find(|t| t.id == id)
        .or_else(|| all.iter().find(|t| t.id == DEFAULT_THEME))
        .unwrap_or(&all[0])
}

/// The themes a front end offers: the built-ins, then the user themes, with
/// the notes of the last load (skipped files, errors, contrast failures).
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeSet {
    themes: Vec<Theme>,
    notes: Vec<String>,
}

impl Default for ThemeSet {
    fn default() -> ThemeSet {
        ThemeSet::builtin()
    }
}

impl ThemeSet {
    /// The built-in themes only.
    pub fn builtin() -> ThemeSet {
        ThemeSet {
            themes: builtins().to_vec(),
            notes: Vec::new(),
        }
    }

    /// The built-ins and the themes of `<app_home>/themes/*.json` (read at
    /// start and on Reload).
    pub fn load(app_home: &Path) -> ThemeSet {
        let mut set = ThemeSet::builtin();
        let (user, notes) = format::load_user(app_home, builtins());
        set.themes.extend(user);
        set.notes = notes;
        set
    }

    /// Every theme in picker order (never empty).
    pub fn themes(&self) -> &[Theme] {
        &self.themes
    }

    /// Messages about the user theme files, for the front end to show.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// The picker index of the theme `id` resolves to (an unknown id: the
    /// default theme).
    pub fn position(&self, id: &str) -> usize {
        let id = legacy_greyscale(id).unwrap_or(id.trim());
        let find = |id: &str| self.themes.iter().position(|t| t.id == id);
        find(id).or_else(|| find(DEFAULT_THEME)).unwrap_or(0)
    }

    /// The theme with `id`, else the default theme.
    pub fn get(&self, id: &str) -> &Theme {
        &self.themes[self.position(id)]
    }

    /// The theme at a picker index.
    pub fn at(&self, index: usize) -> Option<&Theme> {
        self.themes.get(index)
    }

    /// The theme after `id` in picker order, wrapping around.
    pub fn next(&self, id: &str) -> &Theme {
        &self.themes[(self.position(id) + 1) % self.themes.len()]
    }
}

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
