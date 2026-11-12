//! Panels (PLAN.md D46) with small scripted panel processes: cmd scripts on
//! Windows, sh elsewhere.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fast_harness_core::config::layers::Layers;
use fast_harness_core::config::trust;
use fast_harness_core::panels::{self, Host, Panel, Panels, Wake};
use serde_json::{json, Value};

const VIEW: &str = r#"{"type":"view","root":{"type":"column","children":[{"type":"button","id":"go","text":"Go"},{"type":"gauge","id":"g"}]}}"#;
const PATCH: &str =
    r#"{"type":"patch","id":"go","node":{"type":"text","id":"go","text":"clicked"}}"#;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-panels-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("panels")).unwrap();
    dir
}

/// The script body for this OS and the panel file running it.
fn panel_file(home: &Path, id: &str, cmd: &str, sh: &str) {
    let dir = home.join("panels");
    let (command, args) = if cfg!(windows) {
        let path = dir.join(format!("{id}.cmd"));
        std::fs::write(&path, cmd).unwrap();
        (
            "cmd".to_string(),
            vec!["/c".to_string(), path.display().to_string()],
        )
    } else {
        let path = dir.join(format!("{id}.sh"));
        std::fs::write(&path, sh).unwrap();
        ("sh".to_string(), vec![path.display().to_string()])
    };
    let file = json!({"id": id, "title": "Demo", "where": "tab", "command": command, "args": args});
    std::fs::write(dir.join(format!("{id}.json")), file.to_string()).unwrap();
}

fn click_script(home: &Path) {
    let cmd = format!(
        "@echo off\r\necho x>\"%~dp0started.txt\"\r\nset /p init=\r\necho {VIEW}\r\nset /p ev=\r\n>\"%~dp0event.txt\" echo(%ev%\r\necho {PATCH}\r\nset /p rest=\r\n"
    );
    let sh = format!(
        "echo x > \"$FAST_HARNESS_PANEL_DIR/started.txt\"\nread init\necho '{VIEW}'\nread ev\nprintf '%s\\n' \"$ev\" > \"$FAST_HARNESS_PANEL_DIR/event.txt\"\necho '{PATCH}'\nread rest\n"
    );
    panel_file(home, "demo", &cmd, &sh);
}

fn wake() -> Wake {
    Arc::new(|| {})
}

fn host(dir: &Path) -> Host<'_> {
    Host::new(dir, "s1", true)
}

/// Polls `panel` until `done` holds (10 s at most).
fn until(panel: &mut Panel, done: impl Fn(&Panel) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        panel.poll();
        if done(panel) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_scripted_panel_renders_and_gets_a_click() {
    let home = temp("click");
    click_script(&home);
    let (files, loaded, skipped) = panels::load(&Layers::user_only(&home));
    assert_eq!((files.len(), loaded.len(), skipped.len()), (1, 1, 0));
    let mut all = Panels::new(&files);
    assert!(!all.list[0].started(), "no process before the panel opens");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!home.join("panels/started.txt").exists());
    let panel = all.get_mut("demo").unwrap();
    panel.open(host(&home), wake());
    assert!(
        until(panel, |p| p.rows.len() == 2),
        "view: {}",
        panel.status
    );
    assert_eq!(panel.rows[0].cells[0].kind, "button");
    assert_eq!(panel.rows[1].cells[0].text, "unknown widget gauge");
    panel.send("click", "go", Value::Null);
    assert!(
        until(panel, |p| p.rows[0].cells[0].text == "clicked"),
        "{}",
        panel.status
    );
    let event = std::fs::read_to_string(home.join("panels/event.txt")).unwrap();
    let event: Value = serde_json::from_str(event.trim()).unwrap();
    assert_eq!(event, json!({"type": "click", "id": "go", "value": null}));
    all.stop_all();
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_dying_panel_shows_its_last_stderr_line() {
    let home = temp("dies");
    let cmd = "@echo off\r\necho first problem 1>&2\r\necho last words 1>&2\r\nexit /b 3\r\n";
    let sh = "echo first problem >&2\necho last words >&2\nexit 3\n";
    panel_file(&home, "dies", cmd, sh);
    let (files, _, _) = panels::load(&Layers::user_only(&home));
    let mut all = Panels::new(&files);
    let panel = all.get_mut("dies").unwrap();
    panel.open(host(&home), wake());
    assert!(until(panel, |p| p.ended), "{}", panel.status);
    assert!(panel.status.ends_with(": last words"), "{}", panel.status);
    assert!(panel.status.contains("exit code 3"), "{}", panel.status);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn untrusted_project_panels_do_not_load() {
    let home = temp("trust-home");
    let project = temp("trust-project");
    std::fs::create_dir_all(project.join(".fast-harness/panels")).unwrap();
    let file = json!({"id": "p", "command": "x"});
    std::fs::write(
        project.join(".fast-harness/panels/p.json"),
        file.to_string(),
    )
    .unwrap();
    let (files, _, _) = panels::load(&Layers::find(&home, &project));
    assert!(files.is_empty(), "an untrusted project's panels stay out");
    trust::trust(&home, &project).unwrap();
    let (files, _, _) = panels::load(&Layers::find(&home, &project));
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].place, panels::Place::Tab);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&project);
}
