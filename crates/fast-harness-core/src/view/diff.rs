//! A line diff: common prefix and suffix as context, the longest common
//! subsequence on the middle while it is small enough.

use super::tool_detail::LineKind;

/// Above this many cells (middle old lines times middle new lines) the
/// middle shows as all removed, then all added.
const MAX_CELLS: usize = 4_000_000;

/// The lines of `old` and `new` as Context, Removed and Added; in a change
/// Removed lines come before Added ones.
pub fn line_diff(old: &str, new: &str) -> Vec<(LineKind, String)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let (mid_a, mid_b) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let mut out: Vec<(LineKind, String)> = Vec::new();
    out.extend(tagged(LineKind::Context, &a[..prefix]));
    if mid_a.len().saturating_mul(mid_b.len()) <= MAX_CELLS {
        lcs(mid_a, mid_b, &mut out);
    } else {
        out.extend(tagged(LineKind::Removed, mid_a));
        out.extend(tagged(LineKind::Added, mid_b));
    }
    out.extend(tagged(LineKind::Context, &a[a.len() - suffix..]));
    out
}

/// Walks the longest common subsequence table of `a` and `b`.
fn lcs(a: &[&str], b: &[&str], out: &mut Vec<(LineKind, String)>) {
    let width = b.len() + 1;
    // table[i * width + j]: the LCS length of a[i..] and b[j..].
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = if a[i] == b[j] {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push((LineKind::Context, a[i].to_string()));
            (i, j) = (i + 1, j + 1);
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            out.push((LineKind::Removed, a[i].to_string()));
            i += 1;
        } else {
            out.push((LineKind::Added, b[j].to_string()));
            j += 1;
        }
    }
    out.extend(tagged(LineKind::Removed, &a[i..]));
    out.extend(tagged(LineKind::Added, &b[j..]));
}
fn tagged(kind: LineKind, lines: &[&str]) -> Vec<(LineKind, String)> {
    lines.iter().map(|line| (kind, line.to_string())).collect()
}
