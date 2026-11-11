//! The greyscale setting: 0 (full colour) to 100 (pure greys), applied to
//! any palette. Each colour mixes in linear light toward the grey of the
//! same relative luminance, so WCAG contrast ratios stay the same at every
//! level and at 100 every colour has r = g = b.

use super::contrast::{linear, luminance};
use super::{Palette, Rgb};

/// The highest greyscale level: pure greys.
pub const GREYSCALE_MAX: u8 = 100;

fn encode(l: f64) -> u8 {
    let s = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    // In range after the clamp, so the cast cannot truncate.
    (s.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// `c` moved `t` (0 to 1) of the way, in linear light, toward the grey of
/// its own relative luminance.
fn toward_grey(c: Rgb, t: f64) -> Rgb {
    let y = luminance(c);
    let mix = |v: u8| encode(linear(v) * (1.0 - t) + y * t);
    if t >= 1.0 {
        let g = encode(y);
        return Rgb(g, g, g);
    }
    Rgb(mix(c.0), mix(c.1), mix(c.2))
}

/// `palette` at greyscale `level` (0 to 100; higher values count as 100).
pub fn greyscale(palette: &Palette, level: u8) -> Palette {
    let level = level.min(GREYSCALE_MAX);
    if level == 0 {
        return *palette;
    }
    let t = f64::from(level) / f64::from(GREYSCALE_MAX);
    palette.map(|c| toward_grey(c, t))
}

/// A greyscale level from user text or a JSON number: a number, rounded and
/// clamped to 0..=100; `None` when it is not a number.
pub fn parse_level(text: &str) -> Option<u8> {
    let value: f64 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    level_of(value)
}

/// A finite number rounded and clamped to 0..=100.
pub fn level_of(value: f64) -> Option<u8> {
    // Clamped to 0..=100 before the cast.
    value
        .is_finite()
        .then(|| value.round().clamp(0.0, f64::from(GREYSCALE_MAX)) as u8)
}
