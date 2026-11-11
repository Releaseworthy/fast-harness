//! The data folder move of the rename (PLAN.md D22): `~/.nh` becomes `~/.fast-harness` once.

use std::fs;
use std::path::PathBuf;

use fast_harness_core::platform::adopt_old_home;

fn fresh(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fast-harness-home-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_old_folder_alone_moves_to_the_new_name() {
    let home = fresh("move");
    fs::create_dir_all(home.join(".nh").join("sessions")).unwrap();
    fs::write(home.join(".nh").join("config.json"), "{}").unwrap();
    let got = adopt_old_home(&home);
    assert_eq!(got, home.join(".fast-harness"));
    assert_eq!(fs::read_to_string(got.join("config.json")).unwrap(), "{}");
    assert!(got.join("sessions").is_dir());
    assert!(!home.join(".nh").exists());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn an_existing_new_folder_wins_and_the_old_one_stays() {
    let home = fresh("both");
    fs::create_dir_all(home.join(".nh")).unwrap();
    fs::create_dir_all(home.join(".fast-harness")).unwrap();
    assert_eq!(adopt_old_home(&home), home.join(".fast-harness"));
    assert!(home.join(".nh").is_dir());
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn nothing_there_gives_the_new_path_without_creating_it() {
    let home = fresh("none");
    assert_eq!(adopt_old_home(&home), home.join(".fast-harness"));
    assert!(!home.join(".fast-harness").exists());
    fs::remove_dir_all(&home).unwrap();
}
