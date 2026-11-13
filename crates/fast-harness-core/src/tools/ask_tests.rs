use serde_json::json;

use super::*;

#[test]
fn recommended_option_leads_and_loses_its_suffix() {
    let args = json!({"questions": [{"id": "db", "question": "Which database?",
        "options": [{"label": "MySQL"}, {"label": "Postgres (Recommended)", "description": "d"},
            {"label": "SQLite", "recommended": true}]}]});
    let questions = parse(&args).unwrap();
    let labels: Vec<_> = questions[0]
        .options
        .iter()
        .map(|o| o.label.as_str())
        .collect();
    assert_eq!(labels, ["Postgres", "MySQL", "SQLite"]);
    assert!(questions[0].options[0].recommended && questions[0].options[2].recommended);
    assert_eq!(questions[0].options[0].description.as_deref(), Some("d"));
    assert!(!questions[0].multi_select);
}

#[test]
fn bad_calls_say_what_to_fix() {
    assert!(parse(&json!({})).is_err());
    assert!(parse(&json!({"questions": []})).is_err());
    let five: Vec<_> = (0..5)
        .map(|i| json!({"id": i.to_string(), "question": "q"}))
        .collect();
    assert!(parse(&json!({"questions": five})).is_err());
    let same = json!({"questions": [{"id": "a", "question": "q"}, {"id": "a", "question": "r"}]});
    assert_eq!(parse(&same).unwrap_err(), "question ids must be unique");
    let seven: Vec<_> = (0..7).map(|i| json!({"label": i.to_string()})).collect();
    let many = json!({"questions": [{"id": "a", "question": "q", "options": seven}]});
    assert!(parse(&many).unwrap_err().contains("at most 6"));
}

#[test]
fn answers_list_labels_and_custom_text() {
    let args = json!({"questions": [{"id": "a", "question": "One?"},
        {"id": "b", "question": "Many?", "multi_select": true}, {"id": "c", "question": "C?"}]});
    let questions = parse(&args).unwrap();
    assert!(questions[1].multi_select);
    let answers = [
        QuestionAnswer::new("a", vec!["Yes".into()], None),
        QuestionAnswer::new("b", vec!["X".into(), "Y".into()], Some(" mine ".into())),
    ];
    let text = format_answers(&questions, &answers);
    assert_eq!(
        text,
        "The user answered:\n- a (One?): \"Yes\"\n- b (Many?): \"X\", \"Y\", custom answer: \
        \"mine\"\n- c (C?): (no answer)"
    );
}
