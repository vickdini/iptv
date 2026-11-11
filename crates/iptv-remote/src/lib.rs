//! Phone remote: the TV app runs a small web server on the home network.
//! A phone opens its page (by scanning a QR code shown on the TV) and gets a
//! remote control with the channel list, search, favorites, volume and audio
//! / subtitle choice, in any browser: nothing to install.
//!
//! * The page and its JSON API are served by plain HTTP/1.1 over a
//!   [`TcpListener`] (one short-lived thread per request; a home has few
//!   phones). The page waits for changes with a long poll on `/api/state`.
//! * Every API call must carry the PIN shown on the TV (it travels in the QR
//!   code's `#fragment`, so it never appears in a URL the server logs).
//!   Repeated wrong PINs lock the API for a few minutes.
//! * The app publishes what the phone shows with [`Remote::update`] and
//!   receives the phone's requests as [`Command`]s from a channel.

use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, TcpListener, TcpStream, UdpSocket},
    sync::{mpsc, Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use iptv_core::{tuner::normalize, Tuner};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The port tried first (then the next few), so a bookmarked page keeps
/// working across restarts.
pub const PORT: u16 = 47123;

const PAGE: &str = include_str!("page.html");
/// Wrong PINs tolerated per minute before the API locks.
const MAX_FAILURES: usize = 10;
const LOCKOUT: Duration = Duration::from_secs(300);
/// How long `/api/state?seq=` waits for a change before answering anyway.
const LONG_POLL: Duration = Duration::from_secs(25);

/// What the phone asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Command {
    /// A remote-control action of the tuner ("up", "down", "prev", "info"...).
    Action {
        name: String,
        #[serde(default)]
        arg: Option<String>,
    },
    /// Watch this channel (index).
    Tune { idx: usize },
    /// Toggle this channel's favorite star.
    Fav { idx: usize },
    /// Change the volume by this many percent.
    Volume { delta: i32 },
    /// Toggle mute.
    Mute,
    /// Choose an audio / subtitle track (`id` None: subtitles off).
    Track { kind: TrackKind, id: Option<i64> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Audio,
    Sub,
}

/// An audio or subtitle track as the phone shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub id: i64,
    pub label: String,
    pub selected: bool,
}

/// Everything the phone page shows.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Channel names by index (shared, never changes).
    pub names: Arc<Vec<String>>,
    /// The names prepared for searching.
    pub search: Arc<Vec<String>>,
    /// Logo addresses by index ("" when a channel has none).
    pub logos: Arc<Vec<String>>,
    /// Recently watched channels, most recent first.
    pub recent: Vec<usize>,
    /// Channels shown at all (working, in the chosen countries / languages).
    pub listed: Vec<bool>,
    pub favorites: Vec<bool>,
    pub current: usize,
    /// Volume in percent, if the player reports it.
    pub volume: Option<u32>,
    pub muted: bool,
    pub audio: Vec<TrackInfo>,
    pub subs: Vec<TrackInfo>,
}

