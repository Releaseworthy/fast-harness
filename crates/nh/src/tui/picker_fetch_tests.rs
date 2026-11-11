//! The live model list: a fetch on a thread refreshes the open picker.

use std::sync::mpsc;
use std::time::Duration;

use nh_core::llm::anthropic::ModelInfo;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::app::Msg;
use super::picker::{PickerKind, REFRESHING};
use super::ui::{Action, Ui};
use super::worker::{start_fetch, FetchJob};

fn model(id: &str, name: &str, efforts: &[&str], deprecated: bool) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        display_name: name.into(),
        context_window: 200_000,
        max_output: 64_000,
        reasoning: true,
        efforts: efforts.iter().map(|e| e.to_string()).collect(),
        deprecated,
        live: None,
    }
}

fn ctrl(ui: &mut Ui, letter: char) -> Action {
    ui.key(KeyEvent::new(KeyCode::Char(letter), KeyModifiers::CONTROL))
}

/// Runs `job` through the real thread and channel, as the UI loop does.
fn run_fetch(ui: &mut Ui, job: FetchJob) {
    let (tx, rx) = mpsc::channel();
    start_fetch(job, tx).unwrap();
    ui.fetch_started();
    match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
        Msg::Models(result) => ui.models_fetched(result),
        _ => panic!("expected Msg::Models"),
    }
}

fn live_list() -> Vec<ModelInfo> {
    vec![
        model(
            "claude-new-9",
            "Claude New 9",
            &["low", "high", "ultra"],
            false,
        ),
        model("claude-opus-5-5", "Claude Opus 5.5", &["low", "max"], false),
        model("claude-old-1", "Claude Old 1", &[], true),
    ]
}

#[test]
fn ctrl_p_asks_for_a_fetch_and_shows_refreshing() {
    let mut ui = Ui::new("claude-opus-5-5".into(), None);
    assert_eq!(ctrl(&mut ui, 'p'), Action::FetchModels);
    ui.fetch_started();
    let picker = ui.picker.as_ref().unwrap();
    assert!(picker.title().contains(REFRESHING), "{}", picker.title());
    assert!(ui.fetching);
    // A picker opened while a fetch runs shows it too.
    ui.picker = None;
    ctrl(&mut ui, 'e');
    assert!(ui.picker.as_ref().unwrap().title().contains("refreshing"));
}

#[test]
fn a_fetched_list_updates_the_open_picker_and_keeps_the_highlight() {
    let mut ui = Ui::new("claude-opus-5-5".into(), None);
    ctrl(&mut ui, 'p');
    run_fetch(&mut ui, Box::new(|| Ok(live_list())));
    assert!(!ui.fetching);
    let picker = ui.picker.as_ref().unwrap();
    assert_eq!(picker.note, None);
    let ids: Vec<&str> = picker.items.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(ids, ["claude-new-9", "claude-opus-5-5", "claude-old-1"]);
    assert_eq!(picker.items[picker.selected].0, "claude-opus-5-5");
    assert!(picker.items[2].1.contains("deprecated"));
    assert!(!picker.items[0].1.contains("deprecated"));
}

#[test]
fn the_effort_picker_takes_the_live_levels() {
    let mut ui = Ui::new("claude-new-9".into(), Some("high".into()));
    assert_eq!(ctrl(&mut ui, 'e'), Action::FetchModels);
    run_fetch(&mut ui, Box::new(|| Ok(live_list())));
    let picker = ui.picker.as_ref().unwrap();
    assert_eq!(picker.kind, PickerKind::Effort);
    let items: Vec<&str> = picker.items.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(items, ["low", "high", "ultra"]);
    assert_eq!(picker.items[picker.selected].0, "high");
}

#[test]
fn a_failed_fetch_shows_the_error_and_keeps_the_list() {
    let mut ui = Ui::new("claude-opus-5-5".into(), None);
    ctrl(&mut ui, 'p');
    let before = ui.picker.as_ref().unwrap().items.clone();
    run_fetch(&mut ui, Box::new(|| Err("no account".into())));
    let picker = ui.picker.as_ref().unwrap();
    assert_eq!(picker.items, before);
    assert!(
        picker.title().contains("error: no account"),
        "{}",
        picker.title()
    );
    assert!(!ui.fetching);
}

#[test]
fn a_list_arriving_after_the_picker_closed_only_updates_the_catalog() {
    let mut ui = Ui::new("claude-opus-5-5".into(), None);
    run_fetch(&mut ui, Box::new(|| Ok(live_list())));
    assert!(ui.picker.is_none());
    assert_eq!(ui.catalog.len(), 3);
}
