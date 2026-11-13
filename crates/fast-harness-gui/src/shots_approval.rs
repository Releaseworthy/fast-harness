//! The approval prompt (D64) in the screenshot test: a subagent's long
//! command over the composer, with one more request waiting after it.

use std::path::Path;

use crate::{shots_png, ApprovalView, MainWindow};

pub fn view() -> ApprovalView {
    let text = "run: cargo test --workspace -- --test-threads=1 2>&1 | Tee-Object -FilePath target\\test.log; \
git status --short; Write-Output 'tests done, now checking the tree before the commit'";
    ApprovalView {
        id: "a1".into(),
        title: "Approval needed: shell wants to run a command".into(),
        text: text.into(),
        agent: "Asked by subagent review (s-42)".into(),
        more: "1 more waiting after this one".into(),
        once_key: "Alt+Y".into(),
        session_key: "Alt+A".into(),
        decline_key: "Alt+N".into(),
    }
}

/// `<theme>-approval.png`: the prompt on the chat page; cleared after.
pub fn shot(
    ui: &MainWindow,
    dir: &Path,
    theme: &str,
    size: (u32, u32),
    render: impl Fn() -> Vec<u8>,
) {
    ui.set_page("chat".into());
    ui.set_approval(view());
    let png = shots_png::encode(size.0, size.1, &render());
    std::fs::write(dir.join(format!("{theme}-approval.png")), png).expect("write png");
    ui.set_approval(ApprovalView::default());
}