impl Snapshot {
    pub fn from_tuner(tuner: &Tuner) -> Self {
        let mut s = Self {
            names: Arc::new(tuner.channels().iter().map(|c| c.name.clone()).collect()),
            search: Arc::new(tuner.search_names().to_vec()),
            logos: Arc::new(
                tuner
                    .channels()
                    .iter()
                    .map(|c| {
                        c.logo
                            .clone()
                            .filter(|l| l.starts_with("http"))
                            .unwrap_or_default()
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        s.refresh(tuner);
        s
    }

    /// Re-reads what changes as you watch: the channel, favorites and which
    /// channels are listed.
    pub fn refresh(&mut self, tuner: &Tuner) {
        let n = self.names.len();
        self.listed = (0..n).map(|i| tuner.is_listed(i)).collect();
        self.favorites = (0..n).map(|i| tuner.is_favorite(i)).collect();
        self.current = tuner.current();
        self.recent = tuner.recent();
    }
}

struct State {
    snapshot: Snapshot,
    /// Bumped on every update; the page sends the last one it saw.
    seq: u64,
}

#[derive(Default)]
struct Lockout {
    failures: VecDeque<Instant>,
    until: Option<Instant>,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    pin: String,
    commands: Mutex<mpsc::Sender<Command>>,
    lockout: Mutex<Lockout>,
}

/// The running remote server.
pub struct Remote {
    shared: Arc<Shared>,
    port: u16,
}

impl Remote {
    /// Starts serving on the home network. Commands from phones arrive on
    /// the returned receiver.
    pub fn start(pin: String, snapshot: Snapshot) -> io::Result<(Self, mpsc::Receiver<Command>)> {
        // IPTV_REMOTE_LOCAL: only this computer (for testing).
        let host = if std::env::var_os("IPTV_REMOTE_LOCAL").is_some() {
            "127.0.0.1"
        } else {
            "0.0.0.0"
        };
        let listener = (0..10)
            .find_map(|i| TcpListener::bind((host, PORT + i)).ok())
            .map_or_else(|| TcpListener::bind((host, 0)), Ok)?;
        let port = listener.local_addr()?.port();
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State { snapshot, seq: 1 }),
            changed: Condvar::new(),
            pin,
            commands: Mutex::new(tx),
            lockout: Mutex::new(Lockout::default()),
        });
        let accept = shared.clone();
        thread::Builder::new()
            .name("remote".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let shared = accept.clone();
                    let _ = thread::Builder::new()
                        .name("remote-request".into())
                        .spawn(move || {
                            let _ = handle(stream, &shared);
                        });
                }
            })?;
        Ok((Self { shared, port }, rx))
    }

    /// Changes what phones see; waiting pages refresh right away.
    pub fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let mut st = lock(&self.shared.state);
        f(&mut st.snapshot);
        st.seq += 1;
        self.shared.changed.notify_all();
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn pin(&self) -> &str {
        &self.shared.pin
    }

    /// The address to open on a phone, e.g. `http://192.168.1.20:47123`.
    pub fn address(&self) -> Option<String> {
        lan_ip().map(|ip| format!("http://{ip}:{}", self.port))
    }

    /// The address with the PIN, for the QR code (the PIN is in the
    /// fragment, which browsers keep to themselves).
    pub fn pairing_url(&self) -> Option<String> {
        self.address().map(|a| format!("{a}/#{}", self.shared.pin))
    }
}

/// A new random 6-digit PIN.
pub fn new_pin() -> String {
    let mut b = [0u8; 4];
    if getrandom::fill(&mut b).is_err() {
        // Practically never; fall back to the clock rather than fail.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        b = t.to_le_bytes();
    }
    format!("{:06}", u32::from_le_bytes(b) % 1_000_000)
}

/// This device's address on the home network: the one it would use to reach
/// the internet (no packet is sent).
pub fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?; // TEST-NET address: routing only
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// The QR code for `text`: its width in modules and the dark ones, row by
/// row.
pub fn qr(text: &str) -> Option<(usize, Vec<bool>)> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let dark = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    Some((code.width(), dark))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------ HTTP

struct Request {
    method: String,
    path: String,
    query: String,
    key: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> io::Result<Request> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut reader = BufReader::new(stream.try_clone()?.take(64 * 1024));
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| bad("empty request"))?
        .to_string();
    let target = parts.next().ok_or_else(|| bad("no path"))?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (path, query) = (path.to_string(), query.to_string());
    let mut length = 0usize;
    let mut key = None;
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = h.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.parse().map_err(|_| bad("bad length"))?,
                "x-key" => key = Some(value.to_string()),
                _ => {}
            }
        }
    }
    if length > 16 * 1024 {
        return Err(bad("body too large"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        query,
        key,
        body,
    })
}

fn respond(
    mut stream: &TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_json(stream: &TcpStream, status: &str, value: serde_json::Value) -> io::Result<()> {
    respond(
        stream,
        status,
        "application/json",
        value.to_string().as_bytes(),
    )
}

fn handle(stream: TcpStream, shared: &Shared) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let req = read_request(&stream)?;
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/" | "/index.html") => {
            return respond(
                &stream,
                "200 OK",
                "text/html; charset=utf-8",
                PAGE.as_bytes(),
            )
        }
        (_, p) if !p.starts_with("/api/") => {
            return respond(&stream, "404 Not Found", "text/plain", b"not found")
        }
        _ => {}
    }
    if let Err(status) = authorize(shared, req.key.as_deref()) {
        return respond_json(&stream, status, json!({ "error": status }));
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/state") => {
            let seen: u64 = param(&req.query, "seq")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            respond_json(&stream, "200 OK", state_json(shared, seen))
        }
        ("GET", "/api/list") => {
            let q = param(&req.query, "q").unwrap_or_default();
            let show = match (
                param(&req.query, "favs").as_deref(),
                param(&req.query, "recent").as_deref(),
            ) {
                (Some("1"), _) => Show::Favorites,
                (_, Some("1")) => Show::Recent,
                _ => Show::All,
            };
            respond_json(&stream, "200 OK", list_json(shared, &q, show))
        }
        ("POST", "/api/cmd") => match serde_json::from_slice::<Command>(&req.body) {
            Ok(cmd) if valid(shared, &cmd) => {
                let _ = lock(&shared.commands).send(cmd);
                respond_json(&stream, "200 OK", json!({}))
            }
            _ => respond_json(
                &stream,
                "400 Bad Request",
                json!({ "error": "bad command" }),
            ),
        },
        _ => respond(&stream, "404 Not Found", "text/plain", b"not found"),
    }
}

