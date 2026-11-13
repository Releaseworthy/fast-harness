//! Images attached to the next prompt (PLAN.md D70): the session methods a
//! front end calls. An attached image is checked, kept in the session's
//! media folder and queued; the next user message carries every queued
//! image before its text, each after a label line with its size and token
//! cost (`media::user_text` is what the transcript shows).

use std::path::Path;

use super::{busy, ServiceError, Session};
use crate::media::{self, MediaError};
use crate::types::ImageRef;

impl Session {
    /// Attaches an image file (png, jpeg, gif or webp within the caps).
    /// Refused while a turn runs (`Busy`) and when the file is not one.
    pub fn attach_image_file(&self, path: &Path) -> Result<ImageRef, ServiceError> {
        self.attach_with(|dir| media::attach_file(dir, path))
    }

    /// Attaches image bytes (a paste); `media_type`, when given, must match.
    pub fn attach_image_bytes(
        &self,
        bytes: &[u8],
        media_type: Option<&str>,
    ) -> Result<ImageRef, ServiceError> {
        self.attach_with(|dir| media::attach_bytes(dir, bytes, media_type))
    }

    /// The images queued for the next prompt.
    pub fn attached_images(&self) -> Vec<ImageRef> {
        let state = self.lock();
        let conv = state.conv.as_ref();
        conv.map(|c| c.tool_session.attached()).unwrap_or_default()
    }

    /// Drops the queued images (the files stay: another message may use them).
    pub fn clear_attached_images(&self) -> Vec<ImageRef> {
        let state = self.lock();
        let conv = state.conv.as_ref();
        conv.map(|c| c.tool_session.take_attached())
            .unwrap_or_default()
    }

    fn attach_with(
        &self,
        keep: impl FnOnce(&Path) -> Result<ImageRef, MediaError>,
    ) -> Result<ImageRef, ServiceError> {
        let state = self.lock();
        let conv = state.conv.as_ref().ok_or_else(|| busy(&self.id))?;
        let session = &conv.tool_session;
        let dir = session
            .media_dir()
            .ok_or_else(|| ServiceError::Failed("this session keeps no files".into()))?;
        let image = keep(dir).map_err(|err| ServiceError::Failed(err.to_string()))?;
        session.attach(image.clone());
        Ok(image)
    }
}
