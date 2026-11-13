//! The shared picker lists and fetch (D59), for both voices.

use std::sync::mpsc;

use super::*;
use crate::llm::anthropic::builtin_models;

fn pair(id: &str, name: &str) -> Row {
    (id.into(), name.into())
}

fn info(id: &str, name: &str, efforts: &[&str], deprecated: bool) -> ModelInfo {
    let mut model = builtin_models().remove(0);
    model.id = id.into();
    model.display_name = name.into();
    model.efforts = efforts.iter().map(|e| e.to_string()).collect();
    model.deprecated = deprecated;
    model
}

/// `(id, ID)` models; a true flag marks one deprecated.
fn fake(ids: &[(&str, bool)]) -> Vec<ModelInfo> {
    let one = |(id, old): &(&str, bool)| info(id, &id.to_uppercase(), &[], *old);
    ids.iter().map(one).collect()
}

fn live_list() -> Vec<ModelInfo> {
    vec![
        info(
            "claude-new-9",
            "Claude New 9",
            &["low", "high", "ultra"],
            false,
        ),
        info("claude-opus-5-5", "Claude Opus 5.5", &["low", "max"], false),
        info("claude-old-1", "Claude Old 1", &[], true),
    ]
}

fn ids(rows: &[Row]) -> Vec<&str> {
    rows.iter().map(|r| r.0.as_str()).collect()
}

#[test]
fn labels_add_the_id_only_to_shared_names() {
    let table = [
        pair("opus-a", "Opus"),
        pair("opus-b", "Opus"),
        pair("sonnet", "Sonnet"),
    ];
    let list = labelled(&table, "sonnet");
    let labels: Vec<&str> = list.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(labels, ["Opus (opus-a)", "Opus (opus-b)", "Sonnet"]);
    let custom = labelled(&table, "my-model");
    assert_eq!(custom[0], pair("my-model", "my-model"));
    assert_eq!(custom.len(), 4);
    assert_eq!(labelled(&table, "").len(), 3);
}

#[test]
fn builtin_window_labels_are_unique() {
    let list = model_list(
        &Catalog::default(),
        &builtin_models(),
        "claude-opus-5-5",
        Voice::Window,
    );
    assert_eq!(list[0], (String::new(), "Claude".to_string()));
    assert_eq!(list[1].1, "Claude Opus 5.5");
    assert!(list.iter().skip(1).all(|row| !is_header(row)));
    let mut labels: Vec<&str> = list.iter().map(|(_, l)| l.as_str()).collect();
    assert!(labels.contains(&"Claude Opus 4.5 (claude-opus-4-5-20251101)"));
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), list.len());
}

#[test]
fn groups_by_provider_with_title_rows_in_both_voices() {
    let models = [
        info("claude-c", "Claude C", &[], false),
        info("claude-d", "Claude D", &[], false),
    ];
    let list = model_list(&Catalog::default(), &models, "claude-new", Voice::Window);
    let labels: Vec<&str> = list.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(labels, ["Claude", "claude-new", "Claude C", "Claude D"]);
    assert!(is_header(&list[0]));
    assert_eq!(
        model_list(&Catalog::default(), &models[..1], "claude-c", Voice::Window).len(),
        2
    );
    let term = model_list(&Catalog::default(), &models, "claude-new", Voice::Terminal);
    assert_eq!(ids(&term), ["", "claude-new", "claude-c", "claude-d"]);
    assert_eq!(term[1].1, "claude-new");
    assert_eq!(term[2].1, format!("{:<18} claude-c", "Claude C"));
}

#[test]
fn deprecated_models_are_marked_in_each_voice() {
    let models = fake(&[("new-a", false), ("old-b", true)]);
    let window = model_list(&Catalog::default(), &models, "old-b", Voice::Window);
    assert_eq!(window[2], pair("old-b", "OLD-B (deprecated)"));
    let term = model_list(
        &Catalog::default(),
        &live_list(),
        "claude-opus-5-5",
        Voice::Terminal,
    );
    assert_eq!(
        ids(&term),
        ["", "claude-new-9", "claude-opus-5-5", "claude-old-1"]
    );
    assert!(term[3].1.ends_with("  deprecated"));
    assert!(!term[1].1.contains("deprecated"));
}

