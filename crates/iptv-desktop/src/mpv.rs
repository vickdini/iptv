//! Runs mpv as the video window and talks to it over its JSON IPC protocol
//! (a named pipe on Windows, a Unix socket elsewhere).
//!
//! Key presses in the mpv window are bound (via a generated input.conf) to
//! `script-message iptv <action> [arg]`, which mpv forwards to us as
//! `client-message` events.

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, WriteHalf},
    sync::mpsc,
};

const INPUT_CONF: &str = "\
UP          script-message iptv up
DOWN        script-message iptv down
PGUP        script-message iptv page-up
PGDWN       script-message iptv page-down
HOME        script-message iptv home
END         script-message iptv end
WHEEL_UP    script-message iptv wheel-up
WHEEL_DOWN  script-message iptv wheel-down
Shift+WHEEL_UP   script-message iptv page-up
Shift+WHEEL_DOWN script-message iptv page-down
ESC         script-message iptv back
MBTN_LEFT   script-message iptv click
MBTN_RIGHT  script-message iptv list
NEXT        script-message iptv up
PREV        script-message iptv down
LEFT        add volume -5
RIGHT       add volume 5
ENTER       script-message iptv ok
KP_ENTER    script-message iptv ok
BS          script-message iptv prev
TAB         script-message iptv list
F1          script-message iptv help
Ctrl+f      script-message iptv search
DEL         script-message iptv clear
MBTN_LEFT_DBL script-message iptv dblclick
";

pub enum Event {
    /// `script-message` arguments sent from a key binding.
    ClientMessage(Vec<String>),
    /// A stream failed. `entry_id` is mpv's playlist entry id: entries are
    /// numbered 1, 2, 3... in `loadfile` order, which identifies the stream.
    PlaybackError {
        entry_id: Option<u64>,
        error: String,
    },
    /// The stream started decoding.
    FileLoaded,
    /// Mouse position in window pixels; `hover` is false outside the window.
    MousePos {
        x: f64,
        y: f64,
        hover: bool,
    },
    /// Window (OSD) height in pixels.
    OsdSize {
        h: f64,
    },
    /// The current stream's audio and subtitle tracks.
    Tracks(Vec<Track>),
    /// Volume in percent.
    Volume(f64),
    Muted(bool),
    /// mpv exited or the IPC connection closed.
    Closed,
}

/// An audio or subtitle track of the current stream.
#[derive(Clone, Debug)]
pub struct Track {
    /// mpv's track id (`aid` / `sid` value).
    pub id: i64,
    pub kind: TrackKind,
    /// Language code from the stream ("spa", "en"...), may be empty.
    pub lang: String,
    /// Name given by the broadcaster ("Castellano"), may be empty.
    pub title: String,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Sub,
}

impl Track {
    /// What to show the user: the broadcaster's name for the track, else
    /// the language name.
    pub fn label(&self) -> String {
        let lang = iptv_core::lang::language_name(&self.lang);
        let title = self.title.trim();
        if title.is_empty() {
            lang
        } else if self.lang.is_empty() || lang.eq_ignore_ascii_case(title) || lang == "Unknown" {
            title.to_string()
        } else {
            format!("{title} ({lang})")
        }
    }
}

