//! WCAG 2.2 contrast ratios, APCA lightness contrast and the contrast check
//! every built-in theme passes (user themes that fail it still load, and the
//! front ends list the failing pairs).

use std::fmt;

use super::{Palette, Rgb};

pub(crate) fn linear(channel: u8) -> f64 {
    let s = f64::from(channel) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.2 relative luminance, 0 (black) to 1 (white).
pub fn luminance(c: Rgb) -> f64 {
    0.2126 * linear(c.0) + 0.7152 * linear(c.1) + 0.0722 * linear(c.2)
}

/// WCAG 2.2 contrast ratio of two colours, 1 to 21.
pub fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// APCA screen luminance with the soft black clamp.
fn apca_y(c: Rgb) -> f64 {
    let lin = |v: u8| (f64::from(v) / 255.0).powf(2.4);
    let y = 0.212_672_9 * lin(c.0) + 0.715_152_2 * lin(c.1) + 0.072_175_0 * lin(c.2);
    if y < 0.022 {
        y + (0.022 - y).powf(1.414)
    } else {
        y
    }
}

/// APCA-W3 0.0.98G-4g lightness contrast Lc of `text` on `bg`: positive for
/// dark text on a light background, negative for light text on a dark one.
pub fn apca_lc(text: Rgb, bg: Rgb) -> f64 {
    let (yt, yb) = (apca_y(text), apca_y(bg));
    if (yb - yt).abs() < 0.0005 {
        return 0.0;
    }
    let sapc = if yb > yt {
        let s = (yb.powf(0.56) - yt.powf(0.57)) * 1.14;
        if s < 0.1 {
            0.0
        } else {
            s - 0.027
        }
    } else {
        let s = (yb.powf(0.65) - yt.powf(0.62)) * 1.14;
        if s > -0.1 {
            0.0
        } else {
            s + 0.027
        }
    };
    sapc * 100.0
}

/// One promise of the contrast check: `fg` on each of `bgs` reaches
/// `min_ratio` (WCAG 2.2) and `min_lc` (APCA |Lc|).
#[derive(Clone, Copy, Debug)]
pub struct Requirement {
    pub fg: &'static str,
    pub bgs: &'static [&'static str],
    pub min_ratio: f64,
    pub min_lc: f64,
}

