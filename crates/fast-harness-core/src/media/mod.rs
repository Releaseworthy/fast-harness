//! Image input (PLAN.md D70): checks, the session's media folder, token
//! estimates and the words a transcript shows. An image is kept once under
//! `<session>.media/<sha256>.<ext>` and referenced by [`ImageRef`]; its
//! bytes are read only when a request is built ([`base64`]).
//!
//! No image crate: types and sizes come from the headers ([`sniff`]); an
//! image over the caps is refused, never downscaled here (the provider
//! downscales one over its own long edge).

mod png;
pub mod sniff;

use std::fmt;
use std::path::Path;

pub use png::solid_png;

use crate::crypto::{base64_encode, hex, sha256};
use crate::types::{Block, ImageRef, Message, Role};

/// Largest image file: 5,000,000 bytes once base64 encoded, the per-image
/// limit of Amazon Bedrock and Google Cloud (the Claude API takes 10 MB).
pub const MAX_BYTES: u64 = 3_750_000;
/// Longest edge the Claude API takes (8000 x 8000).
pub const MAX_EDGE: u32 = 8000;
/// Above this many images in one request every image must fit [`MANY_EDGE`].
pub const MANY_IMAGES: usize = 20;
pub const MANY_EDGE: u32 = 2000;
/// Most images one request carries (the API limit for 200k context models);
/// older ones are sent as a text placeholder.
pub const MAX_IMAGES: usize = 100;
/// Image bytes one request carries at most (32 MB in base64): the newest
/// images within it go, older ones are sent as a text placeholder. The
/// Messages API limit read 2026-11-13 was 128 MB on platform.claude.com
/// and 32 MB on other pages; partner platforms take 20 to 30 MB.
pub const MAX_REQUEST_BYTES: u64 = 24_000_000;

