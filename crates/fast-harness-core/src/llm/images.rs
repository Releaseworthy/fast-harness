//! Images in a request (PLAN.md D70): which go and which become a text
//! placeholder, and the bytes of those that go, read now. The choice depends
//! only on the history, the files and the model, so a request rebuilt from
//! the same history is the same body (the prompt cache holds).

use std::borrow::Cow;
use std::collections::HashMap;

use crate::media;
use crate::types::{Block, ImageRef, Message, Role};

/// The history to render and the base64 data of every image left in it.
pub struct Prepared<'a> {
    pub messages: Cow<'a, [Message]>,
    bytes: HashMap<String, String>,
}

impl Prepared<'_> {
    /// The base64 bytes of an image left in [`Prepared::messages`].
    pub fn data(&self, image: &ImageRef) -> &str {
        self.bytes.get(&image.path).map_or("", String::as_str)
    }
}

/// Text sent instead of an image to a model that takes none.
pub const NO_VISION: &str = "this model does not take images";

/// Keeps the newest [`media::MAX_IMAGES`] images whose file reads, when the
/// model takes images (`vision`); above [`media::MANY_IMAGES`] an image
/// wider than [`media::MANY_EDGE`] is left out too. Every other image
/// becomes text: in a tool message appended to the result it belongs to,
/// elsewhere a text block in its place. No image: the history unchanged.
pub fn prepare(messages: &[Message], vision: bool) -> Prepared<'_> {
    let all: Vec<&ImageRef> = messages.iter().flat_map(Message::images).collect();
    let mut bytes = HashMap::new();
    if all.is_empty() {
        let messages = Cow::Borrowed(messages);
        return Prepared { messages, bytes };
    }
    let first_kept = all.len().saturating_sub(media::MAX_IMAGES);
    let many = all.len() - first_kept > media::MANY_IMAGES;
    let mut seen = 0;
    let mut why = |image: &ImageRef| -> Option<String> {
        seen += 1;
        if !vision {
            return Some(NO_VISION.into());
        }
        if seen <= first_kept {
            return Some(format!(
                "a request carries at most {} images, this is an earlier one",
                media::MAX_IMAGES
            ));
        }
        if many && image.width.max(image.height) > media::MANY_EDGE {
            return Some(format!(
                "above {} images in a request each must fit {} px",
                media::MANY_IMAGES,
                media::MANY_EDGE
            ));
        }
        match media::base64(image) {
            Ok(data) => {
                bytes.insert(image.path.clone(), data);
                None
            }
            Err(err) => Some(err),
        }
    };
    let messages = messages.iter().map(|m| rewrite(m, &mut why)).collect();
    Prepared {
        messages: Cow::Owned(messages),
        bytes,
    }
}

fn rewrite(message: &Message, why: &mut dyn FnMut(&ImageRef) -> Option<String>) -> Message {
    let mut out = message.clone();
    out.content.clear();
    for block in &message.content {
        let Block::Image(image) = block else {
            out.content.push(block.clone());
            continue;
        };
        let Some(why) = why(image) else {
            out.content.push(block.clone());
            continue;
        };
        let text = media::placeholder(image, &why);
        match out.content.last_mut() {
            Some(Block::ToolResult { content, .. }) if message.role == Role::Tool => {
                content.push('\n');
                content.push_str(&text);
            }
            _ => out.content.push(Block::text(text)),
        }
    }
    out
}