#[test]
fn efforts_come_from_the_live_catalog_else_the_provider() {
    let live = live_list();
    assert_eq!(
        effort_list(&Catalog::default(), &live, "claude-new-9"),
        ["low", "high", "ultra"]
    );
    // An empty live list and an unlisted model fall back to `llm::efforts`.
    let known = Catalog::default().efforts("claude-opus-5-5");
    assert_eq!(
        effort_list(&Catalog::default(), &[], "claude-opus-5-5"),
        known
    );
    assert_eq!(
        effort_list(&Catalog::default(), &live, "claude-old-1"),
        Catalog::default().efforts("claude-old-1")
    );
}

#[test]
fn refit_keeps_a_level_the_model_takes() {
    let (effort, note) = refit(&Catalog::default(), "claude-opus-4-5", Some("max"));
    assert_eq!(effort.as_deref(), Some("high"));
    assert!(note.contains("does not take max"), "{note}");
    let low = refit(&Catalog::default(), "claude-opus-4-5", Some("low"));
    assert_eq!(low, (Some("low".into()), String::new()));
    assert_eq!(
        refit(&Catalog::default(), "claude-opus-4-5", None),
        (None, String::new())
    );
}

#[test]
fn a_refreshed_list_keeps_the_highlighted_value() {
    let before = model_list(
        &Catalog::default(),
        &builtin_models(),
        "claude-opus-5-5",
        Voice::Terminal,
    );
    let after = model_list(
        &Catalog::default(),
        &live_list(),
        "claude-opus-5-5",
        Voice::Terminal,
    );
    let at = before
        .iter()
        .position(|r| r.0 == "claude-opus-5-5")
        .unwrap();
    assert_eq!(kept_row(&before, at, &after), Some(2));
    // A title row or a value the new list lacks keeps nothing.
    assert_eq!(kept_row(&before, 0, &after), None);
    let gone = [pair("gone", "Gone")];
    assert_eq!(kept_row(&gone, 0, &after), None);
    assert_eq!(kept_row(&gone, 5, &after), None);
}

#[test]
fn merge_keeps_the_selection_and_refits_the_effort() {
    let merged = merge(
        &Catalog::default(),
        Ok(fake(&[("new-a", false)])),
        "mine",
        None,
        Voice::Window,
    );
    assert_eq!(merged.list.unwrap()[1], pair("mine", "mine"));
    assert_eq!(merged.note, "");
    let refitted = merge(
        &Catalog::default(),
        Ok(fake(&[("x", false)])),
        "claude-opus-4-5",
        Some("max"),
        Voice::Window,
    );
    assert_eq!(refitted.effort.as_deref(), Some("high"));
    assert!(
        refitted.note.contains("does not take max"),
        "{}",
        refitted.note
    );
}

#[test]
fn a_failure_keeps_the_list_and_notes_the_error_in_each_voice() {
    let merged = merge(
        &Catalog::default(),
        Err("no account".into()),
        "m",
        Some("low"),
        Voice::Window,
    );
    assert_eq!(merged.list, None);
    assert_eq!(merged.effort.as_deref(), Some("low"));
    assert_eq!(merged.note, "Could not fetch the model list: no account");
    let term = fetched(Err("no account".into()), Voice::Terminal);
    assert_eq!(term.unwrap_err(), "error: no account");
    assert_eq!(fetched(Ok(live_list()), Voice::Terminal).unwrap().len(), 3);
}

#[test]
fn the_fetch_thread_delivers_a_fake_result() {
    let (tx, rx) = mpsc::channel();
    let fetch = || Ok(fake(&[("live-1", false)]));
    let handle = spawn_fetch(fetch, move |got| tx.send(got).unwrap()).unwrap();
    assert_eq!(handle.thread().name(), Some("fast-harness-models"));
    handle.join().unwrap();
    let merged = merge(
        &Catalog::default(),
        rx.recv().unwrap(),
        "live-1",
        None,
        Voice::Window,
    );
    let header = (String::new(), "Claude".to_string());
    assert_eq!(merged.list.unwrap(), [header, pair("live-1", "LIVE-1")]);
}
