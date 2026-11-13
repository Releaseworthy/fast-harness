//! Images attached to the next prompt in the TUI (PLAN.md D70 "Views"):
//! `/image <path>` attaches a file (a pasted path is enough: a terminal has
//! no clipboard image), `/image clear` drops the queue, and one line above
//! the input lists the pending images with their labels. The next prompt
//! takes them; its user item shows `media::user_text`, as a replay does.

use std::path::{Path, PathBuf};

use fast_harness_core::app::image_args;
use fast_harness_core::media;
use fast_harness_core::service::ServiceError;
use fast_harness_core::view::NoteKind;
use ratatui::text::{Line, Span};

use super::app::Tui;
use super::theme::Styles;

/// What `/image` alone says.
pub const USAGE: &str = "/image <path> attaches a png, jpeg, gif or webp file to the next prompt; /image clear removes the attached images";

impl Tui {
    /// `/image <path>` or `/image clear`; false for other text.
    pub(super) fn image_command(&mut self, text: &str) -> bool {
        let Some(args) = image_args(text) else {
            return false;
        };
        match args {
            "" => self.note(NoteKind::Info, USAGE),
            "clear" => {
                let dropped = self.current.clear_attached_images().len();
                self.ui.status = format!("removed {dropped} attached image(s)");
            }
            path => self.attach_image(&resolve(&self.cwd, path)),
        }
        self.refresh_images();
        true
    }

    fn attach_image(&mut self, path: &Path) {
        match self.current.attach_image_file(path) {
            Ok(image) => self.ui.status = format!("attached: {}", media::label(&image)),
            Err(ServiceError::Busy(_)) => self.note(
                NoteKind::Error,
                "image not attached: a turn is running; attach it after the turn ends",
            ),
            Err(err) => self.note(
                NoteKind::Error,
                format!("image not attached: {}", words(err)),
            ),
        }
    }

    /// Reads the shown session's queue into the line above the input.
    pub(super) fn refresh_images(&mut self) {
        self.ui.images = (self.current.attached_images().iter())
            .map(media::label)
            .collect();
    }
}

/// A pasted path: quotes (Explorer's "Copy as path") removed, relative to
/// the session's folder.
fn resolve(cwd: &Path, path: &str) -> PathBuf {
    let path = path.trim_matches(|c| c == '"' || c == '\'');
    cwd.join(path)
}

fn words(err: ServiceError) -> String {
    match err {
        ServiceError::Unknown(text)
        | ServiceError::Busy(text)
        | ServiceError::Invalid(text)
        | ServiceError::Failed(text) => text,
    }
}

/// The line: `images  <mark> 1 png 2x2, 70 bytes, about 4 tokens  ...
/// (/image clear removes them)`; `None` without attached images.
pub fn line(labels: &[String], st: &Styles) -> Option<Line<'static>> {
    if labels.is_empty() {
        return None;
    }
    let mut spans = vec![Span::styled("images", st.dim())];
    for (n, label) in labels.iter().enumerate() {
        let mark = st.icons.image;
        spans.push(Span::styled(format!("  {mark} {} ", n + 1), st.accent()));
        spans.push(Span::styled(label.clone(), st.base()));
    }
    spans.push(Span::styled("  (/image clear removes them)", st.dim()));
    Some(Line::from(spans))
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