/// Checks the PIN; too many wrong ones lock the API for a while.
fn authorize(shared: &Shared, key: Option<&str>) -> Result<(), &'static str> {
    let now = Instant::now();
    let mut l = lock(&shared.lockout);
    if l.until.is_some_and(|t| now < t) {
        return Err("429 Too Many Requests");
    }
    if key == Some(shared.pin.as_str()) {
        return Ok(());
    }
    l.failures.push_back(now);
    while l
        .failures
        .front()
        .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
    {
        l.failures.pop_front();
    }
    if l.failures.len() >= MAX_FAILURES {
        l.failures.clear();
        l.until = Some(now + LOCKOUT);
    }
    Err("401 Unauthorized")
}

fn valid(shared: &Shared, cmd: &Command) -> bool {
    let n = lock(&shared.state).snapshot.names.len();
    match cmd {
        Command::Tune { idx } | Command::Fav { idx } => *idx < n,
        Command::Volume { delta } => delta.abs() <= 100,
        Command::Action { name, arg } => {
            name.len() < 32 && arg.as_ref().is_none_or(|a| a.len() < 32)
        }
        Command::Mute | Command::Track { .. } => true,
    }
}

/// The state, once it differs from what the page saw last (`seen`), or
/// after [`LONG_POLL`].
fn state_json(shared: &Shared, seen: u64) -> serde_json::Value {
    let st = lock(&shared.state);
    let (st, _) = shared
        .changed
        .wait_timeout_while(st, LONG_POLL, |st| st.seq == seen)
        .unwrap_or_else(|e| e.into_inner());
    let s = &st.snapshot;
    let cur = s.current.min(s.names.len().saturating_sub(1));
    json!({
        "seq": st.seq,
        "count": s.listed.iter().filter(|l| **l).count(),
        "current": {
            "idx": cur,
            "number": cur + 1,
            "name": s.names.get(cur),
            "fav": s.favorites.get(cur).copied().unwrap_or(false),
            "logo": s.logos.get(cur),
        },
        "volume": s.volume,
        "muted": s.muted,
        "audio": s.audio,
        "subs": s.subs,
    })
}

/// Which channels the phone's list shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Show {
    All,
    Favorites,
    Recent,
}

/// Listed channels matching a search: `[[idx, name, favorite, logo], ...]`
/// (recently watched: most recent first; otherwise by number).
fn list_json(shared: &Shared, query: &str, show: Show) -> serde_json::Value {
    let words: Vec<String> = normalize(query)
        .split_whitespace()
        .map(String::from)
        .collect();
    let st = lock(&shared.state);
    let s = &st.snapshot;
    let order: Vec<usize> = match show {
        Show::Recent => s.recent.clone(),
        _ => (0..s.names.len()).collect(),
    };
    let fav = |i: usize| s.favorites.get(i).copied().unwrap_or(false);
    let items: Vec<serde_json::Value> = order
        .into_iter()
        .filter(|&i| i < s.names.len() && s.listed.get(i).copied().unwrap_or(false))
        .filter(|&i| show != Show::Favorites || fav(i))
        .filter(|&i| words.iter().all(|w| s.search[i].contains(w.as_str())))
        .map(|i| {
            json!([
                i,
                s.names[i],
                fav(i),
                s.logos.get(i).map(String::as_str).unwrap_or("")
            ])
        })
        .collect();
    json!({ "items": items })
}