/// Why an image was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaError {
    /// A declared type other than the four.
    Unsupported(String),
    /// The bytes are not a PNG, JPEG, GIF or WebP image.
    NotImage,
    /// The declared type differs from the bytes.
    Mismatch {
        declared: String,
        actual: String,
    },
    TooLarge {
        bytes: u64,
    },
    /// The header holds no readable size.
    NoSize,
    TooWide {
        width: u32,
        height: u32,
    },
    Io(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const KINDS: &str = "png, jpeg, gif or webp";
        match self {
            MediaError::Unsupported(kind) => write!(f, "{kind} is not supported: only {KINDS}"),
            MediaError::NotImage => write!(f, "not a {KINDS} image"),
            MediaError::Mismatch { declared, actual } => {
                write!(f, "declared {declared} but the bytes are {actual}")
            }
            MediaError::TooLarge { bytes } => write!(
                f,
                "the image has {} and the cap is {}: make it smaller (fewer pixels or jpeg) and attach it again",
                size_words(*bytes),
                size_words(MAX_BYTES)
            ),
            MediaError::NoSize => write!(f, "the image header holds no readable size"),
            MediaError::TooWide { width, height } => write!(
                f,
                "the image is {width}x{height} px and the longest edge allowed is {MAX_EDGE} px"
            ),
            MediaError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for MediaError {}

/// The type and size of `bytes` within the caps; `declared` (a media type)
/// must match the bytes when given.
pub fn check(
    bytes: &[u8],
    declared: Option<&str>,
) -> Result<(&'static str, (u32, u32)), MediaError> {
    let declared = declared.map(|d| d.trim().to_ascii_lowercase());
    let declared = declared.map(|d| {
        if d == "image/jpg" {
            "image/jpeg".to_string()
        } else {
            d
        }
    });
    if let Some(kind) = declared.as_deref() {
        if !matches!(
            kind,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) {
            return Err(MediaError::Unsupported(kind.to_string()));
        }
    }
    let actual = sniff::media_type(bytes).ok_or(MediaError::NotImage)?;
    if let Some(declared) = declared.filter(|d| d != actual) {
        let actual = actual.to_string();
        return Err(MediaError::Mismatch { declared, actual });
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Err(MediaError::TooLarge {
            bytes: bytes.len() as u64,
        });
    }
    let (width, height) = sniff::dimensions(bytes).ok_or(MediaError::NoSize)?;
    if width.max(height) > MAX_EDGE {
        return Err(MediaError::TooWide { width, height });
    }
    Ok((actual, (width, height)))
}

/// Checks `bytes` and keeps them in `dir` (the session's media folder)
/// under their SHA-256; the same image twice is one file.
pub fn attach_bytes(
    dir: &Path,
    bytes: &[u8],
    declared: Option<&str>,
) -> Result<ImageRef, MediaError> {
    let (kind, size) = check(bytes, declared)?;
    let name = format!("{}.{}", &hex(&sha256(bytes))[..32], sniff::extension(kind));
    let path = dir.join(name);
    let kept = std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64);
    if !kept {
        crate::store::write_atomic(&path, bytes).map_err(|err| {
            MediaError::Io(format!("cannot keep the image in {}: {err}", dir.display()))
        })?;
    }
    let path = path.display().to_string();
    Ok(ImageRef::new(path, kind, bytes.len() as u64, size))
}

/// [`attach_bytes`] of a file; its size is checked before it is read.
pub fn attach_file(dir: &Path, path: &Path) -> Result<ImageRef, MediaError> {
    let io = |err: std::io::Error| MediaError::Io(format!("cannot read {}: {err}", path.display()));
    let len = std::fs::metadata(path).map_err(io)?.len();
    if len > MAX_BYTES {
        return Err(MediaError::TooLarge { bytes: len });
    }
    attach_bytes(dir, &std::fs::read(path).map_err(io)?, None)
}

/// The image's bytes as base64, read now; an error names what is wrong
/// (the file is missing or changed).
pub fn base64(image: &ImageRef) -> Result<String, String> {
    let bytes = std::fs::read(&image.path)
        .map_err(|err| format!("its file {} cannot be read: {err}", image.path))?;
    if bytes.len() as u64 > MAX_BYTES
        || sniff::media_type(&bytes) != Some(image.media_type.as_str())
    {
        return Err(format!("its file {} changed", image.path));
    }
    Ok(base64_encode(&bytes))
}

/// Estimated input tokens: 28 x 28 px patches after the provider's
/// downscale to a 2576 px long edge, at most 4784 (Claude's high-resolution
/// tier, the costliest; platform.claude.com vision docs, 2026-11-13).
pub fn tokens(image: &ImageRef) -> u64 {
    let (w, h) = (
        f64::from(image.width.max(1)),
        f64::from(image.height.max(1)),
    );
    let scale = (2576.0 / w.max(h)).min(1.0);
    let patches = |side: f64| (side * scale / 28.0).ceil().max(1.0);
    ((patches(w) * patches(h)) as u64).min(4784)
}

/// `png 640x480, 12.3 KB, about 400 tokens`: what an image costs.
pub fn label(image: &ImageRef) -> String {
    let kind = image.media_type.trim_start_matches("image/");
    format!(
        "{kind} {}x{}, {}, about {} tokens",
        image.width,
        image.height,
        size_words(image.bytes),
        tokens(image)
    )
}

/// The text sent instead of an image that cannot go: `[image not sent:
/// <why>; <label>]`.
pub fn placeholder(image: &ImageRef, why: &str) -> String {
    format!("[image not sent: {why}; {}]", label(image))
}

/// A user message with images: each image after a label line `[image N:
/// <label>]` (images before the text, as the Claude docs advise), then`text`.
/// Without images it is `Message::user(text)`.
pub fn user_message(text: &str, images: &[ImageRef]) -> Message {
    let mut content = Vec::with_capacity(images.len() * 2 + 1);
    for (n, image) in images.iter().enumerate() {
        content.push(Block::text(format!(
            "[image {}: {}]\n",
            n + 1,
            label(image)
        )));
        content.push(Block::Image(image.clone()));
    }
    content.push(Block::text(text));
    Message::new(Role::User, content)
}

/// What a transcript shows for a user message with images: the text of
/// [`user_message`], the same live and on replay.
pub fn user_text(text: &str, images: &[ImageRef]) -> String {
    user_message(text, images).text()
}

fn size_words(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} bytes"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
