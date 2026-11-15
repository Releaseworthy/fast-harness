//! Repeated `grep` and `glob` calls: the same arguments giving the same
//! result as an earlier call of the session answer `[unchanged since call
//! <id>]`, like `read` does, instead of the whole list again. A changed
//! result comes back in full and is remembered in place of the old one.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use serde_json::Value;

use super::ToolCtx;

/// The searches a session returned: key, hash of the result, call id.
#[derive(Debug, Default)]
pub struct Searches(Mutex<Vec<(String, u64, String)>>);

impl Searches {
    /// The call that returned `text` for `key` last, if any; otherwise
    /// remembers `call_id` as the one that did.
    fn seen(&self, key: String, text: &str, call_id: &str) -> Option<String> {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        let mut list = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match list.iter_mut().find(|(known, _, _)| *known == key) {
            Some((_, old, call)) if *old == hash => return Some(call.clone()),
            Some(entry) => *entry = (key, hash, call_id.to_string()),
            None => list.push((key, hash, call_id.to_string())),
        }
        None
    }

    /// Forgets every search: after a compaction the model no longer sees
    /// the earlier results.
    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// `text`, the result of `tool` for `args`, or the unchanged note when an
/// earlier call of the session returned the same (`context.dedupeReads`).
pub(crate) fn dedupe(tool: &str, args: &Value, text: String, ctx: &ToolCtx<'_>) -> String {
    let Some(session) = ctx.session.filter(|_| ctx.dedupe_reads) else {
        return text;
    };
    if ctx.call_id.is_empty() {
        return text;
    }
    let key = format!("{tool}\n{}\n{args}", ctx.cwd.display());
    match session.searches().seen(key, &text, ctx.call_id) {
        Some(call) => format!("[unchanged since call {call}]"),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::Searches;

    #[test]
    fn the_same_result_names_the_call_and_a_change_resets_it() {
        let searches = Searches::default();
        assert_eq!(searches.seen("k".into(), "a", "c1"), None);
        assert_eq!(searches.seen("k".into(), "a", "c2").as_deref(), Some("c1"));
        assert_eq!(searches.seen("other".into(), "a", "c3"), None);
        assert_eq!(searches.seen("k".into(), "b", "c4"), None);
        assert_eq!(searches.seen("k".into(), "b", "c5").as_deref(), Some("c4"));
        searches.clear();
        assert_eq!(searches.seen("k".into(), "b", "c6"), None);
    }
}
