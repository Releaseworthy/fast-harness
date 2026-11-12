use std::path::PathBuf;

use super::*;
use crate::config::layers::{Layers, Source, PROJECT_DIR};
use crate::config::trust;

fn key(text: &str) -> Key {
    text.parse().expect("a valid key")
}

fn dirs(name: &str) -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("fh-keys-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (home, project) = (base.join("home"), base.join("project"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(PROJECT_DIR)).unwrap();
    (base, project)
}

#[test]
fn parse_and_format_round_trip() {
    for text in [
        "ctrl+shift+k",
        "alt+enter",
        "f2",
        "shift+f6",
        "ctrl++",
        "ctrl+,",
        "q",
    ] {
        assert_eq!(key(text).to_string(), text);
    }
    assert_eq!(key("Shift+Ctrl+K").to_string(), "ctrl+shift+k");
    assert_eq!(key("Control+Return").to_string(), "ctrl+enter");
    assert_eq!(key("PgUp").to_string(), "pageup");
    assert_eq!(key("ctrl+p").label(), "Ctrl+P");
    assert_eq!(key("pagedown").label(), "PgDn");
    assert_eq!(key("l").label(), "l");
    for bad in ["", "ctrl+", "hyper+k", "f25", "ctrl+ctrl+k", "enterr"] {
        assert!(bad.parse::<Key>().is_err(), "{bad} parsed");
    }
    assert_eq!(
        Key::from_char('G', true, false, false, false),
        key("ctrl+shift+g")
    );
    assert_eq!(Key::from_char('?', false, false, true, false), key("?"));
}

#[test]
fn defaults_parse_and_have_no_conflicts() {
    for front in [Front::Tui, Front::Gui] {
        for def in front.actions() {
            for text in def.keys {
                assert!(text.parse::<Key>().is_ok(), "{}: {text}", def.name);
            }
        }
        let map = Keymap::defaults(front);
        assert_eq!(map.conflicts(), Vec::new(), "{front:?}");
    }
    let tui = Keymap::defaults(Front::Tui);
    assert_eq!(tui.lookup("chat", &key("enter"), false), Some("send"));
    assert_eq!(tui.lookup("chat", &key("enter"), true), Some("steer"));
    assert_eq!(tui.lookup("chat", &key("alt+enter"), true), Some("queue"));
    assert_eq!(
        tui.lookup("chat", &key("alt+enter"), false),
        Some("newline")
    );
    assert_eq!(tui.lookup("trace", &key("esc"), false), Some("back"));
    assert_eq!(tui.lookup("chat", &key("esc"), false), None);
}

#[test]
fn user_override_and_unbind() {
    let (base, _) = dirs("user");
    let home = base.join("home");
    let file = r#"{"version": 1, "tui": {"pick_model": ["ctrl+k", "f9"], "pause": null},
        "gui": {"new_session": "ctrl+shift+n"}}"#;
    std::fs::write(home.join(FILE_NAME), file).unwrap();
    let (keys, skipped) = load(&Layers::user_only(&home));
    assert!(
        skipped.is_empty() && keys.problems.is_empty(),
        "{:?}",
        keys.problems
    );
    assert_eq!(keys.tui.keys("pick_model"), [key("ctrl+k"), key("f9")]);
    assert_eq!(keys.tui.binding("pick_model").unwrap().source, Source::User);
    assert!(keys.tui.keys("pause").is_empty());
    assert_eq!(keys.tui.lookup("chat", &key("ctrl+g"), true), None);
    assert_eq!(
        keys.gui.lookup("window", &key("ctrl+shift+n"), false),
        Some("new_session")
    );
    assert_eq!(keys.tui.binding("send").unwrap().source, Source::Default);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn project_layer_only_when_trusted() {
    let (base, project) = dirs("project");
    let home = base.join("home");
    std::fs::write(home.join(FILE_NAME), r#"{"tui": {"pick_model": "f8"}}"#).unwrap();
    let project_file = project.join(PROJECT_DIR).join(FILE_NAME);
    std::fs::write(&project_file, r#"{"tui": {"pick_model": "f7"}}"#).unwrap();
    let (keys, _) = load(&Layers::find(&home, &project));
    assert_eq!(keys.tui.keys("pick_model"), [key("f8")]);
    trust::trust(&home, &project).unwrap();
    let (keys, _) = load(&Layers::find(&home, &project));
    assert_eq!(keys.tui.keys("pick_model"), [key("f7")]);
    assert_eq!(
        keys.tui.binding("pick_model").unwrap().source,
        Source::Project
    );
    assert_eq!(keys.files.len(), 2);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn unknown_actions_bad_keys_and_conflicts_are_reported() {
    let (base, _) = dirs("bad");
    let home = base.join("home");
    let file = r#"{"tui": {"fly": "f1", "send": "hyper+x", "pick_model": "ctrl+t"}, "web": {}}"#;
    std::fs::write(home.join(FILE_NAME), file).unwrap();
    let (keys, _) = load(&Layers::user_only(&home));
    let text = keys.problems.join("\n");
    assert!(text.contains("unknown action `fly`"), "{text}");
    assert!(
        text.contains("tui.send: unknown modifier `hyper`"),
        "{text}"
    );
    assert!(text.contains("unknown section `web`"), "{text}");
    assert!(
        text.contains("`ctrl+t` starts both toggle_thinking and pick_model"),
        "{text}"
    );
    assert_eq!(keys.tui.keys("send"), [key("enter")]);
    assert!(keys.note().is_some());
    std::fs::write(home.join(FILE_NAME), "{not json").unwrap();
    let (_, skipped) = load(&Layers::user_only(&home));
    assert_eq!(skipped.len(), 1);
    let _ = std::fs::remove_dir_all(&base);
}