fn parse_tracks(list: &Value) -> Vec<Track> {
    let Some(list) = list.as_array() else {
        return vec![];
    };
    list.iter()
        .filter_map(|t| {
            let kind = match t["type"].as_str()? {
                "audio" => TrackKind::Audio,
                "sub" => TrackKind::Sub,
                _ => return None,
            };
            Some(Track {
                id: t["id"].as_i64()?,
                kind,
                lang: t["lang"].as_str().unwrap_or_default().to_string(),
                title: t["title"].as_str().unwrap_or_default().to_string(),
                selected: t["selected"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub struct Mpv {
    child: Child,
    writer: WriteHalf<Box<dyn Io>>,
    #[cfg(unix)]
    socket_path: PathBuf,
}

pub struct Options {
    pub fullscreen: bool,
    pub maximized: bool,
    pub volume: Option<u8>,
    /// The small picture: a silent, borderless window that stays on top in
    /// the bottom-right corner of the screen.
    pub small: bool,
}

impl Mpv {
    pub async fn spawn(
        mpv: &Path,
        opts: &Options,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Event>)> {
        let ipc = ipc_path(if opts.small { "-small" } else { "" });
        let input_conf = write_input_conf()?;

        let mut cmd = Command::new(mpv);
        // Ignore any personal mpv configuration so the app behaves the same
        // everywhere; our own key bindings come from --input-conf.
        cmd.arg("--config=no")
            .arg("--idle=yes")
            .arg("--force-window=yes")
            .arg("--keep-open=no")
            .arg(format!("--input-ipc-server={}", ipc.display()))
            .arg(format!("--input-conf={}", input_conf.display()))
            .arg("--title=${?force-media-title:${force-media-title}}${!force-media-title:IPTV} — IPTV")
            // No "Drop files or URLs" idle logo when a channel fails.
            .arg("--script-opts=osc-idlescreen=no")
            .arg("--osd-level=1")
            .arg("--osd-font-size=36")
            .arg("--osd-align-x=left")
            .arg("--osd-align-y=top")
            .arg("--network-timeout=10")
            .arg("--cache=yes")
            .arg("--demuxer-max-bytes=64MiB")
            .arg("--msg-level=all=error")
            .arg("--terminal=no")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if opts.small {
            // A quarter of the screen wide, 24 px from the right edge, above the taskbar.
            cmd.args([
                "--ontop",
                "--no-border",
                "--geometry=25%-24-64",
                "--mute=yes",
                "--osd-level=0",
            ]);
        }
        if opts.fullscreen {
            cmd.arg("--fullscreen");
        }
        if opts.maximized {
            cmd.arg("--window-maximized=yes");
        }
        if let Some(v) = opts.volume {
            cmd.arg(format!("--volume={v}"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to start mpv at {}", mpv.display()))?;

        // Wait for mpv to create the IPC endpoint.
        let mut attempts = 0;
        let stream = loop {
            match connect(&ipc).await {
                Ok(s) => break s,
                Err(e) => {
                    if let Some(status) = child.try_wait()? {
                        bail!("mpv exited during startup ({status})");
                    }
                    attempts += 1;
                    if attempts > 100 {
                        let _ = child.kill();
                        bail!("could not connect to mpv IPC at {}: {e}", ipc.display());
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        };

        let (reader, writer) = tokio::io::split(stream);
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(read_events(reader, tx));

        let mut mpv = Self {
            child,
            writer,
            #[cfg(unix)]
            socket_path: ipc,
        };
        mpv.command(json!(["observe_property", 1, "mouse-pos"]))
            .await?;
        mpv.command(json!(["observe_property", 2, "osd-dimensions"]))
            .await?;
        mpv.command(json!(["observe_property", 3, "track-list"]))
            .await?;
        mpv.command(json!(["observe_property", 4, "volume"]))
            .await?;
        mpv.command(json!(["observe_property", 5, "mute"])).await?;
        Ok((mpv, rx))
    }

    pub async fn command(&mut self, args: Value) -> Result<()> {
        let mut line = serde_json::to_vec(&json!({ "command": args }))?;
        line.push(b'\n');
        self.writer.write_all(&line).await?;
        self.writer.flush().await?;
        Ok(())
    }

    pub async fn set_property(&mut self, name: &str, value: Value) -> Result<()> {
        self.command(json!(["set_property", name, value])).await
    }

    pub async fn show_text(&mut self, text: &str, ms: u32) -> Result<()> {
        self.command(json!(["show-text", text, ms])).await
    }

    /// Shows (or replaces) an ASS overlay on a `res_y`-tall virtual canvas.
    pub async fn overlay(&mut self, id: u32, ass: &str, res_y: f64) -> Result<()> {
        self.command(json!({
            "name": "osd-overlay", "id": id, "format": "ass-events",
            "data": ass, "res_x": 0, "res_y": res_y.round() as i64,
        }))
        .await
    }

    pub async fn remove_overlay(&mut self, id: u32) -> Result<()> {
        self.command(json!({ "name": "osd-overlay", "id": id, "format": "none", "data": "" }))
            .await
    }

    /// Preferred audio / subtitle languages for the next streams (lists of
    /// language codes); no subtitle languages = subtitles off.
    pub async fn set_track_prefs(&mut self, alang: &[String], slang: &[String]) -> Result<()> {
        self.set_property("alang", json!(alang.join(","))).await?;
        self.set_property("slang", json!(slang.join(","))).await?;
        self.set_property("aid", json!("auto")).await?;
        self.set_property("sid", json!(if slang.is_empty() { "no" } else { "auto" }))
            .await
    }

    pub async fn play(
        &mut self,
        url: &str,
        title: &str,
        user_agent: &str,
        referrer: &str,
    ) -> Result<()> {
        // Set per-stream options as properties first: this works on every
        // mpv version, unlike the loadfile per-file options syntax.
        self.set_property("user-agent", json!(user_agent)).await?;
        self.set_property("referrer", json!(referrer)).await?;
        self.set_property("force-media-title", json!(title)).await?;
        self.command(json!(["loadfile", url, "replace"])).await
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn read_events(reader: impl AsyncRead + Unpin, tx: mpsc::UnboundedSender<Event>) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(err) = msg.get("error").and_then(Value::as_str) {
            if err != "success" {
                eprintln!("mpv command failed: {err}");
            }
            continue;
        }
        let event = match msg.get("event").and_then(Value::as_str) {
            Some("client-message") => {
                let args = msg["args"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                Event::ClientMessage(args)
            }
            Some("end-file") if msg["reason"] == "error" => Event::PlaybackError {
                entry_id: msg["playlist_entry_id"].as_u64(),
                error: msg["file_error"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_string(),
            },
            Some("file-loaded") => Event::FileLoaded,
            Some("property-change") => {
                let d = &msg["data"];
                let num = |k: &str| d[k].as_f64();
                match msg["name"].as_str() {
                    Some("mouse-pos") => match (num("x"), num("y")) {
                        (Some(x), Some(y)) => Event::MousePos {
                            x,
                            y,
                            hover: d["hover"].as_bool().unwrap_or(true),
                        },
                        _ => continue,
                    },
                    Some("osd-dimensions") => match (num("w"), num("h")) {
                        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Event::OsdSize { h },
                        _ => continue,
                    },
                    Some("track-list") => Event::Tracks(parse_tracks(d)),
                    Some("volume") => match d.as_f64() {
                        Some(v) => Event::Volume(v),
                        None => continue,
                    },
                    Some("mute") => match d.as_bool() {
                        Some(m) => Event::Muted(m),
                        None => continue,
                    },
                    _ => continue,
                }
            }
            Some("shutdown") => Event::Closed,
            _ => continue,
        };
        if tx.send(event).is_err() {
            return;
        }
    }
    let _ = tx.send(Event::Closed);
}

fn write_input_conf() -> Result<PathBuf> {
    let dir = crate::storage::config_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("mpv-input.conf");
    let mut conf = INPUT_CONF.to_string();
    for d in 0..=9 {
        conf.push_str(&format!("{d}           script-message iptv digit {d}\n"));
        conf.push_str(&format!("KP{d}         script-message iptv digit {d}\n"));
    }
    // Letters and common symbols all go to the app, which decides: text while
    // searching, otherwise shortcuts (see main.rs `shortcut`).
    for c in ('a'..='z').chain('A'..='Z').chain("/?-+.&'!():[]".chars()) {
        conf.push_str(&format!("{c}           script-message iptv key {c}\n"));
    }
    conf.push_str("SPACE       script-message iptv key space\n");
    std::fs::write(&path, conf)?;
    Ok(path)
}

#[cfg(windows)]
fn ipc_path(tag: &str) -> PathBuf {
    PathBuf::from(format!(r"\\.\pipe\iptv-mpv-{}{tag}", std::process::id()))
}

#[cfg(unix)]
fn ipc_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("iptv-mpv-{}{tag}.sock", std::process::id()))
}

#[cfg(windows)]
async fn connect(path: &Path) -> std::io::Result<Box<dyn Io>> {
    let client = tokio::net::windows::named_pipe::ClientOptions::new().open(path)?;
    Ok(Box::new(client))
}

#[cfg(unix)]
async fn connect(path: &Path) -> std::io::Result<Box<dyn Io>> {
    Ok(Box::new(tokio::net::UnixStream::connect(path).await?))
}

/// Finds the mpv executable: explicit path, `IPTV_MPV`, next to our own
/// binary, `PATH`, then common install locations.
pub fn locate(explicit: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(p);
    }
    if let Some(p) = std::env::var_os("IPTV_MPV") {
        return Some(PathBuf::from(p));
    }
    let exe = if cfg!(windows) { "mpv.exe" } else { "mpv" };

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            // Windows installer: mpv\mpv.exe; Linux AppImage: usr/bin/mpv.
            candidates.push(dir.join(exe));
            candidates.push(dir.join("mpv").join(exe));
            // macOS app: IPTV.app/Contents/{MacOS/iptv, Resources/mpv.app}.
            candidates.push(dir.join("../Resources/mpv.app/Contents/MacOS/mpv"));
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|d| d.join(exe)));
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                candidates.push(PathBuf::from(&base).join("mpv").join(exe));
                // winget's shinchiro.mpv installer location.
                candidates.push(PathBuf::from(&base).join("MPV Player").join(exe));
            }
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(
                home.join("scoop")
                    .join("apps")
                    .join("mpv")
                    .join("current")
                    .join(exe),
            );
        }
    } else if cfg!(target_os = "macos") {
        candidates.extend(
            [
                "/opt/homebrew/bin/mpv",
                "/usr/local/bin/mpv",
                "/Applications/mpv.app/Contents/MacOS/mpv",
            ]
            .map(PathBuf::from),
        );
    } else {
        candidates
            .extend(["/usr/bin/mpv", "/usr/local/bin/mpv", "/snap/bin/mpv"].map(PathBuf::from));
    }
    candidates.into_iter().find(|p| p.is_file())
}
