//! Loads the playlist, keeping a local copy that is refreshed only when the
//! server has a newer version.
//!
//! On every launch we send a conditional request (`If-None-Match` with the
//! ETag and `If-Modified-Since` with the Last-Modified date of our copy). If
//! the playlist hasn't changed the server answers `304 Not Modified` with an
//! empty body, so the check costs a few milliseconds; otherwise the new
//! version is downloaded and replaces the cached one. When offline, the
//! cached copy is used.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use iptv_core::{parse_m3u, Channel};
use serde::{Deserialize, Serialize};

/// Validators the server gave us for the cached copy.
#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheMeta {
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

/// What happened while loading, for logging.
#[derive(Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Server confirmed our cached copy is the latest.
    UpToDate,
    /// A new version was downloaded.
    Downloaded,
    /// Server unreachable; using the cached copy.
    Offline,
    /// Local file, no update check.
    Local,
}

/// Loads a playlist from a URL (refreshing the copy cached in `cache_dir` if
/// the server has a newer one) or from a local file.
pub fn load(source: &str, cache_dir: &Path) -> Result<(Vec<Channel>, Freshness)> {
    let (text, freshness) = if is_url(source) {
        load_url(source, cache_dir)?
    } else {
        let text = fs::read_to_string(source).with_context(|| format!("reading {source}"))?;
        (text, Freshness::Local)
    };
    let mut channels = parse_m3u(&text);
    if let Some((by_country, by_language)) = companion_urls(source) {
        // Fetched together (each is a conditional request, so usually cheap).
        let (c, l) = std::thread::scope(|scope| {
            let c = scope.spawn(|| load_url(&by_country, cache_dir));
            let l = scope.spawn(|| load_url(&by_language, cache_dir));
            (c.join(), l.join())
        });
        match (c, l) {
            (Ok(Ok((c, _))), Ok(Ok((l, _)))) => iptv_core::m3u::tag_regions(&mut channels, &c, &l),
            _ => eprintln!("warning: country/language lists unavailable; that filter is disabled"),
        }
    }
    Ok((channels, freshness))
}

/// iptv-org publishes the same streams grouped by country and by language
/// next to `index.m3u`; returns those URLs for such a playlist.
pub fn companion_urls(source: &str) -> Option<(String, String)> {
    let base = source.strip_suffix("/index.m3u")?;
    (is_url(source) && base.contains("iptv-org"))
        .then(|| (format!("{base}/index.country.m3u"), format!("{base}/index.language.m3u")))
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn load_url(url: &str, cache_dir: &Path) -> Result<(String, Freshness)> {
    let (data_path, meta_path) = cache_paths(url, cache_dir);
    let cached = fs::read_to_string(&data_path).ok();
    let meta: Option<CacheMeta> = fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(|m: &CacheMeta| m.url == url);

    match fetch(url, cached.as_ref().and(meta.as_ref())) {
        Ok(Fetched::NotModified) => {
            // `NotModified` is only possible when we sent validators, which
            // requires a cached copy.
            Ok((cached.expect("cached copy"), Freshness::UpToDate))
        }
        Ok(Fetched::New { body, etag, last_modified }) => {
            let meta = CacheMeta { url: url.to_string(), etag, last_modified };
            if let Err(e) = save_cache(&data_path, &meta_path, &body, &meta) {
                eprintln!("warning: could not cache playlist: {e:#}");
            }
            Ok((body, Freshness::Downloaded))
        }
        Err(e) => match cached {
            Some(text) => {
                eprintln!("warning: could not check for a newer playlist ({e:#}); using the saved copy");
                Ok((text, Freshness::Offline))
            }
            None => Err(e),
        },
    }
}

enum Fetched {
    NotModified,
    New { body: String, etag: Option<String>, last_modified: Option<String> },
}

fn fetch(url: &str, validators: Option<&CacheMeta>) -> Result<Fetched> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("iptv/", env!("CARGO_PKG_VERSION")))
        .build();
    let mut req = agent.get(url);
    if let Some(m) = validators {
        if let Some(etag) = &m.etag {
            req = req.set("If-None-Match", etag);
        }
        if let Some(lm) = &m.last_modified {
            req = req.set("If-Modified-Since", lm);
        }
    }
    let resp = req.call()?;
    match resp.status() {
        304 if validators.is_some() => Ok(Fetched::NotModified),
        200 => {
            let etag = resp.header("ETag").map(str::to_string);
            let last_modified = resp.header("Last-Modified").map(str::to_string);
            let body = resp.into_string()?;
            Ok(Fetched::New { body, etag, last_modified })
        }
        status => bail!("unexpected HTTP status {status}"),
    }
}

