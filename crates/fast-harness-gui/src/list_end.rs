//! The virtual list's geometry (`virtual_list`): the rows' tops, the rows
//! around the view, and the end a following view holds.

use slint::{Model, VecModel};

/// Moves the window's rows from span `old` to `(a, b)`: its ends move one
/// row at a time, so the rows that stay keep their laid out views.
pub fn set_window(model: &VecModel<i32>, (a0, b0): (usize, usize), (a, b): (usize, usize)) {
    if model.row_count() != b0 - a0 || b <= a0 || a >= b0 {
        model.set_vec((a..b).map(|i| i as i32).collect::<Vec<_>>());
        return;
    }
    for _ in b..b0 {
        model.remove(model.row_count() - 1);
    }
    (b0..b).for_each(|i| model.push(i as i32));
    for _ in a0..a {
        model.remove(0);
    }
    (a..a0).rev().for_each(|i| model.insert(0, i as i32));
}

/// Rows laid out above and below the view, in view heights.
const OVERSCAN: f32 = 0.5;

/// The tops of rows of `heights`, and the end.
pub fn prefix(heights: &[f32]) -> Vec<f32> {
    let mut tops = Vec::with_capacity(heights.len() + 1);
    let mut y = 0.0;
    tops.push(y);
    for height in heights {
        y += height;
        tops.push(y);
    }
    tops
}

/// The rows to lay out for the view at `offset`: in it and around it.
pub fn span(tops: &[f32], n: usize, offset: f32, view: f32) -> (usize, usize) {
    if n == 0 || tops.len() != n + 1 {
        return (0, 0);
    }
    let (low, high) = (offset - OVERSCAN * view, offset + view + OVERSCAN * view);
    let a = tops[1..].partition_point(|t| *t <= low);
    let b = tops[..n].partition_point(|t| *t < high);
    (a.min(b), b)
}

/// A row as the end of a following list sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tail {
    /// Laid out at least once: its height counts.
    Measured,
    /// Only an estimate so far (never laid out).
    Guessed,
    /// A spacer or a row that draws nothing: looked past.
    Skip,
}

/// The end a following view holds: the content's end less the rows at the
/// end that only have an estimate. Such a row draws below the view for a
/// frame and the view moves up by its real height once it reports.
/// Counting its estimate scrolled the view by the estimate at once and
/// back by the difference a frame later (the owner 2026-11-15: a new step
/// jumped the chat up, then back down). With no measured row before them
/// (a list just opened at its end) they all count, so the rows at the end
/// get laid out first.
pub fn settled_end(tops: &[f32], tails: &[Tail]) -> f32 {
    let full = tops.last().copied().unwrap_or(0.0);
    let mut end = full;
    for (i, tail) in tails.iter().enumerate().rev() {
        match tail {
            Tail::Measured => return end,
            Tail::Skip => {}
            Tail::Guessed => {
                let (Some(top), Some(next)) = (tops.get(i), tops.get(i + 1)) else {
                    return full;
                };
                end -= next - top;
            }
        }
    }
    full
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guessed_rows_at_the_end_do_not_count() {
        let tops = prefix(&[100.0, 50.0, 300.0, 20.0]);
        use Tail::*;
        assert_eq!(settled_end(&tops, &[Measured; 4]), 470.0);
        // The new row (300 px estimated) is looked past, the spacer stays.
        assert_eq!(settled_end(&tops, &[Measured, Measured, Guessed, Skip]), 170.0);
        // A guessed row above a measured one counts.
        assert_eq!(settled_end(&tops, &[Guessed, Measured, Measured, Measured]), 470.0);
        // Nothing measured yet (a list just opened): everything counts.
        assert_eq!(settled_end(&tops, &[Guessed; 4]), 470.0);
    }
}
