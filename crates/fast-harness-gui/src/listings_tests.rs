//! The sidebar listing state (D60): coalesced requests, stale results dropped.

use fast_harness_core::session::SessionSummary;

use super::{Arrival, Listings};

fn summary(id: &str) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        created_at: 0,
        title: String::new(),
        turns: 0,
        last_time: 0,
        unfinished: None,
        parent: None,
    }
}

#[test]
fn one_listing_runs_per_workspace_and_the_newest_request_wins() {
    let mut l = Listings::default();
    let first = l.ask("ws").expect("starts");
    assert_eq!(l.ask("ws"), None, "a second request waits");
    let newest = match l.arrive("ws", first, true) {
        Arrival::Again(ticket) => ticket,
        other => panic!("the older result is dropped and the newest starts: {other:?}"),
    };
    assert!(l.sessions("ws").is_none(), "nothing of the older result was stored");
    assert_eq!(l.ask("ws"), None, "the restarted worker still counts");
    // The restarted read is older than the request just made: asked again.
    let last = match l.arrive("ws", newest, true) {
        Arrival::Again(ticket) => ticket,
        other => panic!("{other:?}"),
    };
    assert_eq!(l.arrive("ws", last, true), Arrival::Apply);
    l.store("ws", vec![summary("a")]);
    assert_eq!(l.sessions("ws").map(<[_]>::len), Some(1));
}

#[test]
fn a_listing_for_a_collapsed_workspace_changes_nothing() {
    let mut l = Listings::default();
    let ticket = l.ask("ws").expect("starts");
    l.forget("ws");
    assert_eq!(l.arrive("ws", ticket, false), Arrival::Drop);
    assert!(l.sessions("ws").is_none());
    // Expanded again: a fresh worker starts.
    assert!(l.ask("ws").is_some());
}

#[test]
fn the_rows_shown_stay_until_a_newer_list_arrives() {
    let mut l = Listings::default();
    let ticket = l.ask("ws").expect("starts");
    assert_eq!(l.arrive("ws", ticket, true), Arrival::Apply);
    l.store("ws", vec![summary("a"), summary("b")]);
    let again = l.ask("ws").expect("a turn end asks again");
    assert_eq!(l.sessions("ws").map(<[_]>::len), Some(2), "kept while it reads");
    assert_eq!(l.arrive("ws", again, true), Arrival::Apply);
}

#[test]
fn a_collapse_while_a_listing_runs_waits_for_it_on_re_expand() {
    let mut l = Listings::default();
    let first = l.ask("ws").expect("starts");
    l.forget("ws");
    assert_eq!(l.ask("ws"), None, "the old worker still runs");
    assert!(matches!(l.arrive("ws", first, true), Arrival::Again(_)));
}
