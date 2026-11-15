//! Which lines of a file changed between two versions, from per-line
//! hashes: the common start and end are cut off and the middle gets an LCS
//! when it is small enough, else it is one changed block.

use std::hash::{DefaultHasher, Hash, Hasher};

/// Largest middle (old lines times new lines) diffed line by line.
const LCS_CELLS: usize = 4_000_000;

/// Changed lines: `old_len` lines at `old` (0-based) became `new_len`
/// lines at `new`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hunk {
    pub old: usize,
    pub old_len: usize,
    pub new: usize,
    pub new_len: usize,
}

/// One hash per line of `text` (`str::lines`: `\r\n` and `\n` both end one).
pub(crate) fn line_hashes(text: &str) -> Vec<u64> {
    text.lines()
        .map(|line| {
            let mut hasher = DefaultHasher::new();
            line.hash(&mut hasher);
            hasher.finish()
        })
        .collect()
}

/// The hunks turning `old` into `new`, in order.
pub(crate) fn hunks(old: &[u64], new: &[u64]) -> Vec<Hunk> {
    let start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let (old_rest, new_rest) = (&old[start..], &new[start..]);
    let end = old_rest
        .iter()
        .rev()
        .zip(new_rest.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (a, b) = (
        &old_rest[..old_rest.len() - end],
        &new_rest[..new_rest.len() - end],
    );
    if a.is_empty() && b.is_empty() {
        return Vec::new();
    }
    let whole = Hunk {
        old: start,
        old_len: a.len(),
        new: start,
        new_len: b.len(),
    };
    if a.is_empty() || b.is_empty() || a.len().saturating_mul(b.len()) > LCS_CELLS {
        return vec![whole];
    }
    lcs_hunks(a, b, start)
}

/// The hunks between `a` and `b` (both starting at line `base`) from a
/// longest common subsequence table.
fn lcs_hunks(a: &[u64], b: &[u64], base: usize) -> Vec<Hunk> {
    let width = b.len() + 1;
    // table[i * width + j]: LCS length of a[i..] and b[j..].
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = match a[i] == b[j] {
                true => table[(i + 1) * width + j + 1] + 1,
                false => table[(i + 1) * width + j].max(table[i * width + j + 1]),
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    let mut open: Option<Hunk> = None;
    while i < a.len() || j < b.len() {
        let same = i < a.len() && j < b.len() && a[i] == b[j];
        if same {
            out.extend(open.take());
            (i, j) = (i + 1, j + 1);
            continue;
        }
        let hunk = open.get_or_insert(Hunk {
            old: base + i,
            old_len: 0,
            new: base + j,
            new_len: 0,
        });
        let drop_old =
            j == b.len() || (i < a.len() && table[(i + 1) * width + j] >= table[i * width + j + 1]);
        match drop_old {
            true => (hunk.old_len, i) = (hunk.old_len + 1, i + 1),
            false => (hunk.new_len, j) = (hunk.new_len + 1, j + 1),
        }
    }
    out.extend(open);
    out
}

/// Line `line` (1-based) of the old version in the new one: shifted by
/// the hunks before it; a line inside a hunk maps to the hunk's start.
pub(crate) fn old_to_new(hunks: &[Hunk], line: u64) -> u64 {
    map_line(hunks, line, |h| (h.old, h.old_len, h.new, h.new_len))
}

/// Line `line` (1-based) of the new version in the old one.
pub(crate) fn new_to_old(hunks: &[Hunk], line: u64) -> u64 {
    map_line(hunks, line, |h| (h.new, h.new_len, h.old, h.old_len))
}

fn map_line(
    hunks: &[Hunk],
    line: u64,
    sides: impl Fn(&Hunk) -> (usize, usize, usize, usize),
) -> u64 {
    if line == u64::MAX {
        return line;
    }
    let mut shift: i64 = 0;
    for hunk in hunks {
        let (from, from_len, to, to_len) = sides(hunk);
        let (from, from_len) = (from as u64 + 1, from_len as u64);
        if line < from {
            break;
        }
        if line < from + from_len {
            return to as u64 + 1;
        }
        shift += to_len as i64 - from_len as i64;
    }
    line.saturating_add_signed(shift).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(text: &str) -> Vec<u64> {
        line_hashes(text)
    }

    #[test]
    fn finds_separate_hunks_and_maps_lines_across_them() {
        let old = h("a\nb\nc\nd\ne\nf\n");
        let new = h("a\nB\nc\nd\nx\ny\ne\n");
        let hunks = hunks(&old, &new);
        let want = [
            Hunk {
                old: 1,
                old_len: 1,
                new: 1,
                new_len: 1,
            },
            Hunk {
                old: 4,
                old_len: 0,
                new: 4,
                new_len: 2,
            },
            Hunk {
                old: 5,
                old_len: 1,
                new: 7,
                new_len: 0,
            },
        ];
        assert_eq!(hunks, want);
        assert_eq!(old_to_new(&hunks, 5), 7);
        assert_eq!(new_to_old(&hunks, 7), 5);
        assert_eq!(old_to_new(&hunks, 1), 1);
        assert!(super::hunks(&old, &old).is_empty());
    }
}
