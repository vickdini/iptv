//! Background channel health checks.
//!
//! Each channel is probed with a quick HTTP request (using the channel's own
//! user-agent/referrer). For HLS streams the playlist must be valid and, for a
//! master playlist, its first variant must be reachable too. Results are kept
//! on disk for [`TTL`] so later launches hide dead channels immediately and
//! only re-check old results.

use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use iptv_core::Channel;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::storage;

/// How long a check result is trusted before the channel is re-checked.
pub const TTL: Duration = Duration::from_secs(6 * 3600);
/// Parallel probes.
pub const WORKERS: usize = 24;
const MAX_PLAYLIST_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Ok,
    Dead,
    /// Couldn't tell (e.g. non-HTTP scheme, TLS quirks the player tolerates).
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Entry {
    ok: bool,
    /// Unix time of the check, in seconds.
    at: u64,
}

/// Check results persisted between launches, keyed by stream URL.
pub struct HealthCache {
    path: PathBuf,
    entries: HashMap<String, Entry>,
    dirty: bool,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl HealthCache {
    pub fn load() -> Self {
        let path = storage::cache_dir().join("health.json");
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { path, entries, dirty: false }
    }

    /// `(works, still_fresh)` for a URL checked before.
    pub fn get(&self, url: &str) -> Option<(bool, bool)> {
        self.entries
            .get(url)
            .map(|e| (e.ok, now().saturating_sub(e.at) < TTL.as_secs()))
    }

    pub fn record(&mut self, url: &str, ok: bool) {
        self.entries.insert(url.to_string(), Entry { ok, at: now() });
        self.dirty = true;
    }

    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                fs::create_dir_all(dir)?;
            }
            let tmp = self.path.with_extension("tmp");
            fs::write(&tmp, serde_json::to_vec(&self.entries)?)?;
            fs::rename(&tmp, &self.path)
        })();
        match result {
            Ok(()) => self.dirty = false,
            Err(e) => eprintln!("warning: could not save channel check results: {e}"),
        }
    }
}

pub struct CheckResult {
    pub idx: usize,
    pub probe: Probe,
}

/// Probes `jobs` (channel index + channel) on background threads, in order,
/// sending each result to `tx`. Stops early once `tx` is closed.
pub fn spawn_checker(jobs: Vec<(usize, Channel)>, default_ua: &'static str, tx: UnboundedSender<CheckResult>) {
    let queue = Arc::new(Mutex::new(VecDeque::from(jobs)));
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .redirects(5)
        .build();
    for _ in 0..WORKERS {
        let (queue, agent, tx) = (queue.clone(), agent.clone(), tx.clone());
        thread::spawn(move || loop {
            let Some((idx, ch)) = queue.lock().unwrap().pop_front() else { return };
            let probe = probe(&agent, &ch, default_ua);
            if tx.send(CheckResult { idx, probe }).is_err() {
                return;
            }
        });
    }
}

pub fn probe(agent: &ureq::Agent, ch: &Channel, default_ua: &str) -> Probe {
    let ua = ch.user_agent.as_deref().unwrap_or(default_ua);
    let get = |url: &str| {
        let mut req = agent.get(url).set("User-Agent", ua);
        if let Some(r) = &ch.referrer {
            req = req.set("Referer", r);
        }
        req.call().map_err(classify)
    };

    let resp = match get(&ch.url) {
        Ok(r) => r,
        Err(p) => return p,
    };
    let final_url = resp.get_url().to_string();
    let hls_hint = final_url.to_ascii_lowercase().contains(".m3u8")
        || resp.content_type().to_ascii_lowercase().contains("mpegurl");
    let Some(body) = read_text(resp, if hls_hint { MAX_PLAYLIST_BYTES } else { 4096 }) else {
        return Probe::Dead;
    };

    if !body.trim_start().starts_with("#EXTM3U") {
        // Not a playlist: fine for direct streams (TS, DASH, ...), but an
        // .m3u8 URL answering with something else (an HTML error page) is dead.
        return if hls_hint { Probe::Dead } else { Probe::Ok };
    }
    let Some(variant) = first_variant(&body) else {
        // Media playlist: it must list segments.
        return if body.contains("#EXTINF") { Probe::Ok } else { Probe::Dead };
    };
    // Master playlist: the first variant must be reachable too.
    let Ok(variant_url) = url::Url::parse(&final_url).and_then(|base| base.join(variant)) else {
        return Probe::Dead;
    };
    match get(variant_url.as_str()) {
        Ok(r) => match read_text(r, MAX_PLAYLIST_BYTES) {
            Some(b) if b.trim_start().starts_with("#EXTM3U") => Probe::Ok,
            _ => Probe::Dead,
        },
        Err(p) => p,
    }
}

fn read_text(resp: ureq::Response, limit: u64) -> Option<String> {
    let mut buf = Vec::new();
    resp.into_reader().take(limit).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// URI of the first variant stream in an HLS master playlist.
fn first_variant(playlist: &str) -> Option<&str> {
    let mut lines = playlist.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line.starts_with("#EXT-X-STREAM-INF") {
            return lines.find(|l| !l.is_empty() && !l.starts_with('#'));
        }
    }
    None
}

fn classify(e: ureq::Error) -> Probe {
    match e {
        // 403 (geo-blocked), 404, 5xx ...
        ureq::Error::Status(..) => Probe::Dead,
        ureq::Error::Transport(t) => match t.kind() {
            // rtmp://, rtsp://, udp:// ... : the player may handle these.
            ureq::ErrorKind::UnknownScheme | ureq::ErrorKind::InvalidUrl => Probe::Unknown,
            _ => {
                // The player doesn't verify TLS certificates, so a bad
                // certificate doesn't mean the stream is dead.
                let msg = t.to_string().to_ascii_lowercase();
                if msg.contains("certificate") || msg.contains("tls") || msg.contains("handshake") {
                    Probe::Unknown
                } else {
                    Probe::Dead
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_first_variant() {
        let master = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-STREAM-INF:BANDWIDTH=800000\n\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2000000\nhigh/index.m3u8\n";
        assert_eq!(first_variant(master), Some("low/index.m3u8"));
        assert_eq!(first_variant("#EXTM3U\n#EXTINF:6,\nseg1.ts\n"), None);
    }

    #[test]
    fn unprobeable_schemes_are_unknown() {
        let agent = ureq::agent();
        let ch = Channel { url: "rtmp://example.com/live".into(), ..Default::default() };
        assert_eq!(probe(&agent, &ch, "test"), Probe::Unknown);
    }
}
