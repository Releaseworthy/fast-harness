use super::*;

const REPLY: &str = "## Plan\n\nFirst **bold** and `code`.\n\n- one\n- two\n  1. nested\n\n```rust\nfn main() {}\n```\n\n> quoted\n> line\n\n---\n\n| Name | Size |\n|------|-----:|\n| a.rs | 12 |\n| `b` \\| c | 3 |\n";

#[test]
fn a_table_becomes_one_grid_block_with_a_header_row() {
    let blocks = blocks(REPLY);
    let tables: Vec<&Block> = blocks.iter().filter(|b| b.kind == Kind::Table).collect();
    assert_eq!(tables.len(), 1, "{blocks:#?}");
    let rows = &tables[0].rows;
    assert_eq!(rows[0], vec!["Name", "Size"]);
    assert_eq!(rows[1], vec!["a.rs", "12"]);
    assert_eq!(rows[2], vec!["`b` \\| c", "3"]);
    let shares = shares(rows);
    assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-4);
}

#[test]
fn headings_lists_code_quotes_and_rules_split_into_blocks() {
    let blocks = blocks(REPLY);
    let kinds: Vec<Kind> = blocks.iter().map(|b| b.kind).collect();
    use Kind::*;
    assert_eq!(kinds, vec![Heading, Para, Item, Item, Item, Code, Para, Rule, Table]);
    assert_eq!((blocks[0].text.as_str(), blocks[0].level), ("Plan", 2));
    assert_eq!(blocks[1].text, "First **bold** and `code`.");
    let items: Vec<(&str, &str, usize)> = blocks[2..5]
        .iter()
        .map(|b| (b.marker.as_str(), b.text.as_str(), b.indent))
        .collect();
    assert_eq!(items, vec![("•", "one", 1), ("•", "two", 1), ("1.", "nested", 2)]);
    assert_eq!(blocks[5].text, "fn main() {}");
    assert_eq!((blocks[6].text.as_str(), blocks[6].quote), ("quoted\nline", 1));
}

#[test]
fn a_reply_row_carries_its_blocks() {
    use fast_harness_core::view::Item;
    use slint::Model;
    let item = Item::Assistant {
        turn: 1,
        step: 1,
        text: REPLY.into(),
        thinking: String::new(),
    };
    let row = crate::rows::msg_row(&item);
    let kinds: Vec<String> = row.blocks.iter().map(|b| b.kind.to_string()).collect();
    assert_eq!(kinds, ["h", "p", "li", "li", "li", "code", "p", "rule", "table"]);
    let table = row.blocks.row_data(8).expect("table");
    assert_eq!(table.rows.row_count(), 3);
    let head = table.rows.row_data(0).expect("header");
    assert!(head.head && head.cells.row_count() == 2);
}
