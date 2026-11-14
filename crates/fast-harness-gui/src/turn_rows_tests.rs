use std::collections::HashSet;

use fast_harness_core::view::Item;

use super::*;

fn items() -> Vec<Item> {
    let user = |t: &str| Item::User { text: t.into(), images: vec![] };
    let answer = |t: &str| Item::Assistant { turn: 1, step: 1, text: t.into(), thinking: String::new() };
    vec![user("one"), answer("Looking."), answer("Final answer\nmore"), user("two"), answer("Two.")]
}

#[test]
fn open_turns_draw_rails_and_a_folded_turn_shows_one_line() {
    let items = items();
    let words = vec![None; items.len()];
    let mut decos = vec![Deco::default(); items.len()];
    decorate(&items, &words, 0, &mut decos, &HashSet::new());
    assert!(!decos[0].turn_rail && decos[1].turn_rail && decos[2].turn_rail);
    assert_eq!((decos[4].turn_head, decos[4].turn_no), (4, 2));
    let closed: HashSet<usize> = [1].into();
    let mut decos = vec![Deco::default(); items.len()];
    decorate(&items, &words, 0, &mut decos, &closed);
    assert!(decos[1].turn_hidden && decos[2].turn_hidden && !decos[1].turn_rail);
    // The line sits on the turn's last row (no blank rows end the list).
    assert_eq!(decos[2].turn_line, "1 step · Final answer");
    assert!(decos[1].turn_line.is_empty() && decos[4].turn_rail);
    assert_eq!(head_of(&items, 2), Some(1));
}
