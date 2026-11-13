//! Images attached to the next prompt in the window (PLAN.md D70 "Views"):
//! the composer's attach button (the native "Open" dialog), files dropped on
//! the window (winit's `DroppedFile`) and `/image <path>` or `/image clear`
//! typed in the composer attach to the shown session; the chips above the
//! composer show each with its thumbnail, label and an x. A user row shows
//! its images as thumbnails (`thumbs`) with the label lines in its text.

use std::path::{Path, PathBuf};

use fast_harness_core::media;
use fast_harness_core::platform;
use fast_harness_core::service::ServiceError;
use fast_harness_core::types::ImageRef;
use slint::{Model, ModelRc, VecModel};

use crate::app::{show_error, with_app, App};
use crate::{thumbs, ImageChip, MainWindow};

/// The words of an attach during a turn.
pub const RUNNING: &str = "Image not attached: a turn is running; attach it after the turn ends.";
/// What `/image` alone says.
pub const USAGE: &str = "/image <path> attaches a png, jpeg, gif or webp file to the next prompt; /image clear removes the attached images.";

pub fn connect(ui: &MainWindow) {
    ui.on_attach_image(|| {
        with_app(pick);
    });
    ui.on_remove_image(|index| {
        with_app(|app, ui| remove(app, ui, index));
    });
}

/// A file dropped on the window (`window_events`, the window's one winit
/// hook) attaches on the UI thread, one by one.
pub fn dropped(path: PathBuf) {
    let _ = slint::invoke_from_event_loop(move || {
        with_app(|app, ui| attach(app, ui, &path));
    });
}

/// The "Open" dialog on its own thread, owned by the main window.
fn pick(_app: &mut App, _ui: &MainWindow) {
    let owner = platform::thread_window().unwrap_or(0);
    let title = "Attach an image (png, jpeg, gif or webp)";
    let spawned = std::thread::Builder::new()
        .name("fast-harness-pick".into())
        .spawn(move || {
            let picked = platform::pick_file(owner, title);
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app, ui| match picked {
                    Ok(Some(path)) => drop(attach(app, ui, &path)),
                    Ok(None) => {}
                    Err(err) => show_error(ui, format!("Cannot open the file dialog: {err}")),
                });
            });
        });
    if spawned.is_err() {
        with_app(|_, ui| show_error(ui, "Cannot start the file dialog thread."));
    }
}

/// Attaches `path` to the shown session (opened in the service first, as a
/// first message does); false with the reason shown when refused.
pub fn attach(app: &mut App, ui: &MainWindow, path: &Path) -> bool {
    if app.runs.shown.busy {
        show_error(ui, RUNNING);
        return false;
    }
    let attached = crate::turn::shown_session(app).and_then(|_| {
        let session = (app.runs.shown.session.as_ref()).ok_or_else(|| "no session".to_string())?;
        session.attach_image_file(path).map_err(words)
    });
    match attached {
        // The chip says it in words; the banner is for errors.
        Ok(_) => ui.set_error_text("".into()),
        Err(err) => show_error(ui, format!("Image not attached: {err}")),
    }
    refresh(app, ui);
    true
}

/// The x of chip `index`: the session keeps no single removal, so the
/// others are attached again from their media files.
pub fn remove(app: &mut App, ui: &MainWindow, index: i32) {
    if let Some(session) = app.runs.shown.session.clone() {
        let mut images = session.clear_attached_images();
        if let Ok(index) = usize::try_from(index) {
            if index < images.len() {
                images.remove(index);
            }
        }
        for image in images {
            if let Err(err) = session.attach_image_file(Path::new(&image.path)) {
                show_error(ui, format!("Image dropped: {}", words(err)));
            }
        }
    }
    refresh(app, ui);
}

/// `/image <path>`, `/image clear` or `/image` typed in the composer.
pub fn typed(app: &mut App, ui: &MainWindow, args: &str) {
    match args {
        "" => ui.set_error_text(USAGE.into()),
        "clear" => {
            if let Some(session) = &app.runs.shown.session {
                session.clear_attached_images();
            }
            refresh(app, ui);
        }
        path => {
            let path = path.trim_matches(|c| c == '"' || c == '\'');
            let path: PathBuf = app.cwd.join(path);
            attach(app, ui, &path);
        }
    }
}

/// The chips of the shown session's queue (none while a turn runs: the
/// turn took them), and the thumbnails still to decode.
pub fn refresh(app: &App, ui: &MainWindow) {
    let run = &app.runs.shown;
    let images = match (&run.session, run.busy) {
        (Some(session), false) => session.attached_images(),
        _ => Vec::new(),
    };
    ui.set_attached(chips(&images));
    start_thumbs();
}

/// The chips of `images`: thumbnail (empty until decoded), label, file.
pub fn chips(images: &[ImageRef]) -> ModelRc<ImageChip> {
    let chips: Vec<ImageChip> = (images.iter().enumerate())
        .map(|(index, image)| ImageChip {
            thumb: thumbs::get(Path::new(&image.path)),
            label: media::label(image).into(),
            path: image.path.as_str().into(),
            index: index as i32,
        })
        .collect();
    ModelRc::new(VecModel::from(chips))
}

/// Decodes the waiting thumbnails on a worker; they come back to
/// [`arrived`] on the UI thread.
pub fn start_thumbs() {
    thumbs::start(|results| {
        let _ = slint::invoke_from_event_loop(move || {
            thumbs::insert(results);
            with_app(arrived);
        });
    });
}

/// New thumbnails: the chips and the shown session's user rows with images.
pub fn arrived(app: &mut App, ui: &MainWindow) {
    refresh(app, ui);
    let items = &app.runs.shown.view.items;
    for (index, item) in items.iter().enumerate().take(app.rows.row_count()) {
        let fast_harness_core::view::Item::User { images, .. } = item else {
            continue;
        };
        if let (false, Some(mut row)) = (images.is_empty(), app.rows.row_data(index)) {
            row.images = chips(images);
            app.rows.set_row_data(index, row);
        }
    }
}

fn words(err: ServiceError) -> String {
    match err {
        ServiceError::Unknown(text)
        | ServiceError::Busy(text)
        | ServiceError::Invalid(text)
        | ServiceError::Failed(text) => text,
    }
}

#[cfg(test)]
#[path = "images_view_tests.rs"]
mod tests;
