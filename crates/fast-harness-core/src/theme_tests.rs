use super::*;

/// `(text role, background roles, least WCAG ratio, least |APCA Lc|)`.
type Check = (&'static str, &'static [&'static str], f64, f64);

/// The contrast every theme promises (module docs).
const CHECKS: &[Check] = &[
    (
        "text",
        &[
            "bg",
            "surface",
            "raised",
            "hover",
            "selection",
            "bubble",
            "code",
        ],
        7.0,
        75.0,
    ),
    (
        "dim",
        &["bg", "surface", "raised", "hover", "bubble"],
        4.5,
        60.0,
    ),
    ("accent", &["bg", "surface", "raised"], 4.5, 60.0),
    ("on_accent", &["accent"], 4.5, 60.0),
    ("error", &["bg", "surface"], 4.5, 60.0),
    ("warning", &["bg", "surface"], 4.5, 60.0),
    ("success", &["bg", "surface"], 4.5, 60.0),
    // Non-text contrast: control borders and the focus ring.
    ("border", &["bg", "surface", "raised"], 3.0, 0.0),
    ("accent", &["hover", "selection"], 3.0, 0.0),
];

fn role(palette: &Palette, name: &str) -> Rgb {
    palette
        .roles()
        .iter()
        .find(|(role, _)| *role == name)
        .map(|(_, colour)| *colour)
        .unwrap_or_else(|| panic!("no role {name}"))
}

#[test]
fn reference_values() {
    let (black, white, grey) = (Rgb(0, 0, 0), Rgb(255, 255, 255), Rgb(136, 136, 136));
    assert!((contrast_ratio(black, white) - 21.0).abs() < 1e-9);
    assert!((contrast_ratio(Rgb(0x76, 0x76, 0x76), white) - 4.54).abs() < 0.01);
    // apca-w3 reference results.
    assert!((apca_lc(black, white) - 106.041).abs() < 0.01);
    assert!((apca_lc(white, black) + 107.885).abs() < 0.01);
    assert!((apca_lc(grey, white) - 63.056).abs() < 0.01);
    assert!((apca_lc(white, grey) + 68.541).abs() < 0.01);
    assert!((apca_lc(Rgb(0x11, 0x22, 0x33), Rgb(0xdd, 0xee, 0xff)) - 91.668).abs() < 0.01);
    assert_eq!(apca_lc(grey, grey), 0.0);
}

#[test]
fn every_theme_meets_wcag_and_apca() {
    let mut failures = Vec::new();
    for theme in &THEMES {
        let p = &theme.palette;
        for (fg, backgrounds, ratio, lc) in CHECKS {
            for bg in *backgrounds {
                let (a, b) = (role(p, fg), role(p, bg));
                let (r, l) = (contrast_ratio(a, b), apca_lc(a, b));
                if r < *ratio || l.abs() < *lc {
                    failures.push(format!("{}: {fg} on {bg} {r:.2}:1 Lc {l:.1}", theme.id));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn dark_flag_follows_the_background() {
    for theme in &THEMES {
        let bg = luminance(theme.palette.bg);
        assert_eq!(theme.dark, bg < 0.18, "{}", theme.id);
    }
}

#[test]
fn greyscale_themes_hold_only_greys() {
    let grey: Vec<&Theme> = THEMES
        .iter()
        .filter(|t| t.id.starts_with("greyscale"))
        .collect();
    assert_eq!(grey.len(), 2);
    for theme in grey {
        for (name, colour) in theme.palette.roles() {
            assert!(colour.is_grey(), "{} {name} {}", theme.id, colour.hex());
        }
    }
    assert!(!THEMES[0].palette.accent.is_grey());
}

#[test]
fn lookup_and_cycle() {
    assert_eq!(by_id("light").id, "light");
    assert_eq!(by_id(" greyscale-dark ").id, "greyscale-dark");
    assert_eq!(by_id("nope").id, DEFAULT_THEME);
    assert_eq!(next("dark").id, "light");
    assert_eq!(next("greyscale-light").id, "dark");
    assert_eq!(next("nope").id, "dark");
    let mut ids: Vec<&str> = THEMES.iter().map(|t| t.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), THEMES.len());
    assert_eq!(rgb(0x1B1D22).hex(), "#1b1d22");
}
