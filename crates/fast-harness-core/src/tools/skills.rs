//! Skills (Agent Skills format, PLAN.md D43): `skills/<name>/SKILL.md` of
//! the user and trusted project folders. Only names reach the system prompt
//! ([`skills_line`]); `find_tools` matches a skill by name and description
//! and returns its body ([`body`]) as text with the folder of its other
//! files. A body block starts with the line `[skill <name>: <folder>]`, so
//! the loaded skills are rebuilt from the conversation ([`loaded`]).

use std::path::PathBuf;

use crate::config::index::front_matter;
use crate::types::{Block, Message, Role};

/// Most bytes of one skill body in a `find_tools` result.
pub const SKILL_BYTES: usize = 20 * 1024;
/// Most bytes of the skill names in the system prompt line.
pub const LINE_BYTES: usize = 1024;
/// Most skill bodies one `find_tools` call returns.
pub const MAX_SKILLS: usize = 3;
/// Starts the first line of a skill body block.
pub const MARK: &str = "[skill ";

/// One indexed skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The `SKILL.md` file.
    pub path: PathBuf,
}

/// `Skills: a, b, c (load one with find_tools)`, names only, at most
/// [`LINE_BYTES`] of names then `and n more`; empty without skills.
pub fn skills_line(names: &[&str]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let mut shown = String::new();
    let mut count = 0;
    for name in names {
        if shown.len() + name.len() + 2 > LINE_BYTES {
            break;
        }
        if count > 0 {
            shown.push_str(", ");
        }
        shown.push_str(name);
        count += 1;
    }
    let more = match names.len() - count {
        0 => String::new(),
        n => format!(" and {n} more"),
    };
    format!("Skills: {shown}{more} (load one with find_tools)")
}

/// The block `find_tools` returns for `skill`: the mark line naming its
/// folder, then the body after the front matter, cut at [`SKILL_BYTES`].
pub fn body(skill: &Skill) -> String {
    let folder = skill.path.parent().unwrap_or(&skill.path).display();
    let head = format!(
        "{MARK}{}: {folder}]\n(other files of the skill are in this folder)\n",
        skill.name
    );
    let text = match std::fs::read_to_string(&skill.path) {
        Ok(text) => text,
        Err(err) => return format!("{head}could not read {}: {err}", skill.path.display()),
    };
    let body = front_matter(&text).1.trim();
    if body.len() <= SKILL_BYTES {
        return format!("{head}{body}");
    }
    let mut end = SKILL_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{head}{}\n[skill body cut at 20 KB; read the rest from {}]",
        &body[..end],
        skill.path.display()
    )
}

/// `(name, folder)` of a mark line, `None` for other text.
pub fn parse_mark(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix(MARK)?.strip_suffix(']')?;
    rest.split_once(": ")
}

/// Every mark line with `prefix` in the tool results and user text of
/// `messages` (never the model's own text), in order, without duplicates.
pub fn marked_lines<'a>(messages: &'a [Message], prefix: &str) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for message in messages.iter().filter(|m| m.role != Role::Assistant) {
        for block in &message.content {
            let text = match block {
                Block::ToolResult { content, .. } => content.as_str(),
                Block::Text { text } => text.as_str(),
                _ => continue,
            };
            if !text.contains(prefix) {
                continue;
            }
            for line in text.lines().filter(|line| line.starts_with(prefix)) {
                if !out.contains(&line) {
                    out.push(line);
                }
            }
        }
    }
    out
}

/// The skills whose bodies the conversation holds, first loaded first.
pub fn loaded(messages: &[Message]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in marked_lines(messages, MARK) {
        if let Some((name, _)) = parse_mark(line) {
            if !out.iter().any(|known| known == name) {
                out.push(name.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_names_only_and_more() {
        assert_eq!(skills_line(&[]), "");
        assert_eq!(
            skills_line(&["a", "b"]),
            "Skills: a, b (load one with find_tools)"
        );
        let many: Vec<String> = (0..300).map(|i| format!("skill-{i:03}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        let line = skills_line(&names);
        assert!(line.len() < LINE_BYTES + 80, "{}", line.len());
        assert!(line.contains(" more (load one with find_tools)"));
        assert_eq!(
            parse_mark("[skill a-b: C:\\x\\a-b]"),
            Some(("a-b", "C:\\x\\a-b"))
        );
    }
}
