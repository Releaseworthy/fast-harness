//! `Runtime::parts` picks the provider and the credential from the model id
//! (PLAN.md D24). `antigravity/<id>` models moved to branch `antigravity`
//! (D53): such a value in the settings or a picker falls back to the default
//! model with a note instead of failing.

use std::path::PathBuf;

use fast_harness_core::app::Runtime;
use fast_harness_core::config::Config;
struct Home(PathBuf);

impl Home {
    fn new(tag: &str) -> Home {
        let dir =
            std::env::temp_dir().join(format!("fast-harness-routing-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Home(dir)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn parts_follow_the_model_id() {
    let home = Home::new("claude");
    let mut rt = Runtime::load_at(home.0.clone()).unwrap();
    rt.pool.add_api_key("fake-key-route", "route key").unwrap();
    rt.config.model = "claude-opus-5-5".into();
    let parts = rt.parts(&home.0).unwrap();
    assert_eq!(parts.provider.id(), "anthropic");
    assert!(rt.take_notes().is_empty());
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn a_picked_antigravity_model_falls_back_with_a_note() {
    let home = Home::new("retired-pick");
    let mut rt = Runtime::load_at(home.0.clone()).unwrap();
    rt.pool.add_api_key("fake-key-route", "route key").unwrap();
    rt.config.model = "antigravity/gemini-3-flash".into();
    let parts = rt.parts(&home.0).expect("a retired model must not fail");
    assert_eq!(parts.provider.id(), "anthropic");
    assert_eq!(parts.config.model, Config::default().model);
    let notes = rt.take_notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("antigravity/gemini-3-flash") && notes[0].contains("D53"));
}

#[test]
fn a_configured_antigravity_model_falls_back_on_load() {
    let home = Home::new("retired-config");
    std::fs::write(
        home.0.join("config.json"),
        r#"{"model":"antigravity/gemini-3-pro"}"#,
    )
    .unwrap();
    let rt = Runtime::load_at(home.0.clone()).expect("a retired model must not fail the load");
    assert_eq!(rt.config.model, Config::default().model);
    let notes = rt.take_notes();
    assert!(
        notes.iter().any(|n| n.contains("antigravity/gemini-3-pro")),
        "{notes:?}"
    );
}
