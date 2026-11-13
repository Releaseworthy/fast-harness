use fast_harness_core::view::Item;

use super::*;

fn user(text: &str) -> Item {
    Item::User {
        text: text.into(),
        images: Vec::new(),
    }
}

fn reply(text: &str) -> Item {
    Item::Assistant {
        turn: 1,
        step: 1,
        text: text.into(),
        thinking: String::new(),
    }
}

fn note() -> Item {
    Item::Note {
        kind: fast_harness_core::view::NoteKind::Info,
        text: "Retrying".into(),
    }
}

#[test]
fn one_tick_per_user_turn_at_its_row() {
    let items = [
        note(),
        user("First question"),
        reply(""),
        reply("First answer."),
        reply("Final answer."),
        reply("  "),
        user("\n\n  Second question\nmore lines"),
        note(),
    ];
    let turns = turns(&items);
    let rows: Vec<(usize, usize)> = turns.iter().map(|t| (t.number, t.row)).collect();
    assert_eq!(rows, [(1, 1), (2, 6)]);
    assert_eq!(turns[0].prompt, "First question");
    // The last reply with words (the final answer), not the first step's.
    assert_eq!(turns[0].preview, "Final answer.");
    assert_eq!(turns[1].prompt, "Second question");
    assert_eq!(turns[1].preview, "", "no reply yet");
}

#[test]
fn elide_flattens_and_cuts_with_an_ellipsis() {
    assert_eq!(elide("a  b\n\nc", 10), "a b c");
    assert_eq!(elide("abcdefghij", 10), "abcdefghij");
    assert_eq!(elide("abcde fghijk", 8), "abcde f…");
    assert_eq!(elide("abcd   efgh", 6), "abcd…", "no space before the ellipsis");
    // Characters, not bytes.
    assert_eq!(elide("ưươơêê", 4), "ưươ…");
    let long = "word ".repeat(10_000);
    assert_eq!(elide(&long, PREVIEW_CHARS).chars().count(), PREVIEW_CHARS);
}

#[test]
fn the_prompt_is_one_line_of_bounded_length() {
    let text = format!("{}\nsecond line", "x".repeat(300));
    let turns = turns(&[user(&text)]);
    assert_eq!(turns[0].prompt.chars().count(), PROMPT_CHARS);
    assert!(turns[0].prompt.ends_with('…'));
    assert!(!turns[0].prompt.contains('\n'));
}

#[test]
fn sampling_keeps_the_first_and_last_and_stays_in_order() {
    let items: Vec<Item> = (0..200).map(|i| user(&format!("q{i}"))).collect();
    let all = turns(&items);
    let picked = sample(all.clone(), MAX_TICKS);
    assert_eq!(picked.len(), MAX_TICKS);
    assert_eq!(picked.first(), all.first());
    assert_eq!(picked.last(), all.last());
    assert!(picked.windows(2).all(|w| w[0].row < w[1].row));
    let few = sample(all[..5].to_vec(), MAX_TICKS);
    assert_eq!(few, all[..5]);
}
