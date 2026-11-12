use std::path::PathBuf;

use super::*;

/// The greyscale levels every built-in theme is checked at.
const LEVELS: [u8; 5] = [0, 25, 50, 75, 100];

const IDS: [&str; 8] = [
    "dark",
    "light",
    "aurora",
    "synthwave",
    "sunset",
    "emerald",
    "sakura",
    "citrus",
];

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir =
            std::env::temp_dir().join(format!("fast-harness-theme-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(format::THEMES_DIR)).unwrap();
        TempDir(dir)
    }

    fn write(&self, file: &str, text: &str) {
        std::fs::write(self.0.join(format::THEMES_DIR).join(file), text).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn doc(id: &str, extends: Option<&str>, colors: &str) -> String {
    let extends = extends.map_or(String::new(), |e| format!(r#""extends":"{e}","#));
    format!(
        r#"{{"version":1,"id":"{id}","name":"N {id}","kind":"dark",{extends}"colors":{{{colors}}}}}"#
    )
}

/// `(weakest body ratio, weakest body |Lc|, weakest other-text ratio,
/// weakest other |Lc|)` of `p`.
fn weakest(p: &Palette) -> (f64, f64, f64, f64) {
    let mut w = (99.0_f64, 999.0_f64, 99.0_f64, 999.0_f64);
    for r in REQUIREMENTS {
        for bg in r.bgs {
            let (a, b) = (p.get(r.fg).unwrap(), p.get(bg).unwrap());
            let (ratio, lc) = (contrast_ratio(a, b), apca_lc(a, b).abs());
            if r.min_ratio >= 7.0 {
                (w.0, w.1) = (w.0.min(ratio), w.1.min(lc));
            } else if r.min_lc > 0.0 {
                (w.2, w.3) = (w.2.min(ratio), w.3.min(lc));
            }
        }
    }
    w
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
fn builtins_parse_in_picker_order() {
    let ids: Vec<&str> = builtins().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, IDS);
    let light: Vec<&str> = builtins()
        .iter()
        .filter(|t| !t.dark)
        .map(|t| t.id.as_str())
        .collect();
    assert_eq!(light, ["light", "sakura", "citrus"]);
    for theme in builtins() {
        assert!(
            theme.is_builtin() && theme.extends.is_none(),
            "{}",
            theme.id
        );
        let bg = luminance(theme.palette.bg);
        assert_eq!(theme.dark, bg < 0.18, "{} kind follows bg", theme.id);
        assert!(!theme.palette.accent.is_grey(), "{} accent", theme.id);
    }
}

#[test]
fn every_builtin_meets_wcag_and_apca_at_every_greyscale_level() {
    let mut failures = Vec::new();
    for theme in builtins() {
        for level in LEVELS {
            for f in check(&theme.shown(level)) {
                failures.push(format!("{} greyscale {level}: {f}", theme.id));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The design table: `cargo test -p fast-harness-core palette_report -- --ignored --nocapture`.
#[test]
#[ignore]
fn palette_report() {
    for theme in builtins() {
        for level in LEVELS {
            let (br, bl, or, ol) = weakest(&theme.shown(level));
            println!(
                "{:<10} grey {level:>3}: body {br:.2}:1 Lc {bl:.1} | other {or:.2}:1 Lc {ol:.1}",
                theme.id
            );
        }
    }
}

#[test]
fn greyscale_levels() {
    let p = by_id("aurora").palette;
    assert_eq!(greyscale(&p, 0), p);
    let grey = greyscale(&p, 100);
    for (name, c) in grey.roles() {
        assert!(c.is_grey(), "{name} {}", c.hex());
    }
    assert_eq!(greyscale(&p, 250), grey);
    let half = greyscale(&p, 50);
    assert_ne!(half, p);
    assert!(!half.accent.is_grey());
    // Same relative luminance, so the WCAG ratios hold.
    for (name, c) in p.roles() {
        let (a, b) = (luminance(c), luminance(grey.get(name).unwrap()));
        assert!((a - b).abs() < 0.005, "{name} {a} {b}");
    }
    assert_eq!(parse_level(" 40 "), Some(40));
    assert_eq!(parse_level("72.6%"), Some(73));
    assert_eq!(parse_level("-5"), Some(0));
    assert_eq!(parse_level("1000"), Some(100));
    assert_eq!(parse_level("grey"), None);
    assert_eq!(level_of(f64::NAN), None);
}

#[test]
fn hex_parsing() {
    assert_eq!(Rgb::parse("#1B1d22"), Some(Rgb(0x1b, 0x1d, 0x22)));
    assert_eq!(Rgb(0x1b, 0x1d, 0x22).hex(), "#1b1d22");
    for bad in ["1b1d22", "#1b1d2", "#1b1d2g", "#1b1d22ff", "#ééé", ""] {
        assert_eq!(Rgb::parse(bad), None, "{bad}");
    }
}

#[test]
fn parse_errors_name_the_file_and_the_key() {
    let err = |text: &str| format::parse(text, "my.json").unwrap_err();
    let e = err(&doc("x", Some("dark"), r##""glitter":"#ffffff""##));
    assert!(
        e.starts_with("my.json: colors.glitter: unknown role"),
        "{e}"
    );
    let e = err(&doc("x", Some("dark"), r##""bg":"#fff""##));
    assert_eq!(
        e,
        r##"my.json: colors.bg: "#fff" is not a "#rrggbb" colour"##
    );
    let e = err(&doc("x", Some("dark"), r#""bg":7"#));
    assert!(e.starts_with("my.json: colors.bg: 7 is not"), "{e}");
    let e = err(&doc("x", None, "").replace(r#""version":1"#, r#""version":2"#));
    assert!(e.contains("version: 2 is not supported"), "{e}");
    let e = err(&doc("Bad Id", None, ""));
    assert!(e.starts_with("my.json: id: \"Bad Id\""), "{e}");
    let e = err(&doc("x", None, "").replace(r#""kind":"dark""#, r#""kind":"dim""#));
    assert!(e.starts_with("my.json: unknown variant `dim`"), "{e}");
    let e = err(r#"{"version":1,"id":"x","name":"x","kind":"dark","colours":{}}"#);
    assert!(e.contains("unknown field `colours`"), "{e}");
    assert!(err("{").starts_with("my.json: "));
    assert!(err("").starts_with("my.json: "));
}

#[test]
fn extends_chains_cycles_and_missing_bases() {
    let docs: Vec<format::Doc> = [
        doc("a", Some("b"), r##""accent":"#00ff00""##),
        doc("b", Some("dark"), r##""bg":"#000000""##),
        doc("c", Some("d"), ""),
        doc("d", Some("c"), ""),
        doc("e", Some("nowhere"), ""),
        doc("f", None, r##""bg":"#000000""##),
        doc("g", Some("f"), ""),
    ]
    .iter()
    .map(|text| format::parse(text, "t.json").unwrap())
    .collect();
    let (themes, errors) = format::resolve(&docs, builtins());
    let ids: Vec<&str> = themes.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
    let dark = by_id("dark").palette;
    let a = &themes[0].palette;
    assert_eq!(
        (a.accent, a.bg, a.text),
        (Rgb(0, 255, 0), Rgb(0, 0, 0), dark.text)
    );
    assert_eq!(errors.len(), 5, "{errors:#?}");
    assert_eq!(errors[0], "t.json: extends cycle: c -> d -> c");
    assert_eq!(errors[1], "t.json: extends cycle: d -> c -> d");
    assert_eq!(errors[2], "t.json: extends: no theme \"nowhere\"");
    assert!(
        errors[3].starts_with("t.json: colors: missing bg_end, surface"),
        "{}",
        errors[3]
    );
    assert!(
        errors[4].starts_with("t.json: extends \"f\": colors: missing"),
        "{}",
        errors[4]
    );
}

#[test]
fn contrast_check_lists_the_failing_pairs() {
    assert!(check(&by_id("dark").palette).is_empty());
    let mut p = by_id("dark").palette;
    p.text = p.bg;
    let failures = check(&p);
    assert_eq!(
        failures.len(),
        11,
        "text fails on every background, tinted too"
    );
    assert_eq!(
        (failures[9].bg, failures[9].tint),
        ("code", Some(("success", 0.12)))
    );
    let tinted = failures[10].to_string();
    assert!(tinted.starts_with("text on code+error@0.12: "), "{tinted}");
    let first = &failures[0];
    assert_eq!((first.fg, first.bg), ("text", "bg"));
    assert!((first.ratio - 1.0).abs() < 1e-9);
    assert_eq!(
        first.to_string(),
        "text on bg: 1.00:1 (needs 7:1), APCA Lc 0.0 (needs 75)"
    );
    p.text = by_id("dark").palette.text;
    p.border = p.surface;
    let f = check(&p);
    assert_eq!(f.len(), 4);
    assert!(f[0].to_string().starts_with("border on bg: ") && !f[0].to_string().contains("Lc"));
}

#[test]
fn blend_mixes_each_channel_and_rounds() {
    let (black, white) = (Rgb(0, 0, 0), Rgb(255, 255, 255));
    assert_eq!(blend(black, white, 0.12), Rgb(31, 31, 31));
    assert_eq!(
        blend(Rgb(100, 200, 10), Rgb(200, 0, 10), 0.5),
        Rgb(150, 100, 10)
    );
    assert_eq!(blend(white, black, 0.0), white);
    assert_eq!(TINTS.len(), 10);
}

/// The tinted pairs: cargo test -p fast-harness-core tint_report -- --ignored --nocapture.
#[test]
#[ignore]
fn tint_report() {
    for theme in builtins() {
        for level in LEVELS {
            let p = theme.shown(level);
            let cells: Vec<String> = TINTS
                .iter()
                .filter_map(|t| {
                    let bg = blend(p.get(t.bg)?, p.get(t.tint)?, t.alpha);
                    let fg = p.get(t.fg)?;
                    let (r, lc) = (contrast_ratio(fg, bg), apca_lc(fg, bg).abs());
                    Some(format!("{}/{} {r:.2} Lc {lc:.0}", t.fg, t.tint))
                })
                .collect();
            println!("{:<10} grey {level:>3}: {}", theme.id, cells.join(" | "));
        }
    }
}

#[test]
fn user_themes_load_from_the_themes_folder() {
    let dir = TempDir::new("user");
    dir.write(
        "b-ocean.json",
        &doc("ocean", Some("aurora"), r##""accent":"#40e0ff""##),
    );
    dir.write("a-mine.json", &doc("mine", Some("ocean"), ""));
    dir.write("c-bad.json", &doc("bad", Some("dark"), r##""bg":"#12""##));
    dir.write("d-dark.json", &doc("dark", Some("light"), ""));
    dir.write("e-again.json", &doc("ocean", Some("dark"), ""));
    dir.write(
        "f-low.json",
        &doc("low", Some("dark"), r##""dim":"#30343c""##),
    );
    dir.write("notes.txt", "not a theme");
    let set = ThemeSet::load(&dir.0);
    let ids: Vec<&str> = set.themes().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(&ids[..8], IDS);
    assert_eq!(&ids[8..], ["mine", "ocean", "low"]);
    let ocean = set.get("ocean");
    assert_eq!(ocean.palette.accent, Rgb(0x40, 0xe0, 0xff));
    assert_eq!(ocean.palette.bg, by_id("aurora").palette.bg);
    assert!(ocean.file.as_ref().unwrap().ends_with("b-ocean.json") && !ocean.is_builtin());
    assert_eq!(set.get("mine").palette, ocean.palette);
    assert_eq!(
        set.get("dark"),
        by_id("dark"),
        "a built-in id never gets replaced"
    );
    let notes = set.notes().join("\n");
    assert_eq!(set.notes().len(), 4, "{notes}");
    assert!(notes.contains("c-bad.json: colors.bg: \"#12\""), "{notes}");
    assert!(notes.contains("d-dark.json: skipped: \"dark\" is a built-in theme id"));
    assert!(notes.contains("e-again.json: skipped: id \"ocean\" is already used by"));
    assert!(notes.contains("f-low.json: theme \"low\" fails the contrast check: dim on bg: "));
}

#[test]
fn a_missing_themes_folder_gives_the_builtins() {
    let dir = TempDir::new("none");
    std::fs::remove_dir_all(dir.0.join(format::THEMES_DIR)).unwrap();
    let set = ThemeSet::load(&dir.0);
    assert_eq!(set, ThemeSet::builtin());
    assert!(set.notes().is_empty());
}

#[test]
fn lookup_and_cycle() {
    let set = ThemeSet::builtin();
    assert_eq!(set.get(" light ").id, "light");
    assert_eq!(set.get("nope").id, DEFAULT_THEME);
    assert_eq!(set.get("greyscale-light").id, "light");
    assert_eq!(by_id("greyscale-dark").id, "dark");
    assert_eq!(legacy_greyscale("greyscale-dark"), Some("dark"));
    assert_eq!(legacy_greyscale("dark"), None);
    assert_eq!(set.next("dark").id, "light");
    assert_eq!(set.next("citrus").id, "dark");
    assert_eq!(set.position("sunset"), 4);
    assert_eq!(set.at(7).map(|t| t.id.as_str()), Some("citrus"));
    assert!(set.at(8).is_none());
    assert_eq!(ROLES.len(), 22);
    assert_eq!(by_id("dark").palette.roles().len(), ROLES.len());
}

/// Design helper: pushes each failing foreground away from its backgrounds
/// until the check passes with a small margin at every level, and prints
/// `id role #rrggbb` for the roles it moved.
#[test]
#[ignore]
fn palette_tune() {
    let fails = |p: &Palette, role: &str| {
        LEVELS.iter().any(|&level| {
            check(&greyscale(p, level))
                .iter()
                .any(|f| (f.fg == role || (f.bg == role && f.fg == "on_accent")) && f.bg != "x")
        }) || REQUIREMENTS.iter().filter(|r| r.fg == role).any(|r| {
            r.bgs.iter().any(|bg| {
                let (a, b) = (p.get(role).unwrap(), p.get(bg).unwrap());
                contrast_ratio(a, b) < r.min_ratio + 0.05
                    || (r.min_lc > 0.0 && apca_lc(a, b).abs() < r.min_lc + 1.0)
            })
        })
    };
    for theme in builtins() {
        let mut p = theme.palette;
        for role in [
            "text", "dim", "accent", "accent2", "error", "warning", "success", "info", "border",
        ] {
            let base = p.get(role).unwrap();
            let mut t = 0.0;
            while fails(&p, role) && t < 1.0 {
                t += 0.01;
                let push = |v: u8| {
                    let v = f64::from(v);
                    (if theme.dark {
                        v + (255.0 - v) * t
                    } else {
                        v * (1.0 - t)
                    })
                    .round() as u8
                };
                p.set(role, Rgb(push(base.0), push(base.1), push(base.2)));
            }
            if t > 0.0 {
                println!("TUNE {} {role} {}", theme.id, p.get(role).unwrap().hex());
            }
        }
    }
}
