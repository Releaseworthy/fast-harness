//! The user's own `~/.fast-harness` is the user layer, never a project, even
//! when `FAST_HARNESS_HOME` points elsewhere: a workspace under the user
//! profile (`%TEMP%` is inside it on Windows) walked up to it and showed
//! "project C:\Users\<user> is not trusted" (2026-11-13, real window).
//! One test in its own binary: it sets the user-home variable of this process.

use std::fs;
use std::path::{Path, PathBuf};

use fast_harness_core::config::layers::{project_root, Layers, PROJECT_DIR};

fn fresh(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("fh-{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn the_users_own_home_folder_is_never_a_project() {
    let user = fresh("user-home");
    let app_home = fresh("app-home");
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::set_var(key, &user);
    // The real user layer: config and a provider, as on the owner's machine.
    put(
        &user.join(PROJECT_DIR).join("config.json"),
        r#"{"model": "x/m"}"#,
    );
    put(
        &user.join(PROJECT_DIR).join("providers").join("p.json"),
        "{}",
    );
    let ws = user.join("AppData").join("Local").join("Temp").join("ws2");
    fs::create_dir_all(&ws).unwrap();

    assert_eq!(project_root(&app_home, &ws), None);
    let layers = Layers::find(&app_home, &ws);
    assert_eq!(layers.project, None);
    assert_eq!(layers.untrusted_note(), None);

    // A real project below the user folder is still found.
    let proj = user.join("code").join("proj");
    fs::create_dir_all(proj.join(PROJECT_DIR)).unwrap();
    let inner = proj.join("src");
    fs::create_dir_all(&inner).unwrap();
    let found = project_root(&app_home, &inner).unwrap();
    assert_eq!(
        fs::canonicalize(found).unwrap(),
        fs::canonicalize(&proj).unwrap()
    );

    let _ = fs::remove_dir_all(&user);
    let _ = fs::remove_dir_all(&app_home);
}
