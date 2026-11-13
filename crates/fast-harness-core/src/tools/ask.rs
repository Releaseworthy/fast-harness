//! `ask_user_question`: the model asks the user one to four questions and
//! waits for the answers (PLAN.md D81). The tool's own `run` is the answer
//! when nobody can answer (print mode, a front end without the popup): it
//! refuses at once. With a front end that answers, the agent's tool runner
//! asks through the session's gate instead (`agent/approval/ask.rs`).

use serde_json::{json, Value};

use super::args::object_schema;
use super::{Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

pub const NAME: &str = "ask_user_question";
/// At most this many questions in one call.
pub const MAX_QUESTIONS: usize = 4;
/// At most this many options in one question (the custom answer is extra).
pub const MAX_OPTIONS: usize = 6;
/// The result when nobody can answer.
pub const NO_USER: &str = "no user is available to answer: decide yourself with the \
safest reasonable choice, say which one you picked and why, and go on";
/// The result when the user skipped the questions.
pub const SKIPPED: &str = "the user skipped these questions without answering: decide \
yourself or ask in your reply";
const RECOMMENDED: &str = "(Recommended)";

/// One option of a question.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuestionOption {
    /// Without a trailing "(Recommended)".
    pub label: String,
    pub description: Option<String>,
    pub recommended: bool,
}

impl QuestionOption {
    pub fn new(label: &str, recommended: bool) -> QuestionOption {
        QuestionOption {
            label: label.to_string(),
            description: None,
            recommended,
        }
    }
}

/// One question; the recommended option comes first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Question {
    pub id: String,
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

/// The user's answer to one question: chosen option labels and the
/// custom text ("Other").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuestionAnswer {
    pub id: String,
    pub chosen: Vec<String>,
    pub other: Option<String>,
}

impl QuestionAnswer {
    pub fn new(id: &str, chosen: Vec<String>, other: Option<String>) -> QuestionAnswer {
        QuestionAnswer {
            id: id.to_string(),
            chosen,
            other,
        }
    }
}

pub(crate) struct AskTool;

const DESCRIPTION: &str = "Ask the user one to four short questions and wait for the \
answers. Use it only when you cannot go on well without the user's choice: a real fork \
in the requirements, a preference, or a risky step to confirm; never for facts you can \
look up or for a plain confirmation of what you will do anyway. Give each question an \
`id`, the `question`, up to 6 `options` (end the best one's label with \"(Recommended)\" \
or set `recommended`) and `multi_select` when several may apply. The user may always type \
a custom answer instead. The result lists each id with the chosen labels and/or the \
custom text; it may say that nobody can answer: then decide yourself.";

impl Tool for AskTool {
    /// Not `Read`: the call blocks on the user, so it never runs alongside
    /// other calls (D54); the approval gate never asks for it (D64).
    fn effect(&self) -> Effect {
        Effect::Run
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec::new(NAME, DESCRIPTION, schema())
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        match parse(args) {
            Ok(_) => ToolOutput::err(NO_USER),
            Err(message) => ToolOutput::err(message),
        }
    }
}

fn schema() -> Value {
    let option = json!({"type": "object", "properties": {
        "label": {"type": "string", "description": "Short choice text; end it with \"(Recommended)\" for the best one."},
        "description": {"type": "string", "description": "One sentence on the trade-off."},
        "recommended": {"type": "boolean"}}, "required": ["label"]});
    let question = json!({"type": "object", "properties": {
        "id": {"type": "string", "description": "Stable id, echoed in the answer."},
        "header": {"type": "string", "description": "Optional heading of 1 to 3 words."},
        "question": {"type": "string"},
        "options": {"type": "array", "items": option, "maxItems": MAX_OPTIONS},
        "multi_select": {"type": "boolean", "description": "Several options may be chosen."}},
        "required": ["id", "question"]});
    object_schema(
        json!({"questions": {"type": "array", "items": question,
            "minItems": 1, "maxItems": MAX_QUESTIONS}}),
        &["questions"],
    )
}

fn text(value: &Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn parse_option(value: &Value) -> Result<QuestionOption, String> {
    let label = text(value, "label").ok_or("each option needs a non-empty `label`")?;
    let marked = label.ends_with(RECOMMENDED);
    let label = label.trim_end_matches(RECOMMENDED).trim_end().to_string();
    let flag = value.get("recommended").and_then(Value::as_bool) == Some(true);
    Ok(QuestionOption {
        label,
        description: text(value, "description"),
        recommended: marked || flag,
    })
}

fn parse_question(value: &Value) -> Result<Question, String> {
    let id = text(value, "id").ok_or("each question needs a non-empty `id`")?;
    let question = text(value, "question").ok_or("each question needs a `question` text")?;
    let raw = match value.get("options") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items.clone(),
        Some(_) => return Err("`options` must be an array".into()),
    };
    if raw.len() > MAX_OPTIONS {
        return Err(format!("at most {MAX_OPTIONS} options per question"));
    }
    let mut options = raw
        .iter()
        .map(parse_option)
        .collect::<Result<Vec<_>, _>>()?;
    // The first recommended option leads; the others keep their order.
    if let Some(at) = options.iter().position(|o| o.recommended) {
        let first = options.remove(at);
        options.insert(0, first);
    }
    Ok(Question {
        id,
        header: text(value, "header"),
        question,
        options,
        multi_select: value.get("multi_select").and_then(Value::as_bool) == Some(true),
    })
}

/// The questions of a call, or what the model must fix.
pub fn parse(args: &Value) -> Result<Vec<Question>, String> {
    let Some(items) = args.get("questions").and_then(Value::as_array) else {
        return Err("`questions` must be an array of 1 to 4 questions".into());
    };
    if items.is_empty() || items.len() > MAX_QUESTIONS {
        return Err(format!("ask 1 to {MAX_QUESTIONS} questions in one call"));
    }
    let questions = items
        .iter()
        .map(parse_question)
        .collect::<Result<Vec<_>, _>>()?;
    let mut ids: Vec<&str> = questions.iter().map(|q| q.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != questions.len() {
        return Err("question ids must be unique".into());
    }
    Ok(questions)
}

/// The text the model reads: one line per question.
pub fn format_answers(questions: &[Question], answers: &[QuestionAnswer]) -> String {
    let mut out = String::from("The user answered:");
    for question in questions {
        let answer = answers.iter().find(|a| a.id == question.id);
        let mut parts: Vec<String> = Vec::new();
        if let Some(answer) = answer {
            parts.extend(answer.chosen.iter().map(|label| format!("\"{label}\"")));
            let other = answer.other.as_deref().map(str::trim);
            if let Some(other) = other.filter(|o| !o.is_empty()) {
                parts.push(format!("custom answer: \"{other}\""));
            }
        }
        let shown = match parts.is_empty() {
            true => "(no answer)".to_string(),
            false => parts.join(", "),
        };
        out.push_str(&format!(
            "\n- {} ({}): {shown}",
            question.id, question.question
        ));
    }
    out
}

#[cfg(test)]
#[path = "ask_tests.rs"]
mod tests;
