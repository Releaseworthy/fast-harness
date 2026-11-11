//! Colour themes shared by the TUI and the GUI.
//!
//! Every theme meets the WCAG 2.2 contrast criteria (W3C Recommendation,
//! <https://www.w3.org/TR/WCAG22/>, read 2026-11-11): body text at level AAA
//! (7:1, SC 1.4.6), secondary, link and status text at AA (4.5:1, SC 1.4.3),
//! control borders and the focus colour at 3:1 (SC 1.4.11, 2.4.13). It also
//! meets the APCA Bronze levels of the WCAG 3 draft (|Lc| 75 for body text,
//! 60 for other text; APCA-W3 0.0.98G-4g, <https://github.com/Myndex/apca-w3>,
//! read 2026-11-11). The greyscale themes hold only neutral greys (r = g = b)
//! for text and background alike, so a front end never lets colour alone
//! carry a meaning (SC 1.4.1): it pairs a status colour with a word or a mark.

/// An sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

const fn rgb(hex: u32) -> Rgb {
    Rgb(
        ((hex >> 16) & 0xff) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

/// The colour roles a front end draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Window, transcript and page background.
    pub bg: Rgb,
    /// Sidebar, header and status bars, cards and the composer.
    pub surface: Rgb,
    /// Input fields, dropdowns and buttons at rest.
    pub raised: Rgb,
    /// A hovered row or control.
    pub hover: Rgb,
    /// A selected row or list item.
    pub selection: Rgb,
    /// The user's message bubble.
    pub bubble: Rgb,
    /// Code spans and code blocks.
    pub code: Rgb,
    /// Decorative separators; no contrast requirement.
    pub line: Rgb,
    /// Borders of inputs, dropdowns and buttons: 3:1 against bg, surface and raised.
    pub border: Rgb,
    /// Body text.
    pub text: Rgb,
    /// Secondary text: captions, timings, thinking, hints.
    pub dim: Rgb,
    /// Links, the focus ring, active markers and the primary button.
    pub accent: Rgb,
    /// Text and icons on an `accent` background.
    pub on_accent: Rgb,
    pub error: Rgb,
    pub warning: Rgb,
    pub success: Rgb,
}

impl Palette {
    /// Every role with its name, in declaration order.
    pub fn roles(&self) -> [(&'static str, Rgb); 16] {
        [
            ("bg", self.bg),
            ("surface", self.surface),
            ("raised", self.raised),
            ("hover", self.hover),
            ("selection", self.selection),
            ("bubble", self.bubble),
            ("code", self.code),
            ("line", self.line),
            ("border", self.border),
            ("text", self.text),
            ("dim", self.dim),
            ("accent", self.accent),
            ("on_accent", self.on_accent),
            ("error", self.error),
            ("warning", self.warning),
            ("success", self.success),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Stable id, saved as `theme` in `config.json`.
    pub id: &'static str,
    /// The name pickers show.
    pub name: &'static str,
    /// A dark background: front ends pick the dark variant of native widgets.
    pub dark: bool,
    pub palette: Palette,
}

/// The id of the theme used when none (or an unknown one) is configured.
pub const DEFAULT_THEME: &str = "dark";

/// The themes in picker order.
pub static THEMES: [Theme; 4] = [
    Theme {
        id: "dark",
        name: "Dark",
        dark: true,
        palette: Palette {
            bg: rgb(0x1B1D22),
            surface: rgb(0x23262C),
            raised: rgb(0x2B2F36),
            hover: rgb(0x323741),
            selection: rgb(0x2E3A50),
            bubble: rgb(0x283244),
            code: rgb(0x262A31),
            line: rgb(0x343944),
            border: rgb(0x7B8390),
            text: rgb(0xDDE1E7),
            dim: rgb(0xBCC3CD),
            accent: rgb(0x93BDF8),
            on_accent: rgb(0x0F1420),
            error: rgb(0xF5ADA5),
            warning: rgb(0xEBC985),
            success: rgb(0x9AD7A9),
        },
    },
    Theme {
        id: "light",
        name: "Light",
        dark: false,
        palette: Palette {
            bg: rgb(0xF6F7F9),
            surface: rgb(0xECEEF2),
            raised: rgb(0xFFFFFF),
            hover: rgb(0xE3E6EB),
            selection: rgb(0xDCE7F9),
            bubble: rgb(0xE1EAF8),
            code: rgb(0xEDEFF3),
            line: rgb(0xD5D9E0),
            border: rgb(0x7D8590),
            text: rgb(0x1F2328),
            dim: rgb(0x57606A),
            accent: rgb(0x2B5FB3),
            on_accent: rgb(0xFFFFFF),
            error: rgb(0xB3261E),
            warning: rgb(0x8A5300),
            success: rgb(0x1D7A3A),
        },
    },
    Theme {
        id: "greyscale-dark",
        name: "Greyscale dark",
        dark: true,
        palette: Palette {
            bg: rgb(0x1C1C1C),
            surface: rgb(0x242424),
            raised: rgb(0x2C2C2C),
            hover: rgb(0x343434),
            selection: rgb(0x3A3A3A),
            bubble: rgb(0x333333),
            code: rgb(0x282828),
            line: rgb(0x383838),
            border: rgb(0x767676),
            text: rgb(0xDEDEDE),
            dim: rgb(0xBEBEBE),
            accent: rgb(0xEDEDED),
            on_accent: rgb(0x1C1C1C),
            error: rgb(0xEBEBEB),
            warning: rgb(0xC9C9C9),
            success: rgb(0xB8B8B8),
        },
    },
    Theme {
        id: "greyscale-light",
        name: "Greyscale light",
        dark: false,
        palette: Palette {
            bg: rgb(0xF4F4F4),
            surface: rgb(0xEAEAEA),
            raised: rgb(0xFFFFFF),
            hover: rgb(0xE0E0E0),
            selection: rgb(0xD6D6D6),
            bubble: rgb(0xE2E2E2),
            code: rgb(0xECECEC),
            line: rgb(0xD4D4D4),
            border: rgb(0x858585),
            text: rgb(0x1E1E1E),
            dim: rgb(0x595959),
            accent: rgb(0x262626),
            on_accent: rgb(0xF4F4F4),
            error: rgb(0x1A1A1A),
            warning: rgb(0x3D3D3D),
            success: rgb(0x4A4A4A),
        },
    },
];

/// The theme with `id`, else the default theme.
pub fn by_id(id: &str) -> &'static Theme {
    let id = id.trim();
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
}

/// The theme after `id` in picker order, wrapping around; the first one for
/// an unknown id.
pub fn next(id: &str) -> &'static Theme {
    let id = id.trim();
    let index = THEMES
        .iter()
        .position(|t| t.id == id)
        .map_or(0, |i| (i + 1) % THEMES.len());
    &THEMES[index]
}

fn linear(channel: u8) -> f64 {
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

#[cfg(test)]
#[path = "theme_tests.rs"]
mod tests;
