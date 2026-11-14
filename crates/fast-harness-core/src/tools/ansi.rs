//! Terminal escape sequences (ANSI CSI colour codes, OSC titles and links)
//! taken out of command output: the model cannot see colour and every
//! escape costs tokens.

use std::borrow::Cow;
use std::iter::Peekable;
use std::str::Chars;

const ESC: char = '\u{1b}';
/// The one-character (C1) form of `ESC [`.
const CSI: char = '\u{9b}';
/// The one-character (C1) string terminator.
const ST: char = '\u{9c}';
const BEL: char = '\u{7}';

/// `text` without its escape sequences; borrowed when it has none.
pub(crate) fn strip_ansi(text: &str) -> Cow<'_, str> {
    if !text.contains([ESC, CSI]) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ESC => escape(&mut chars),
            CSI => control(&mut chars),
            _ => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Skips what follows an `ESC`: a control sequence, a string (OSC, DCS,
/// PM, APC) or a short escape (intermediates then one final character).
fn escape(chars: &mut Peekable<Chars<'_>>) {
    match chars.next() {
        Some('[') => control(chars),
        Some(']' | 'P' | 'X' | '^' | '_') => string(chars),
        Some(' '..='/') => {
            while chars.next_if(|c| (' '..='/').contains(c)).is_some() {}
            chars.next_if(|c| ('0'..='~').contains(c));
        }
        _ => {}
    }
}

/// Skips the parameters, intermediates and final byte of a control
/// sequence; a malformed one ends at the first other character, kept.
fn control(chars: &mut Peekable<Chars<'_>>) {
    while chars.next_if(|c| (' '..='?').contains(c)).is_some() {}
    chars.next_if(|c| ('@'..='~').contains(c));
}

/// Skips a string up to BEL or ST; an unterminated one ends before the
/// line break, which stays.
fn string(chars: &mut Peekable<Chars<'_>>) {
    while let Some(c) = chars.next_if(|c| *c != '\n') {
        match c {
            BEL | ST => return,
            ESC => {
                chars.next_if_eq(&'\\');
                return;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn colour_codes_and_titles_go() {
        let warning = "\u{1b}[33;1mWARNING: careful\u{1b}[0m\n";
        assert_eq!(strip_ansi(warning), "WARNING: careful\n");
        let title = "\u{1b}]0;title\u{7}ok \u{1b}]8;;https://x\u{1b}\\link\u{1b}]8;;\u{1b}\\\n";
        assert_eq!(strip_ansi(title), "ok link\n");
        assert_eq!(strip_ansi("\u{9b}31mred\u{1b}(Bé\u{1b}=x"), "redéx");
    }

    #[test]
    fn plain_text_is_borrowed_and_broken_escapes_keep_the_line() {
        assert!(matches!(
            strip_ansi("plain é"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(strip_ansi("a\u{1b}]never ends\nnext"), "a\nnext");
        assert_eq!(strip_ansi("a\u{1b}[12\nb"), "a\nb");
        assert_eq!(strip_ansi("end\u{1b}"), "end");
    }
}
