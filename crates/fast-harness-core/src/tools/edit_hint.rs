//! Where a failed `edit`'s `old_string` probably is (PLAN.md D87): the
//! lines of the file most like it, numbered, so the model sees the
//! difference instead of reading the file again.

use super::read_text::show_line;

/// Below this likeness (0 to 1) nothing is suggested.
const MIN_LIKENESS: f64 = 0.5;

/// Lines shown of the nearest text at most.
const SHOWN_LINES: usize = 8;

/// Line pairs compared at most: a hint on a huge file is not worth it.
const MAX_PAIRS: usize = 1_000_000;

/// The note after "old_string not found": the nearest lines of `content`
/// to `old` with their numbers, or nothing when none is alike enough.
pub(super) fn nearest(content: &str, old: &str) -> String {
    let wanted: Vec<Vec<(char, char)>> = old.lines().map(bigrams).collect();
    let lines: Vec<&str> = content.lines().collect();
    let n = wanted.len().max(1);
    if lines.is_empty() || lines.len().saturating_mul(n) > MAX_PAIRS {
        return String::new();
    }
    let have: Vec<Vec<(char, char)>> = lines.iter().map(|l| bigrams(l)).collect();
    let mut best = (0.0, 0);
    for start in 0..lines.len() {
        let pairs = wanted.iter().zip(&have[start..]);
        let score = pairs.map(|(w, h)| likeness(w, h)).sum::<f64>() / n as f64;
        if score > best.0 {
            best = (score, start);
        }
    }
    let (score, start) = best;
    if score < MIN_LIKENESS {
        return String::new();
    }
    let end = (start + n).min(lines.len());
    let range = match end - start {
        1 => format!("line {}", start + 1),
        _ => format!("lines {}-{end}", start + 1),
    };
    let mut text = format!(
        "; the most similar text ({:.0}% alike) is at {range}:\n",
        score * 100.0
    );
    for (i, line) in lines[start..end].iter().enumerate().take(SHOWN_LINES) {
        text.push_str(&show_line((start + i + 1) as u64, line, 0, true));
    }
    if end - start > SHOWN_LINES {
        text.push_str(&format!("[{} more line(s)]\n", end - start - SHOWN_LINES));
    }
    text
}

/// The sorted character pairs of `line` without its outer whitespace.
fn bigrams(line: &str) -> Vec<(char, char)> {
    let chars: Vec<char> = line.trim().chars().collect();
    let mut pairs: Vec<(char, char)> = chars.windows(2).map(|w| (w[0], w[1])).collect();
    if pairs.is_empty() {
        pairs.extend(chars.first().map(|&c| (c, c)));
    }
    pairs.sort_unstable();
    pairs
}

/// How alike two lines are (0 to 1): the mean of the share of `wanted`'s
/// pairs found in `have` (a part of a line counts) and their Dice score.
fn likeness(wanted: &[(char, char)], have: &[(char, char)]) -> f64 {
    if wanted.is_empty() && have.is_empty() {
        return 1.0;
    }
    if wanted.is_empty() || have.is_empty() {
        return 0.0;
    }
    let (mut i, mut j, mut common) = (0, 0, 0usize);
    while i < wanted.len() && j < have.len() {
        match wanted[i].cmp(&have[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                common += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let share = common as f64 / wanted.len() as f64;
    let dice = 2.0 * common as f64 / (wanted.len() + have.len()) as f64;
    (share + dice) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_lines_come_numbered_and_unlike_text_gets_no_hint() {
        let content = "a = 1\nfn alpha(x: u32) {\n    x + 1\n}\n";
        let hint = nearest(content, "fn alpha(x: u64) {\n    x + 1\n}");
        assert!(
            hint.contains("at lines 2-4:\n2: fn alpha(x: u32) {\n"),
            "{hint}"
        );
        assert_eq!(nearest(content, "completely different words here"), "");
        assert!(nearest(content, "x + 2").contains("at line 3:"));
    }
}