/// A query-string parameter, percent-decoded.
fn param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == name).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi * 16 + lo) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        let names = vec![
            "Rai 1".to_string(),
            "Canal Sur".to_string(),
            "Télé Nord".to_string(),
        ];
        Snapshot {
            search: Arc::new(names.iter().map(|n| normalize(n)).collect()),
            names: Arc::new(names),
            listed: vec![true, true, true],
            favorites: vec![false, true, false],
            recent: vec![2, 0],
            ..Default::default()
        }
    }

    fn call(port: u16, req: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn serves_page_and_checks_pin() {
        let (remote, rx) = Remote::start("123456".into(), snapshot()).unwrap();
        let p = remote.port();
        assert!(call(p, "GET / HTTP/1.1\r\n\r\n").contains("200 OK"));
        assert!(call(p, "GET /api/state HTTP/1.1\r\nX-Key: 000000\r\n\r\n").contains("401"));
        let state = call(p, "GET /api/state HTTP/1.1\r\nX-Key: 123456\r\n\r\n");
        assert!(state.contains("\"name\":\"Rai 1\""), "{state}");

        let list = call(p, "GET /api/list?q=tele HTTP/1.1\r\nX-Key: 123456\r\n\r\n");
        assert!(
            list.contains("Télé Nord") && !list.contains("Rai 1"),
            "{list}"
        );
        let favs = call(p, "GET /api/list?favs=1 HTTP/1.1\r\nX-Key: 123456\r\n\r\n");
        assert!(
            favs.contains("Canal Sur") && !favs.contains("Rai 1"),
            "{favs}"
        );
        let recent = call(
            p,
            "GET /api/list?recent=1 HTTP/1.1\r\nX-Key: 123456\r\n\r\n",
        );
        let (t, r) = (
            recent.find("Télé Nord").unwrap(),
            recent.find("Rai 1").unwrap(),
        );
        assert!(
            t < r && !recent.contains("Canal Sur"),
            "most recent first: {recent}"
        );

        let body = r#"{"cmd":"tune","idx":2}"#;
        let r = call(
            p,
            &format!(
                "POST /api/cmd HTTP/1.1\r\nX-Key: 123456\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(r.contains("200 OK"), "{r}");
        assert_eq!(rx.recv().unwrap(), Command::Tune { idx: 2 });
        let body = r#"{"cmd":"tune","idx":99}"#;
        let r = call(
            p,
            &format!(
                "POST /api/cmd HTTP/1.1\r\nX-Key: 123456\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(r.contains("400"), "{r}");
    }

    #[test]
    fn wrong_pins_lock_the_api() {
        let (remote, _rx) = Remote::start("654321".into(), snapshot()).unwrap();
        for _ in 0..MAX_FAILURES {
            call(remote.port(), "GET /api/state HTTP/1.1\r\nX-Key: 1\r\n\r\n");
        }
        // Even the right PIN is refused while locked.
        let r = call(
            remote.port(),
            "GET /api/state HTTP/1.1\r\nX-Key: 654321\r\n\r\n",
        );
        assert!(r.contains("429"), "{r}");
    }

    #[test]
    fn long_poll_returns_on_change() {
        let (remote, _rx) = Remote::start("111111".into(), snapshot()).unwrap();
        let p = remote.port();
        let first = call(p, "GET /api/state HTTP/1.1\r\nX-Key: 111111\r\n\r\n");
        assert!(first.contains("\"seq\":1"));
        let started = Instant::now();
        let waiter = thread::spawn(move || {
            call(p, "GET /api/state?seq=1 HTTP/1.1\r\nX-Key: 111111\r\n\r\n")
        });
        thread::sleep(Duration::from_millis(300));
        remote.update(|s| s.current = 1);
        let r = waiter.join().unwrap();
        assert!(
            r.contains("Canal Sur") && started.elapsed() < Duration::from_secs(5),
            "{r}"
        );
    }

    #[test]
    fn decodes_queries_and_makes_pins_and_codes() {
        assert_eq!(
            param("q=t%C3%A9l%C3%A9+nord&favs=1", "q").unwrap(),
            "télé nord"
        );
        assert_eq!(param("favs=1", "favs").unwrap(), "1");
        assert_eq!(percent_decode("100%"), "100%");
        let pin = new_pin();
        assert!(pin.len() == 6 && pin.chars().all(|c| c.is_ascii_digit()));
        let (w, dark) = qr("http://192.168.1.20:47123/#123456").unwrap();
        assert_eq!(dark.len(), w * w);
    }
}
