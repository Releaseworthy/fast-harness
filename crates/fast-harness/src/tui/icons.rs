//! The marks of the TUI in two sets: Nerd Font icons (setting `icons`
//! `"nerd"`, the default) and plain Unicode marks (`"plain"`). Every mark is
//! one cell wide, so layouts are the same in both sets. Code points are from
//! Nerd Fonts v3.5.1 (`md-*` Material Design, `cod-*` Codicons).

/// One set of marks; an empty mark is not drawn (see [`lead`]).
#[derive(Debug, PartialEq, Eq)]
pub struct Icons {
    pub id: &'static str,
    /// The app mark in the header.
    pub app: &'static str,
    pub chat: &'static str,
    pub trace: &'static str,
    /// The user's prompt and the highlighted row of a list.
    pub pointer: &'static str,
    pub ok: &'static str,
    pub error: &'static str,
    pub info: &'static str,
    pub retry: &'static str,
    pub switch: &'static str,
    pub thinking: &'static str,
    pub tool: &'static str,
    pub response: &'static str,
    /// A tool call waiting for its result.
    pub pending: &'static str,
    /// A call the approval gate ended before it ran (PLAN.md D64).
    pub not_run: &'static str,
    /// The active account.
    pub account: &'static str,
    /// The "more below" badge.
    pub more: &'static str,
    /// Before the model and effort of the input box.
    pub settings: &'static str,
}

pub const NERD: Icons = Icons {
    id: "nerd",
    app: "\u{F140B}",     // md-lightning_bolt
    chat: "\u{EAC7}",     // cod-comment_discussion
    trace: "\u{EB31}",    // cod-pulse
    pointer: "\u{EAB6}",  // cod-chevron_right
    ok: "\u{EAB2}",       // cod-check
    error: "\u{EA87}",    // cod-error
    info: "\u{EA74}",     // cod-info
    retry: "\u{EA77}",    // cod-sync
    switch: "\u{EB99}",   // cod-account
    thinking: "\u{EA61}", // cod-lightbulb
    tool: "\u{EB6D}",     // cod-tools
    response: "\u{EB08}", // cod-hubot
    pending: "\u{EA82}",  // cod-history
    not_run: "\u{EABD}",  // cod-circle_slash
    account: "\u{EB99}",  // cod-account
    more: "\u{EAB4}",     // cod-chevron_down
    settings: "\u{EB51}", // cod-settings_gear
};

pub const PLAIN: Icons = Icons {
    id: "plain",
    app: "",
    chat: "",
    trace: "",
    pointer: "›",
    ok: "✓",
    error: "✗",
    info: "",
    retry: "↻",
    switch: "⇄",
    thinking: "✻",
    tool: "▸",
    response: "◆",
    pending: "○",
    not_run: "⊘",
    account: "*",
    more: "↓",
    settings: "",
};

/// Both sets, default first.
#[cfg(test)]
pub const ICONS: [&Icons; 2] = [&NERD, &PLAIN];

/// The set named `id`; anything but `plain` is the Nerd Font set.
pub fn by_id(id: &str) -> &'static Icons {
    if id.trim().eq_ignore_ascii_case("plain") {
        &PLAIN
    } else {
        &NERD
    }
}

/// `mark` and a space, or nothing for an empty mark.
pub fn lead(mark: &str) -> String {
    if mark.is_empty() {
        String::new()
    } else {
        format!("{mark} ")
    }
}

#[cfg(test)]
impl Icons {
    /// Every mark of the set, for the width tests.
    pub fn all(&self) -> [&'static str; 17] {
        [
            self.app,
            self.chat,
            self.trace,
            self.pointer,
            self.ok,
            self.error,
            self.info,
            self.retry,
            self.switch,
            self.thinking,
            self.tool,
            self.response,
            self.pending,
            self.not_run,
            self.account,
            self.more,
            self.settings,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    #[test]
    fn every_mark_is_one_cell_wide() {
        for icons in ICONS {
            for mark in icons.all() {
                let width = Span::raw(mark).width();
                // Plain marks may be empty (not drawn); nothing is two cells.
                assert!(width == 1 || (width == 0 && mark.is_empty()), "{mark:?}");
            }
        }
        assert!(NERD.all().iter().all(|m| !m.is_empty()));
    }

    #[test]
    fn plain_is_chosen_by_name_and_nerd_is_the_default() {
        assert_eq!(by_id("plain").id, "plain");
        assert_eq!(by_id(" Plain ").id, "plain");
        assert_eq!(by_id("nerd").id, "nerd");
        assert_eq!(by_id("other").id, "nerd");
        assert_eq!(lead(""), "");
        assert_eq!(lead("✓"), "✓ ");
    }
}
