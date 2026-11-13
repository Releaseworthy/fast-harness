//! Each session keeps its own composer draft (D65).

use super::{Draft, Drafts};

fn draft(text: &str, cursor: usize) -> Draft {
    Draft {
        text: text.into(),
        cursor,
    }
}

#[test]
fn a_to_b_to_a_brings_back_a_draft_with_its_cursor() {
    let mut drafts = Drafts::new(Some("a"));
    let shown_b = drafts.switch(draft("for a", 3), Some("b"));
    assert_eq!(shown_b, Some(Draft::default()), "b has no draft");
    assert_eq!(drafts.owner(), Some("b"));
    let shown_a = drafts.switch(draft("for b", 5), Some("a"));
    assert_eq!(shown_a, Some(draft("for a", 3)));
    assert_eq!(drafts.saved(Some("b")), Some(&draft("for b", 5)));
    assert_eq!(
        drafts.saved(Some("a")),
        None,
        "the owner's text is the composer's"
    );
}

#[test]
fn the_new_session_keeps_its_draft_after_switching_away() {
    let mut drafts = Drafts::new(None);
    let shown_a = drafts.switch(Draft::at_end("new text"), Some("a"));
    assert_eq!(shown_a, Some(Draft::default()));
    // New session pressed again: the old draft comes back.
    let shown_new = drafts.switch(Draft::default(), None);
    assert_eq!(shown_new, Some(Draft::at_end("new text")));
    assert_eq!(drafts.owner(), None);
}

#[test]
fn the_same_owner_keeps_the_composer_as_it_is() {
    let mut drafts = Drafts::new(Some("a"));
    assert_eq!(drafts.switch(draft("typing", 2), Some("a")), None);
    assert_eq!(drafts.saved(Some("a")), None);
    let mut fresh = Drafts::new(None);
    assert_eq!(fresh.switch(draft("typing", 2), None), None);
}

#[test]
fn an_empty_composer_clears_the_slot_it_leaves() {
    let mut drafts = Drafts::new(Some("a"));
    drafts.switch(draft("old", 3), Some("b"));
    drafts.switch(Draft::default(), Some("a"));
    // The draft of a was taken and a left empty: nothing comes back.
    drafts.switch(Draft::default(), Some("b"));
    assert_eq!(drafts.saved(Some("a")), None);
    assert_eq!(
        drafts.switch(Draft::default(), Some("a")),
        Some(Draft::default())
    );
}

#[test]
fn the_first_prompt_moves_the_new_session_to_its_id_and_clears_its_slot() {
    let mut drafts = Drafts::new(None);
    // Typed, sent (the composer empties), and the session is created.
    drafts.created("s1");
    assert_eq!(drafts.owner(), Some("s1"));
    drafts.switch(Draft::at_end("next for s1"), Some("a"));
    // New session: the sent text does not come back, the slot is empty.
    assert_eq!(
        drafts.switch(Draft::default(), None),
        Some(Draft::default())
    );
    assert_eq!(
        drafts.saved(Some("s1")),
        Some(&Draft::at_end("next for s1"))
    );
}

#[test]
fn created_leaves_a_session_owner_alone() {
    let mut drafts = Drafts::new(Some("a"));
    drafts.created("s1");
    assert_eq!(drafts.owner(), Some("a"));
}

#[test]
fn a_superseded_open_keeps_the_text_typed_for_it() {
    // A shown; B asked, then C asked before B arrived (D60): the text typed
    // while B loaded is B's, and the composer shows C's.
    let mut drafts = Drafts::new(Some("a"));
    drafts.switch(draft("for a", 5), Some("b"));
    let shown_c = drafts.switch(draft("for b", 5), Some("c"));
    assert_eq!(shown_c, Some(Draft::default()));
    assert_eq!(drafts.saved(Some("b")), Some(&draft("for b", 5)));
    assert_eq!(drafts.saved(Some("a")), Some(&draft("for a", 5)));
}