/// Writes the playlist and its validators atomically (temp file + rename),
/// so an interrupted download never leaves a truncated cache behind.
fn save_cache(data_path: &Path, meta_path: &Path, body: &str, meta: &CacheMeta) -> Result<()> {
    if let Some(dir) = data_path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Remove the old validators first: if we crash between the two renames
    // the next launch simply re-downloads instead of trusting stale data.
    let _ = fs::remove_file(meta_path);
    write_atomic(data_path, body.as_bytes())?;
    write_atomic(meta_path, &serde_json::to_vec_pretty(meta)?)?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn cache_paths(url: &str, cache_dir: &Path) -> (PathBuf, PathBuf) {
    // Simple stable file name derived from the URL (FNV-1a).
    let hash = url
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    let base = cache_dir.join(format!("playlist-{hash:016x}"));
    (base.with_extension("m3u"), base.with_extension("json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    /// Minimal HTTP server: serves `body` with an ETag that changes when the
    /// body changes, answers 304 to a matching If-None-Match. Records the
    /// status of each response.
    struct Server {
        url: String,
        body: Arc<Mutex<String>>,
        log: Arc<Mutex<Vec<u16>>>,
    }

    impl Server {
        fn start(body: &str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/index.m3u", listener.local_addr().unwrap());
            let body = Arc::new(Mutex::new(body.to_string()));
            let log = Arc::new(Mutex::new(Vec::new()));
            let (b, l) = (body.clone(), log.clone());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = stream.unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut if_none_match = None;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("if-none-match:") {
                            if_none_match = Some(v.trim().to_string());
                        }
                    }
                    let body = b.lock().unwrap().clone();
                    let etag = format!("\"v{}\"", body.len());
                    let resp = if if_none_match.as_deref() == Some(etag.as_str()) {
                        l.lock().unwrap().push(304);
                        format!("HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nConnection: close\r\n\r\n")
                    } else {
                        l.lock().unwrap().push(200);
                        format!(
                            "HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    };
                    stream.write_all(resp.as_bytes()).unwrap();
                }
            });
            Self { url, body, log }
        }
    }

    const V1: &str = "#EXTM3U\n#EXTINF:-1,One\nhttp://one\n";
    const V2: &str = "#EXTM3U\n#EXTINF:-1,One\nhttp://one\n#EXTINF:-1,Two\nhttp://two\n";

    #[test]
    fn downloads_once_then_revalidates_then_updates() {
        let dir = std::env::temp_dir().join(format!("iptv-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let server = Server::start(V1);

        // First launch: no cache -> full download.
        let (text, f) = load_url(&server.url, &dir).unwrap();
        assert_eq!((text.as_str(), f), (V1, Freshness::Downloaded));

        // Second launch: unchanged -> 304, served from cache.
        let (text, f) = load_url(&server.url, &dir).unwrap();
        assert_eq!((text.as_str(), f), (V1, Freshness::UpToDate));

        // Playlist changes upstream -> new version downloaded and cached.
        *server.body.lock().unwrap() = V2.to_string();
        let (text, f) = load_url(&server.url, &dir).unwrap();
        assert_eq!((text.as_str(), f), (V2, Freshness::Downloaded));
        let (text, f) = load_url(&server.url, &dir).unwrap();
        assert_eq!((text.as_str(), f), (V2, Freshness::UpToDate));

        assert_eq!(*server.log.lock().unwrap(), vec![200, 304, 200, 304]);

        // Cache deleted but validators left behind -> must not send them.
        let (data, _) = cache_paths(&server.url, &dir);
        fs::remove_file(data).unwrap();
        let (text, f) = load_url(&server.url, &dir).unwrap();
        assert_eq!((text.as_str(), f), (V2, Freshness::Downloaded));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn companion_urls_only_for_iptv_org() {
        assert_eq!(
            companion_urls("https://iptv-org.github.io/iptv/index.m3u"),
            Some((
                "https://iptv-org.github.io/iptv/index.country.m3u".into(),
                "https://iptv-org.github.io/iptv/index.language.m3u".into()
            ))
        );
        assert_eq!(companion_urls("https://example.com/index.m3u"), None);
        assert_eq!(companion_urls("https://iptv-org.github.io/iptv/countries/it.m3u"), None);
    }

    #[test]
    fn falls_back_to_cache_when_offline() {
        let dir = std::env::temp_dir().join(format!("iptv-test-offline-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let server = Server::start(V1);
        load_url(&server.url, &dir).unwrap();

        // Same cache files, but pretend the URL now points nowhere by
        // copying them under the name of an unreachable URL.
        let dead = "http://127.0.0.1:9/index.m3u";
        let (src_data, src_meta) = cache_paths(&server.url, &dir);
        let (dst_data, dst_meta) = cache_paths(dead, &dir);
        fs::copy(src_data, dst_data).unwrap();
        let mut meta: CacheMeta = serde_json::from_str(&fs::read_to_string(src_meta).unwrap()).unwrap();
        meta.url = dead.into();
        fs::write(dst_meta, serde_json::to_string(&meta).unwrap()).unwrap();

        let (text, f) = load_url(dead, &dir).unwrap();
        assert_eq!((text.as_str(), f), (V1, Freshness::Offline));

        // No cache at all and offline -> error.
        assert!(load_url("http://127.0.0.1:9/other.m3u", &dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
