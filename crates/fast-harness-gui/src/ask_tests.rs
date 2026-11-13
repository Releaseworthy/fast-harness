use super::*;
use fast_harness_core::events::{Question, QuestionOption};

fn model() -> ModelRc<AskQuestion> {
    let mut one = Question::default();
    one.id = "db".into();
    one.options = vec![QuestionOption::new("Postgres", true), QuestionOption::new("MySQL", false)];
    let mut many = one.clone();
    many.id = "os".into();
    many.multi_select = true;
    let mut request = QuestionRequest::default();
    request.questions = vec![one, many];
    ModelRc::new(VecModel::from(rows(&request)))
}

#[test]
fn single_multi_and_custom_answers_reach_the_reply() {
    let questions = model();
    toggle(&questions, 0, 1);
    toggle(&questions, 0, 0);
    toggle(&questions, 1, 0);
    toggle(&questions, 1, 1);
    other(&questions, 1, "BSD");
    let got = answers(&questions);
    assert_eq!(got[0], QuestionAnswer::new("db", vec!["Postgres".into()], None));
    let os = QuestionAnswer::new("os", vec!["Postgres".into(), "MySQL".into()], Some("BSD".into()));
    assert_eq!(got[1], os);
    other(&questions, 0, "Mine");
    let got = answers(&questions);
    assert_eq!(got[0], QuestionAnswer::new("db", Vec::new(), Some("Mine".into())), "typing chose Other");
}
