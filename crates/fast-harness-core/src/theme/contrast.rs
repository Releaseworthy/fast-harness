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

/// A colour pair below its requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub fg: &'static str,
    pub bg: &'static str,
    pub ratio: f64,
    pub lc: f64,
    pub min_ratio: f64,
    pub min_lc: f64,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} on {}: {:.2}:1 (needs {}:1)",
            self.fg, self.bg, self.ratio, self.min_ratio
        )?;
        if self.min_lc > 0.0 {
            write!(f, ", APCA Lc {:.1} (needs {})", self.lc.abs(), self.min_lc)?;
        }
        Ok(())
    }
}

/// Every colour pair of `palette` below its [`REQUIREMENTS`] entry; empty
/// when the palette passes.
pub fn check(palette: &Palette) -> Vec<Failure> {
    let mut failures = Vec::new();
    for r in REQUIREMENTS {
        let Some(fg) = palette.get(r.fg) else {
            continue;
        };
        for &bg_name in r.bgs {
            let Some(bg) = palette.get(bg_name) else {
                continue;
            };
            let (ratio, lc) = (contrast_ratio(fg, bg), apca_lc(fg, bg));
            if ratio < r.min_ratio || lc.abs() < r.min_lc {
                failures.push(Failure {
                    fg: r.fg,
                    bg: bg_name,
                    ratio,
                    lc,
                    min_ratio: r.min_ratio,
                    min_lc: r.min_lc,
                });
            }
        }
    }
    failures
}
