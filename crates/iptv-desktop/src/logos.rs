//! Channel logos for the mpv window.
//!
//! mpv's text overlays can't show pictures, but its `overlay-add` command
//! shows raw BGRA images from a file. Logos are downloaded in the background
//! (a few at a time), shrunk and kept on disk, so each one is fetched once;
//! for display they are scaled to the size needed and written as raw files.

use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime},
};

use image::{imageops::FilterType, RgbaImage};
use tokio::sync::{mpsc, Semaphore};

/// Largest size kept on disk (logos are shown much smaller).
const KEEP_W: u32 = 192;
const KEEP_H: u32 = 96;
/// Downloads at the same time.
const PARALLEL: usize = 6;
/// A logo that failed to load is retried after this long.
const RETRY_AFTER: Duration = Duration::from_secs(3 * 24 * 3600);

/// A logo scaled for display: a raw premultiplied-BGRA file for mpv.
pub struct Rendered {
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
}

enum Entry {
    Ready(Arc<RgbaImage>),
    Failed,
}

pub struct Logos {
    dir: PathBuf,
    user_agent: String,
    mem: HashMap<String, Entry>,
    loading: HashSet<String>,
    limit: Arc<Semaphore>,
    /// Finished loads: (logo URL, image or None).
    tx: mpsc::UnboundedSender<(String, Option<RgbaImage>)>,
}

impl Logos {
    /// Loaded logos arrive on the returned receiver; pass them to [`Logos::loaded`].
    pub fn new(
        cache_dir: PathBuf,
        user_agent: &str,
    ) -> (Self, mpsc::UnboundedReceiver<(String, Option<RgbaImage>)>) {
        let dir = cache_dir.join("logos");
        let _ = std::fs::create_dir_all(dir.join("render"));
        let (tx, rx) = mpsc::unbounded_channel();
        let logos = Self {
            dir,
            user_agent: user_agent.to_string(),
            mem: HashMap::new(),
            loading: HashSet::new(),
            limit: Arc::new(Semaphore::new(PARALLEL)),
            tx,
        };
        (logos, rx)
    }

    /// The logo if it's ready; otherwise starts loading it (once) and
    /// returns None.
    pub fn get(&mut self, url: &str) -> Option<Arc<RgbaImage>> {
        match self.mem.get(url) {
            Some(Entry::Ready(img)) => return Some(img.clone()),
            Some(Entry::Failed) => return None,
            None => {}
        }
        if url.is_empty() || !self.loading.insert(url.to_string()) {
            return None;
        }
        let (url, dir, ua, tx, limit) = (
            url.to_string(),
            self.dir.clone(),
            self.user_agent.clone(),
            self.tx.clone(),
            self.limit.clone(),
        );
        tokio::spawn(async move {
            let Ok(_permit) = limit.acquire_owned().await else {
                return;
            };
            let key = url.clone();
            let img = tokio::task::spawn_blocking(move || load(&dir, &key, &ua))
                .await
                .ok()
                .flatten();
            let _ = tx.send((url, img));
        });
        None
    }

    /// Records a finished load.
    pub fn loaded(&mut self, url: String, img: Option<RgbaImage>) {
        self.loading.remove(&url);
        let entry = match img {
            Some(img) => Entry::Ready(Arc::new(img)),
            None => Entry::Failed,
        };
        self.mem.insert(url, entry);
    }

    /// The logo scaled to fit `max_w` x `max_h` pixels, as a file for mpv.
    pub fn render(&mut self, url: &str, max_w: u32, max_h: u32) -> Option<Rendered> {
        let img = self.get(url)?;
        let (w, h) = fit(img.width(), img.height(), max_w.max(1), max_h.max(1));
        let path = self
            .dir
            .join("render")
            .join(format!("{:016x}-{w}x{h}.bgra", key(url)));
        if !path.is_file() {
            let small = image::imageops::resize(img.as_ref(), w, h, FilterType::Triangle);
            // mpv wants premultiplied BGRA.
            let mut raw = Vec::with_capacity((w * h * 4) as usize);
            for p in small.pixels() {
                let [r, g, b, a] = p.0;
                let pm = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
                raw.extend_from_slice(&[pm(b), pm(g), pm(r), a]);
            }
            std::fs::write(&path, raw).ok()?;
        }
        Some(Rendered { path, w, h })
    }
}

/// The largest size with the image's proportions that fits the box.
fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

fn key(url: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    h.finish()
}

/// From the disk cache, else downloaded, shrunk and cached. Blocking.
fn load(dir: &std::path::Path, url: &str, user_agent: &str) -> Option<RgbaImage> {
    let base = dir.join(format!("{:016x}", key(url)));
    let png = base.with_extension("png");
    let failed = base.with_extension("failed");
    if let Ok(img) = image::open(&png) {
        return Some(img.to_rgba8());
    }
    let recent_failure = std::fs::metadata(&failed)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < RETRY_AFTER);
    if recent_failure {
        return None;
    }
    let decoded = iptv_net::logo::fetch(url, user_agent)
        .ok()
        .and_then(|bytes| image::load_from_memory(&bytes).ok());
    let Some(img) = decoded else {
        let _ = std::fs::write(&failed, b"");
        return None;
    };
    let img = img.to_rgba8();
    let (w, h) = fit(img.width(), img.height(), KEEP_W, KEEP_H);
    let img = if w < img.width() {
        image::imageops::resize(&img, w, h, FilterType::Lanczos3)
    } else {
        img
    };
    let _ = img.save(&png);
    Some(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_keeping_proportions() {
        assert_eq!(fit(400, 200, 40, 24), (40, 20));
        assert_eq!(fit(100, 300, 40, 24), (8, 24));
        assert_eq!(fit(1, 1, 40, 24), (24, 24));
    }
}
