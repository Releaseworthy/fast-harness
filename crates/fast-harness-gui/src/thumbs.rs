//! Thumbnails of attached and sent images (PLAN.md D70 "Views"). Slint's
//! `Image::load_from_path` keeps the whole decoded image (and a 5 MB cache
//! per thread), so the window never shows a file directly: a worker decodes
//! each image once, shrinks it to at most `EDGE` pixels on its longest side
//! and hands back the small pixels, which become an `Image` on the UI
//! thread. The cache is keyed by the media path (its name is the image's
//! sha256) and keeps the newest `KEPT` thumbnails.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// The longest edge of a thumbnail in pixels (64 logical px rows at 200%).
pub const EDGE: u32 = 128;
/// Thumbnails kept (at most 64 KB each).
pub const KEPT: usize = 64;

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

#[derive(Default)]
struct Cache {
    done: HashMap<PathBuf, Image>,
    order: VecDeque<PathBuf>,
    /// Asked once: a file that fails to decode keeps its label only.
    asked: HashSet<PathBuf>,
    waiting: Vec<PathBuf>,
    running: bool,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// The thumbnail of `path`, or an empty image while it is not decoded (the
/// path then waits for [`start`]).
pub fn get(path: &Path) -> Image {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(image) = cache.done.get(path) {
            return image.clone();
        }
        if cache.asked.insert(path.to_path_buf()) {
            cache.waiting.push(path.to_path_buf());
        }
        Image::default()
    })
}

/// The decoded thumbnails, newest last, oldest dropped past [`KEPT`].
pub fn insert(results: Vec<(PathBuf, Option<Pixels>)>) {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.running = false;
        for (path, pixels) in results {
            let Some(pixels) = pixels else { continue };
            cache.done.insert(path.clone(), Image::from_rgba8(pixels));
            cache.order.push_back(path);
        }
        while cache.order.len() > KEPT {
            if let Some(old) = cache.order.pop_front() {
                cache.done.remove(&old);
                cache.asked.remove(&old);
            }
        }
    });
}

/// Starts one worker for the waiting paths (none while one runs); `post`
/// hands its results to the UI thread.
pub fn start(post: impl FnOnce(Vec<(PathBuf, Option<Pixels>)>) + Send + 'static) {
    let paths = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.running || cache.waiting.is_empty() {
            return Vec::new();
        }
        cache.running = true;
        std::mem::take(&mut cache.waiting)
    });
    if paths.is_empty() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("fast-harness-thumbs".into())
        .spawn(move || post(paths.iter().map(|p| (p.clone(), decode(p))).collect()));
    if spawned.is_err() {
        CACHE.with(|cache| cache.borrow_mut().running = false);
    }
}

/// Decodes `path` and shrinks it to at most [`EDGE`] pixels (on a worker:
/// the full image lives only during this call and this thread's cache).
pub fn decode(path: &Path) -> Option<Pixels> {
    let full = Image::load_from_path(path).ok()?.to_rgba8()?;
    Some(shrink(&full, EDGE))
}

/// A box filter down to at most `edge` pixels on the longest side.
pub fn shrink(full: &Pixels, edge: u32) -> Pixels {
    let (w, h) = (full.width(), full.height());
    let long = w.max(h);
    if long <= edge || w == 0 || h == 0 {
        return full.clone();
    }
    let (nw, nh) = ((w * edge / long).max(1), (h * edge / long).max(1));
    let mut out = Pixels::new(nw, nh);
    let src = full.as_slice();
    let dst = out.make_mut_slice();
    for y in 0..nh {
        let rows = (y * h / nh)..((y + 1) * h / nh).max(y * h / nh + 1);
        for x in 0..nw {
            let cols = (x * w / nw)..((x + 1) * w / nw).max(x * w / nw + 1);
            dst[(y * nw + x) as usize] = average(src, w, rows.clone(), cols);
        }
    }
    out
}

fn average(
    src: &[Rgba8Pixel],
    w: u32,
    rows: std::ops::Range<u32>,
    cols: std::ops::Range<u32>,
) -> Rgba8Pixel {
    let (mut sum, mut n) = ([0u32; 4], 0u32);
    for y in rows {
        for x in cols.clone() {
            let p = src[(y * w + x) as usize];
            for (s, v) in sum.iter_mut().zip([p.r, p.g, p.b, p.a]) {
                *s += u32::from(v);
            }
            n += 1;
        }
    }
    let [r, g, b, a] = sum.map(|s| (s / n.max(1)) as u8);
    Rgba8Pixel { r, g, b, a }
}