const fn req(fg: &'static str, bgs: &'static [&'static str], r: f64, lc: f64) -> Requirement {
    Requirement {
        fg,
        bgs,
        min_ratio: r,
        min_lc: lc,
    }
}

const ALL_BGS: &[&str] = &[
    "bg",
    "bg_end",
    "surface",
    "raised",
    "hover",
    "selection",
    "bubble",
    "code",
    "accent_surface",
];
const PAGES: &[&str] = &["bg", "bg_end", "surface", "raised"];
const STATUS_BGS: &[&str] = &["bg", "bg_end", "surface"];

/// Body text at WCAG AAA (7:1, SC 1.4.6) and APCA Bronze Lc 75; other text
/// at AA (4.5:1, SC 1.4.3) and Lc 60; borders and the focus colour at 3:1
/// (SC 1.4.11, 2.4.13).
pub const REQUIREMENTS: &[Requirement] = &[
    req("text", ALL_BGS, 7.0, 75.0),
    req(
        "dim",
        &[
            "bg",
            "bg_end",
            "surface",
            "raised",
            "hover",
            "bubble",
            "code",
            "accent_surface",
        ],
        4.5,
        60.0,
    ),
    req("accent", PAGES, 4.5, 60.0),
    req("accent2", PAGES, 4.5, 60.0),
    req("on_accent", &["accent", "accent2"], 4.5, 60.0),
    req("error", STATUS_BGS, 4.5, 60.0),
    req("warning", STATUS_BGS, 4.5, 60.0),
    req("success", STATUS_BGS, 4.5, 60.0),
    req("info", STATUS_BGS, 4.5, 60.0),
    req("border", PAGES, 3.0, 0.0),
    req(
        "accent",
        &["hover", "selection", "accent_surface"],
        3.0,
        0.0,
    ),
];

/// Alpha of the success or error tint under a diff row.
pub const DIFF_ALPHA: f64 = 0.12;
/// Alpha of a tone's tint under its tool icon tile.
pub const TILE_ALPHA: f64 = 0.16;

/// A promise on a tinted background: `fg` on `bg` blended with `tint` at
/// `alpha` reaches `min_ratio` (WCAG 2.2) and `min_lc` (APCA |Lc|).
#[derive(Clone, Copy, Debug)]
pub struct TintRequirement {
    pub fg: &'static str,
    pub bg: &'static str,
    pub tint: &'static str,
    pub alpha: f64,
    pub min_ratio: f64,
    pub min_lc: f64,
}

const fn tint(fg: &'static str, bg: &'static str, tint: &'static str, a: f64) -> TintRequirement {
    let (min_ratio, min_lc) = match fg.as_bytes() {
        b"text" => (7.0, 75.0),
        b"dim" => (4.5, 60.0),
        _ => (3.0, 0.0),
    };
    TintRequirement {
        fg,
        bg,
        tint,
        alpha: a,
        min_ratio,
        min_lc,
    }
}

/// Diff rows: body and dim text on `code` tinted with success or error.
/// Tool icon tiles: each tone's glyph on `surface` tinted with that tone.
pub const TINTS: &[TintRequirement] = &[
    tint("text", "code", "success", DIFF_ALPHA),
    tint("text", "code", "error", DIFF_ALPHA),
    tint("dim", "code", "success", DIFF_ALPHA),
    tint("dim", "code", "error", DIFF_ALPHA),
    tint("success", "surface", "success", TILE_ALPHA),
    tint("error", "surface", "error", TILE_ALPHA),
    tint("info", "surface", "info", TILE_ALPHA),
    tint("warning", "surface", "warning", TILE_ALPHA),
    tint("accent", "surface", "accent", TILE_ALPHA),
    tint("accent2", "surface", "accent2", TILE_ALPHA),
];

/// `bg` with `tint` laid over it at `alpha`, per sRGB channel, rounded.
pub fn blend(bg: Rgb, tint: Rgb, alpha: f64) -> Rgb {
    let mix = |b: u8, t: u8| (f64::from(b) * (1.0 - alpha) + f64::from(t) * alpha).round() as u8;
    Rgb(mix(bg.0, tint.0), mix(bg.1, tint.1), mix(bg.2, tint.2))
}

/// A colour pair below its requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub fg: &'static str,
    pub bg: &'static str,
    /// The tint laid over `bg` and its alpha, for a [`TINTS`] entry.
    pub tint: Option<(&'static str, f64)>,
    pub ratio: f64,
    pub lc: f64,
    pub min_ratio: f64,
    pub min_lc: f64,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} on {}", self.fg, self.bg)?;
        if let Some((tint, alpha)) = self.tint {
            write!(f, "+{tint}@{alpha:.2}")?;
        }
        write!(f, ": {:.2}:1 (needs {}:1)", self.ratio, self.min_ratio)?;
        if self.min_lc > 0.0 {
            write!(f, ", APCA Lc {:.1} (needs {})", self.lc.abs(), self.min_lc)?;
        }
        Ok(())
    }
}

/// Every colour pair of `palette` below its [`REQUIREMENTS`] or [`TINTS`]
/// entry; empty when the palette passes.
pub fn check(palette: &Palette) -> Vec<Failure> {
    let mut failures = Vec::new();
    for r in REQUIREMENTS {
        for &bg in r.bgs {
            let pair = (r.fg, bg, None, r.min_ratio, r.min_lc);
            failures.extend(check_pair(palette, pair));
        }
    }
    for t in TINTS {
        let pair = (t.fg, t.bg, Some((t.tint, t.alpha)), t.min_ratio, t.min_lc);
        failures.extend(check_pair(palette, pair));
    }
    failures
}

type Pair = (
    &'static str,
    &'static str,
    Option<(&'static str, f64)>,
    f64,
    f64,
);

/// The failure of one pair, None when it passes or a role is missing.
fn check_pair(
    palette: &Palette,
    (fg_name, bg_name, tint, min_ratio, min_lc): Pair,
) -> Option<Failure> {
    let fg = palette.get(fg_name)?;
    let mut bg = palette.get(bg_name)?;
    if let Some((tint_name, alpha)) = tint {
        bg = blend(bg, palette.get(tint_name)?, alpha);
    }
    let (ratio, lc) = (contrast_ratio(fg, bg), apca_lc(fg, bg));
    (ratio < min_ratio || lc.abs() < min_lc).then_some(Failure {
        fg: fg_name,
        bg: bg_name,
        tint,
        ratio,
        lc,
        min_ratio,
        min_lc,
    })
}
