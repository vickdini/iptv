// Release builds on Windows are GUI apps: no console window pops up.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod gui;
mod mpv;
mod panel;
mod storage;
mod updater;

use std::{path::PathBuf, time::Duration};

use anyhow::{bail, Context, Result};
use clap::Parser;
use iptv_core::{lang::language_codes, tuner::DIGIT_TIMEOUT_MS, Action, Effect, Tuner};
use iptv_net::{health, playlist, DEFAULT_USER_AGENT};
use iptv_remote::{self as remote, Remote};
use serde_json::json;
use tokio::time::Instant;

use crate::{
    health::{CheckResult, HealthCache, Probe},
    mpv::{Event, Mpv, Track, TrackKind},
    panel::{Hit, Panel, Picker, PickerTab},
};

const DEFAULT_PLAYLIST: &str = "https://iptv-org.github.io/iptv/index.m3u";
const PANEL_OVERLAY_ID: u32 = 1;
/// The "this channel has other languages" hint at the bottom.
const HINT_OVERLAY_ID: u32 = 2;
const HINT_MS: u64 = 6000;
/// The phone remote's QR card.
const REMOTE_OVERLAY_ID: u32 = 3;
const WHEEL_ROWS: isize = 3;

/// Watch IPTV like a TV: zap channels with Up/Down, type channel numbers,
/// jump back with Backspace. Press 'h' in the player window for all keys.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Playlist URL or local .m3u file.
    #[arg(short, long, default_value = DEFAULT_PLAYLIST)]
    playlist: String,

    /// Path to the mpv executable (default: search PATH and common locations).
    #[arg(long, env = "IPTV_MPV")]
    mpv: Option<PathBuf>,

    /// Start on this channel number.
    #[arg(short, long)]
    channel: Option<usize>,

    /// Only zap through this group (e.g. "News", "Movies").
    #[arg(short, long)]
    group: Option<String>,

    /// List the groups in the playlist and exit.
    #[arg(long)]
    list_groups: bool,

    /// List channels (optionally filtered by text) and exit.
    #[arg(long, value_name = "FILTER", num_args = 0..=1, default_missing_value = "")]
    list: Option<String>,

    /// Start in fullscreen.
    #[arg(short, long)]
    fullscreen: bool,

    /// Start in a normal-sized window instead of maximized.
    #[arg(long)]
    no_maximize: bool,

    /// Don't check which channels work (show them all).
    #[arg(long)]
    no_check: bool,

    /// Don't check for or install new versions.
    #[arg(long)]
    no_update: bool,

    /// Don't start the phone remote (even if it was turned on before).
    #[arg(long)]
    no_remote: bool,

    /// Initial volume (0-100).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
    volume: Option<u8>,
}

fn main() {
    let has_console = gui::attach_parent_console();
    let args = Args::parse();
    let runtime = tokio::runtime::Runtime::new().expect("failed to start async runtime");
    if let Err(e) = runtime.block_on(run(args)) {
        eprintln!("Error: {e:#}");
        if !has_console {
            gui::error_box(&format!("{e:#}"));
        }
        drop(runtime);
        std::process::exit(1);
    }
}

async fn load_channels(source: &str) -> Result<Vec<iptv_core::Channel>> {
    let source = source.to_string();
    let started = std::time::Instant::now();
    let (channels, freshness) =
        tokio::task::spawn_blocking(move || playlist::load(&source, &storage::cache_dir()))
            .await??;
    let what = match freshness {
        playlist::Freshness::UpToDate => "already up to date",
        playlist::Freshness::Downloaded => "downloaded new version",
        playlist::Freshness::Offline => "offline, using saved copy",
        playlist::Freshness::Local => "local file",
    };
    eprintln!(
        "Playlist: {what} ({} channels, {:.0?})",
        channels.len(),
        started.elapsed()
    );
    if channels.is_empty() {
        bail!("The playlist contains no channels.");
    }
    Ok(channels)
}

async fn run(args: Args) -> Result<()> {
    if args.list_groups || args.list.is_some() {
        let channels = load_channels(&args.playlist).await?;
        let tuner = Tuner::new(channels, &Default::default()).expect("non-empty playlist");
        if args.list_groups {
            for g in tuner.groups() {
                let n = tuner
                    .channels()
                    .iter()
                    .filter(|c| c.groups.contains(g))
                    .count();
                println!("{g} ({n})");
            }
        }
        if let Some(filter) = &args.list {
            let filter = filter.to_lowercase();
            for (i, c) in tuner.channels().iter().enumerate() {
                if c.name.to_lowercase().contains(&filter) {
                    println!("{:>6}  {}  [{}]", i + 1, c.name, c.groups.join(", "));
                }
            }
        }
        return Ok(());
    }

    let mpv_path = mpv::locate(args.mpv.clone()).context(
        "The video player component (mpv) was not found.

         If you installed IPTV with its installer, reinstalling it should fix this.          Otherwise install mpv (Windows: `winget install shinchiro.mpv`, macOS: `brew install mpv`,          Linux: your package manager) or pass --mpv <path>.",
    )?;

    // Open the player window right away so the user sees something while
    // the (multi-megabyte) playlist downloads.
    let opts = mpv::Options {
        fullscreen: args.fullscreen,
        maximized: !args.no_maximize,
        volume: args.volume,
    };
    let (mut player, mut events) = Mpv::spawn(&mpv_path, &opts).await?;
    player.show_text("Loading channels…", 120_000).await?;

    let channels = load_channels(&args.playlist)
        .await
        .context("Could not load the channel list. Check your internet connection.")?;

    let mut state = storage::load_state();
    if let Some(n) = args.channel {
        if n == 0 || n > channels.len() {
            bail!("Channel {n} does not exist (1-{}).", channels.len());
        }
        state.last_url = None;
        state.last_index = n - 1;
    }
    let mut tuner = Tuner::new(channels, &state).expect("non-empty playlist");
    if let Some(g) = &args.group {
        if !tuner.set_group(g) {
            bail!("Unknown group {g:?}; use --list-groups to see the available ones.");
        }
    }
    eprintln!(
        "Loaded {} channels. Using mpv at {}",
        tuner.channels().len(),
        mpv_path.display()
    );
    eprintln!(
        "Press 'h' in the player window for the remote-control keys, 'l' for the channel list."
    );

    // Hide channels already known to be dead; queue the rest for checking,
    // nearest to the current channel first.
    let health = HealthCache::load(&storage::cache_dir());
    let mut jobs = Vec::new();
    for idx in 0..tuner.channels().len() {
        match health.get(&tuner.channels()[idx].url) {
            Some((ok, fresh)) => {
                tuner.set_dead(idx, !ok);
                if !fresh {
                    jobs.push(idx);
                }
            }
            None => jobs.push(idx),
        }
    }
    let n = tuner.channels().len();
    let cur = tuner.current();
    jobs.sort_by_key(|&i| (i + n - cur) % n);
    let check_total = jobs.len();
    let (check_tx, mut checks) = tokio::sync::mpsc::unbounded_channel::<CheckResult>();
    if !args.no_check {
        let jobs = jobs
            .into_iter()
            .map(|i| (i, tuner.channels()[i].clone()))
            .collect();
        health::spawn_checker(jobs, DEFAULT_USER_AGENT, move |r| check_tx.send(r).is_ok());
    } else {
        drop(check_tx);
    }
    eprintln!(
        "Channel check: {} known dead, {} to check",
        (0..n).filter(|&i| tuner.is_dead(i)).count(),
        if args.no_check { 0 } else { check_total }
    );

    // Phone remote requests, forwarded from the remote's server threads.
    let (remote_tx, mut remote_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App {
        player,
        tuner,
        panel: None,
        digit_deadline: None,
        mouse: None,
        osd_h: 720.0,
        debug: std::env::var_os("IPTV_DEBUG").is_some(),
        health,
        check_total: if args.no_check { 0 } else { check_total },
        check_done: 0,
        view_dirty: false,
        tune_seq: 0,
        current_loaded: false,
        typing: false,
        query: String::new(),
        picker: None,
        tracks: Vec::new(),
        hinted_seq: 0,
        hint_until: None,
        remote: None,
        remote_tx,
        remote_card: false,
        volume: None,
        muted: false,
    };
    let fx = app.tuner.start();
    app.apply(fx).await?;
    // Once turned on (r), the phone remote starts with the app, so a phone
    // that bookmarked it just works.
    if !args.no_remote && app.tuner.remote_pin().is_some() {
        if let Err(e) = app.start_remote() {
            eprintln!("warning: phone remote not available: {e:#}");
        }
    }

    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut last_save = Instant::now();
    let mut checks_open = true;

    // Look for a newer version in the background; playback isn't delayed.
    let (update_tx, mut update_rx) = tokio::sync::oneshot::channel();
    let mut update_open = !args.no_update && updater::enabled();
    if update_open {
        updater::clean_old_downloads();
        tokio::task::spawn_blocking(move || {
            let _ = update_tx.send(updater::fetch());
        });
    }
    let mut install_at: Option<(Instant, PathBuf)> = None;

    loop {
        let timer = async {
            match app.digit_deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            ev = events.recv() => match ev {
                None | Some(Event::Closed) => break,
                Some(Event::ClientMessage(args)) => {
                    if args.first().map(String::as_str) != Some("iptv") {
                        continue;
                    }
                    let name = args.get(1).map(String::as_str).unwrap_or_default();
                    app.on_key(name, args.get(2).map(String::as_str)).await?;
                }
                Some(Event::PlaybackError { entry_id, error }) => {
                    // Ignore late errors from a stream the user already left.
                    if entry_id.is_none_or(|id| id == app.tune_seq) {
                        app.on_playback_failed(&error).await?;
                    }
                }
                Some(Event::FileLoaded) => {
                    app.current_loaded = true;
                    let cur = app.tuner.current();
                    let url = app.tuner.channels()[cur].url.clone();
                    app.health.record(&url, true);
                    if app.tuner.is_dead(cur) {
                        app.tuner.set_dead(cur, false);
                        app.view_dirty = true;
                    }
                }
                Some(Event::MousePos { x, y, hover }) => {
                    app.mouse = hover.then_some((x, y));
                    if app.hover_select() {
                        app.redraw().await?;
                    }
                }
                Some(Event::OsdSize { h }) => {
                    app.osd_h = h;
                    app.redraw().await?;
                }
                Some(Event::Tracks(tracks)) => app.on_tracks(tracks).await?,
                Some(Event::Volume(v)) => {
                    app.volume = Some(v);
                    app.sync_remote();
                }
                Some(Event::Muted(m)) => {
                    app.muted = m;
                    app.sync_remote();
                }
            },
            res = checks.recv(), if checks_open => match res {
                Some(r) => app.on_check(r),
                None => {
                    checks_open = false;
                    eprintln!(
                        "Channel check finished: {} of {} channels working",
                        app.tuner.working_count(),
                        app.tuner.channels().len()
                    );
                    app.view_dirty = true;
                }
            },
            Some(cmd) = remote_rx.recv() => app.on_remote(cmd).await?,
            _ = tick.tick() => {
                if app.view_dirty {
                    app.sync_remote();
                }
                if app.view_dirty || (app.panel.is_some() && checks_open) {
                    app.view_dirty = false;
                    app.refresh_panel().await?;
                }
                if app.hint_until.is_some_and(|t| Instant::now() >= t) {
                    app.hint_until = None;
                    app.player.remove_overlay(HINT_OVERLAY_ID).await?;
                }
                if last_save.elapsed() > Duration::from_secs(20) {
                    app.health.save_if_dirty();
                    last_save = Instant::now();
                }
            }
            _ = timer => {
                app.digit_deadline = None;
                app.run(Action::DigitTimeout).await?;
            }
            res = &mut update_rx, if update_open => {
                update_open = false;
                match res {
                    Ok(Ok(Some((update, installer)))) => {
                        eprintln!("Update {} downloaded; installing", update.version);
                        let text = format!("Updating IPTV to version {}…\nIt will restart in a few seconds.", update.version);
                        app.player.show_text(&text, 6000).await?;
                        install_at = Some((Instant::now() + Duration::from_secs(4), installer));
                    }
                    Ok(Ok(None)) => eprintln!("IPTV is up to date ({})", env!("CARGO_PKG_VERSION")),
                    Ok(Err(e)) => eprintln!("warning: update check failed: {e:#}"),
                    Err(_) => {}
                }
            }
            _ = async {
                match &install_at {
                    Some((when, _)) => tokio::time::sleep_until(*when).await,
                    None => std::future::pending().await,
                }
            } => {
                let (_, installer) = install_at.take().expect("set");
                match updater::install(&installer) {
                    Ok(()) => break, // exit so the installer can replace our files
                    Err(e) => eprintln!("warning: could not start the update: {e:#}"),
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    storage::save_state(&app.tuner.state());
    app.health.save_if_dirty();
    Ok(())
}

struct App {
    player: Mpv,
    tuner: Tuner,
    /// The channel list panel, when open.
    panel: Option<Panel>,
    digit_deadline: Option<Instant>,
    /// Mouse position in window pixels while it hovers the window.
    mouse: Option<(f64, f64)>,
    /// Window height in pixels (to map the mouse onto the overlay canvas).
    osd_h: f64,
    debug: bool,
    /// Saved channel check results.
    health: HealthCache,
    /// Background check progress.
    check_total: usize,
    check_done: usize,
    /// Channels appeared/disappeared since the panel was last refreshed.
    view_dirty: bool,
    /// Number of streams loaded so far = mpv playlist entry id of the
    /// current stream.
    tune_seq: u64,
    /// The current stream started playing.
    current_loaded: bool,
    /// Typing a search in the channel list, and the text so far.
    typing: bool,
    query: String,
    /// The country / language chooser, when open (inside the channel list).
    picker: Option<Picker>,
    /// Audio and subtitle tracks of the current stream.
    tracks: Vec<Track>,
    /// `tune_seq` of the stream whose language hint was shown.
    hinted_seq: u64,
    /// When to hide the language hint.
    hint_until: Option<Instant>,
    /// The phone remote's web server, once turned on.
    remote: Option<Remote>,
    /// Where the remote's commands are forwarded (into the main loop).
    remote_tx: tokio::sync::mpsc::UnboundedSender<remote::Command>,
    /// The QR card is showing.
    remote_card: bool,
    volume: Option<f64>,
    muted: bool,
}

/// What a letter key does when not typing a search.
fn shortcut(c: &str) -> Option<&'static str> {
    Some(match c {
        "i" => "info",
        "l" => "list",
        "g" => "group-next",
        "G" => "group-prev",
        "s" => "fav",
        "S" => "fav-only",
        "h" | "?" => "help",
        "/" => "search",
        "c" => "regions",
        "r" => "remote",
        "a" => "audio",
        "t" => "subtitles",
        "f" => "fullscreen",
        "m" => "mute",
        "p" | "space" => "pause",
        "q" | "Q" => "quit",
        _ => return None,
    })
}

impl App {
    fn on_check(&mut self, r: CheckResult) {
        self.check_done += 1;
        let url = self.tuner.channels()[r.idx].url.clone();
        match r.probe {
            Probe::Ok => {
                self.health.record(&url, true);
                if self.tuner.is_dead(r.idx) {
                    self.tuner.set_dead(r.idx, false);
                    self.view_dirty = true;
                }
            }
            // A channel that is actually playing right now works, whatever
            // the probe says; one still loading is left to the player.
            Probe::Dead if r.idx == self.tuner.current() => {}
            Probe::Dead => {
                self.health.record(&url, false);
                if !self.tuner.is_dead(r.idx) {
                    self.tuner.set_dead(r.idx, true);
                    self.view_dirty = true;
                }
            }
            Probe::Unknown => {}
        }
    }

    /// The track list changed. The first time a channel turns out to offer
    /// other languages or subtitles, say so (and how to pick them).
    async fn on_tracks(&mut self, tracks: Vec<Track>) -> Result<()> {
        self.tracks = tracks;
        self.sync_remote();
        let audio = self.choices(TrackKind::Audio).len();
        let subs = self.choices(TrackKind::Sub).len();
        if (audio < 2 && subs == 0) || self.hinted_seq == self.tune_seq {
            return Ok(());
        }
        self.hinted_seq = self.tune_seq;
        let mut parts = Vec::new();
        if audio > 1 {
            parts.push(format!("{audio} audio languages  (a)"));
        }
        if subs > 0 {
            parts.push(format!(
                "{subs} subtitle{}  (t)",
                if subs == 1 { "" } else { "s" }
            ));
        }
        let ass = panel::hint(&format!("This channel has {}", parts.join("   ·   ")));
        self.player.overlay(HINT_OVERLAY_ID, &ass, panel::H).await?;
        self.hint_until = Some(Instant::now() + Duration::from_millis(HINT_MS));
        Ok(())
    }

    /// The distinct tracks of a kind. Some streams list the same language
    /// once per quality level; one entry each is enough.
    fn choices(&self, kind: TrackKind) -> Vec<Track> {
        let mut out: Vec<Track> = Vec::new();
        for t in self.tracks.iter().filter(|t| t.kind == kind) {
            match out
                .iter_mut()
                .find(|o| o.lang == t.lang && o.title == t.title)
            {
                Some(same) => same.selected |= t.selected,
                None => out.push(t.clone()),
            }
        }
        out
    }

    /// `a` / `t`: switches to the next audio track, or the next subtitles
    /// (then off), and remembers the language for other channels.
    async fn cycle_track(&mut self, kind: TrackKind) -> Result<()> {
        let list = self.choices(kind);
        let audio = kind == TrackKind::Audio;
        if (audio && list.len() < 2) || (!audio && list.is_empty()) {
            let msg = if audio {
                "This channel has a single audio language"
            } else {
                "This channel has no subtitles"
            };
            return self.player.show_text(msg, 2500).await;
        }
        let pos = list.iter().position(|t| t.selected);
        // Subtitles cycle through "off" too (position None).
        let next = match (audio, pos) {
            (true, p) => Some((p.map_or(0, |p| p + 1)) % list.len()),
            (false, None) => Some(0),
            (false, Some(p)) => (p + 1 < list.len()).then_some(p + 1),
        };
        self.select_track(kind, next, &list).await
    }

    /// Plays `list[next]` (None: subtitles off), remembers its language for
    /// other channels and says so on screen.
    async fn select_track(
        &mut self,
        kind: TrackKind,
        next: Option<usize>,
        list: &[Track],
    ) -> Result<()> {
        let audio = kind == TrackKind::Audio;
        let (audio_pref, sub_pref) = self.tuner.track_prefs();
        let text = match next {
            Some(i) => {
                let t = &list[i];
                let lang = (!t.lang.is_empty()).then(|| t.lang.clone());
                let prop = if audio { "aid" } else { "sid" };
                self.player.set_property(prop, json!(t.id)).await?;
                let fx = if audio {
                    self.tuner.set_track_prefs(lang.or(audio_pref), sub_pref)
                } else {
                    self.tuner.set_track_prefs(audio_pref, lang)
                };
                self.apply(fx).await?;
                let what = if audio { "Audio" } else { "Subtitles" };
                format!("{what}: {}   ({}/{})", t.label(), i + 1, list.len())
            }
            None => {
                self.player.set_property("sid", json!("no")).await?;
                let fx = self.tuner.set_track_prefs(audio_pref, None);
                self.apply(fx).await?;
                "Subtitles: off".to_string()
            }
        };
        // Update our copy now; mpv's track-list event follows.
        let chosen = next.map(|i| list[i].id);
        for t in self.tracks.iter_mut().filter(|t| t.kind == kind) {
            t.selected = Some(t.id) == chosen;
        }
        self.sync_remote();
        self.player.show_text(&text, 3000).await
    }

    // --- phone remote ---------------------------------------------------------

    /// Starts the remote's web server (creating its PIN the first time).
    fn start_remote(&mut self) -> Result<()> {
        if self.remote.is_some() {
            return Ok(());
        }
        let pin = match self.tuner.remote_pin() {
            Some(p) => p.to_string(),
            None => {
                let pin = remote::new_pin();
                self.tuner.set_remote_pin(Some(pin.clone()));
                storage::save_state(&self.tuner.state());
                pin
            }
        };
        let (server, rx) = Remote::start(pin, remote::Snapshot::from_tuner(&self.tuner))
            .context("could not start the phone remote")?;
        eprintln!(
            "Phone remote: {} (PIN {})",
            server
                .address()
                .unwrap_or_else(|| format!("port {}", server.port())),
            server.pin()
        );
        // The server's threads hand commands over here; forward them into
        // the async main loop.
        let tx = self.remote_tx.clone();
        std::thread::spawn(move || {
            while let Ok(cmd) = rx.recv() {
                if tx.send(cmd).is_err() {
                    break;
                }
            }
        });
        self.remote = Some(server);
        self.sync_remote();
        Ok(())
    }

    /// `r`: turns the remote on and shows how to connect a phone.
    async fn open_remote_card(&mut self) -> Result<()> {
        if let Err(e) = self.start_remote() {
            return self.player.show_text(&format!("{e:#}"), 5000).await;
        }
        if self.panel.is_some() {
            self.close_panel().await?;
        }
        let Some(server) = &self.remote else {
            return Ok(());
        };
        let url = server.pairing_url();
        let code = url.as_deref().and_then(remote::qr);
        let address = server.address().unwrap_or_default();
        let ass = match &code {
            Some(code) => panel::remote_card(Some(panel::Pairing {
                address: &address,
                pin: server.pin(),
                qr: code,
            })),
            None => panel::remote_card(None),
        };
        self.remote_card = true;
        self.player.show_text("", 1).await?;
        self.player.overlay(REMOTE_OVERLAY_ID, &ass, panel::H).await
    }

    async fn close_remote_card(&mut self) -> Result<()> {
        self.remote_card = false;
        self.player.remove_overlay(REMOTE_OVERLAY_ID).await
    }

    /// Tells phones what is on now (channel, favorites, tracks, volume).
    fn sync_remote(&self) {
        let Some(server) = &self.remote else { return };
        let info = |kind| -> Vec<remote::TrackInfo> {
            self.choices(kind)
                .into_iter()
                .map(|t| remote::TrackInfo {
                    id: t.id,
                    label: t.label(),
                    selected: t.selected,
                })
                .collect()
        };
        let (audio, subs) = (info(TrackKind::Audio), info(TrackKind::Sub));
        server.update(|s| {
            s.refresh(&self.tuner);
            s.audio = audio;
            s.subs = subs;
            s.volume = self.volume.map(|v| v.round().max(0.0) as u32);
            s.muted = self.muted;
        });
    }

    /// A request from a phone.
    async fn on_remote(&mut self, cmd: remote::Command) -> Result<()> {
        if self.debug {
            eprintln!("remote: {cmd:?}");
        }
        match cmd {
            remote::Command::Action { name, arg } => match Action::from_name(&name, arg.as_deref())
            {
                Some(action) => self.run(action).await,
                None => Ok(()),
            },
            remote::Command::Tune { idx } => {
                let fx = self.tuner.tune_to(idx);
                self.apply(fx).await
            }
            remote::Command::Fav { idx } => {
                let fx = self.tuner.toggle_favorite_at(idx);
                self.apply(fx).await
            }
            remote::Command::Volume { delta } => {
                self.player
                    .command(json!(["osd-msg-bar", "add", "volume", delta]))
                    .await
            }
            remote::Command::Mute => {
                self.player
                    .command(json!(["osd-msg", "cycle", "mute"]))
                    .await
            }
            remote::Command::Track { kind, id } => {
                let kind = match kind {
                    remote::TrackKind::Audio => TrackKind::Audio,
                    remote::TrackKind::Sub => TrackKind::Sub,
                };
                let list = self.choices(kind);
                match id {
                    Some(id) => match list.iter().position(|t| t.id == id) {
                        Some(i) => self.select_track(kind, Some(i), &list).await,
                        None => Ok(()),
                    },
                    None if kind == TrackKind::Sub => self.select_track(kind, None, &list).await,
                    None => Ok(()),
                }
            }
        }
    }

    /// The current stream failed: hide it and quietly move on.
    async fn on_playback_failed(&mut self, error: &str) -> Result<()> {
        let cur = self.tuner.current();
        let url = self.tuner.channels()[cur].url.clone();
        if self.debug {
            eprintln!("channel {} failed ({error}), skipping", cur + 1);
        }
        self.health.record(&url, false);
        self.view_dirty = true;
        let fx = self.tuner.current_failed();
        self.apply(fx).await
    }

    /// Re-reads the visible channel list (channels found dead/alive) without
    /// moving the user's scroll position, and redraws progress.
    async fn refresh_panel(&mut self) -> Result<()> {
        if let Some(panel) = self.panel.as_mut() {
            panel.browser.replace_items(self.tuner.view());
        }
        self.redraw().await
    }

    fn check_status(&self) -> Option<String> {
        (self.check_done < self.check_total).then(|| {
            format!(
                "checking channels {}%",
                self.check_done * 100 / self.check_total.max(1)
            )
        })
    }

    async fn on_key(&mut self, name: &str, arg: Option<&str>) -> Result<()> {
        if self.debug {
            eprintln!("key: {name} {}", arg.unwrap_or_default());
        }
        if self.remote_card {
            return self.close_remote_card().await;
        }
        if self.picker.is_some() {
            return self.picker_key(name, arg).await;
        }
        if self.typing {
            return self.search_key(name, arg).await;
        }
        // Letter keys are shortcuts unless typing a search.
        let (name, arg) = match (name, arg) {
            ("key", Some(c)) => match shortcut(c) {
                Some(n) => (n, None),
                None => return Ok(()),
            },
            other => other,
        };
        match name {
            "search" => return self.start_search().await,
            "regions" => return self.open_picker().await,
            "remote" => return self.open_remote_card().await,
            "fullscreen" => return self.player.command(json!(["cycle", "fullscreen"])).await,
            "mute" => return self.player.command(json!(["cycle", "mute"])).await,
            "audio" => return self.cycle_track(TrackKind::Audio).await,
            "subtitles" => return self.cycle_track(TrackKind::Sub).await,
            "pause" => return self.player.command(json!(["cycle", "pause"])).await,
            "quit" => return self.player.command(json!(["quit"])).await,
            _ => {}
        }
        if self.panel.is_some() {
            self.panel_key(name, arg).await
        } else {
            self.tv_key(name, arg).await
        }
    }

    /// Keys while watching (no panel).
    async fn tv_key(&mut self, name: &str, arg: Option<&str>) -> Result<()> {
        let action = match name {
            "list" => return self.open_panel().await,
            "wheel-up" | "page-up" => Action::ChannelUp,
            "wheel-down" | "page-down" => Action::ChannelDown,
            "back" => return self.player.set_property("fullscreen", json!(false)).await,
            "dblclick" => return self.player.command(json!(["cycle", "fullscreen"])).await,
            "click" | "home" | "end" => return Ok(()),
            _ => match Action::from_name(name, arg) {
                Some(a) => a,
                None => {
                    eprintln!("unknown action {name:?}");
                    return Ok(());
                }
            },
        };
        self.run(action).await
    }

    /// Keys while the channel panel is open.
    async fn panel_key(&mut self, name: &str, arg: Option<&str>) -> Result<()> {
        let Some(panel) = self.panel.as_mut() else {
            return Ok(());
        };
        let b = &mut panel.browser;
        match name {
            "up" => b.move_by(-1),
            "down" => b.move_by(1),
            "page-up" => b.page(-1),
            "page-down" => b.page(1),
            "home" => b.home(),
            "end" => b.end(),
            "wheel-up" | "wheel-down" => {
                b.scroll_by(if name == "wheel-up" {
                    -WHEEL_ROWS
                } else {
                    WHEEL_ROWS
                });
                self.hover_select();
            }
            "back" | "list" => return self.close_panel().await,
            "click" | "dblclick" => return self.click().await,
            "ok" => {
                if self.tuner.has_pending_digits() {
                    return self.run(Action::Confirm).await;
                }
                if let Some(ch) = b.selected_channel() {
                    let fx = self.tuner.tune_to(ch);
                    return self.apply(fx).await;
                }
            }
            "fav" => {
                if let Some(ch) = b.selected_channel() {
                    let fx = self.tuner.toggle_favorite_at(ch);
                    return self.apply(fx).await;
                }
            }
            _ => match Action::from_name(name, arg) {
                Some(a) => return self.run(a).await,
                None => eprintln!("unknown action {name:?}"),
            },
        }
        self.redraw().await
    }

    /// Opens the country / language chooser in the channel list.
    async fn open_picker(&mut self) -> Result<()> {
        if self.panel.is_none() {
            self.open_panel().await?;
        }
        self.typing = false;
        self.picker = Some(Picker::new(&self.tuner));
        self.hover_select();
        self.redraw().await
    }

    /// Back from the chooser to the channel list (which now shows the chosen channels).
    async fn close_picker(&mut self) -> Result<()> {
        self.picker = None;
        if let Some(panel) = self.panel.as_mut() {
            panel.refresh(&self.tuner);
        }
        self.redraw().await
    }

    /// Ticks / unticks the selected country or language; applies at once.
    async fn toggle_region(&mut self) -> Result<()> {
        let Some(picker) = self.picker.as_ref() else {
            return Ok(());
        };
        if picker.clear_selected() {
            return self.clear_regions().await;
        }
        let Some(name) = picker.selected_name().map(str::to_string) else {
            return Ok(());
        };
        let (mut countries, mut languages) = self.tuner.regions();
        let list = if picker.tab == PickerTab::Countries {
            &mut countries
        } else {
            &mut languages
        };
        match list.iter().position(|x| *x == name) {
            Some(i) => {
                list.remove(i);
            }
            None => list.push(name),
        }
        let fx = self.tuner.set_regions(countries, languages);
        self.apply(fx).await?;
        self.refresh_picker().await
    }

    /// Clears every country and language: all channels again.
    async fn clear_regions(&mut self) -> Result<()> {
        let fx = self.tuner.set_regions(vec![], vec![]);
        self.apply(fx).await?;
        self.refresh_picker().await
    }

    async fn refresh_picker(&mut self) -> Result<()> {
        if let Some(picker) = self.picker.as_mut() {
            picker.update(&self.tuner);
        }
        self.redraw().await
    }

    /// Keys while choosing countries / languages.
    async fn picker_key(&mut self, name: &str, arg: Option<&str>) -> Result<()> {
        let mouse = self.mouse_canvas();
        let Some(picker) = self.picker.as_mut() else {
            return Ok(());
        };
        let b = &mut picker.browser;
        match (name, arg) {
            ("up", _) => b.move_by(-1),
            ("down", _) => b.move_by(1),
            ("page-up", _) => b.page(-1),
            ("page-down", _) => b.page(1),
            ("home", _) => b.home(),
            ("end", _) => b.end(),
            ("wheel-up" | "wheel-down", _) => {
                b.scroll_by(if name == "wheel-up" {
                    -WHEEL_ROWS
                } else {
                    WHEEL_ROWS
                });
                self.hover_select();
            }
            ("ok", _) => return self.toggle_region().await,
            ("click" | "dblclick", _) => {
                let Some((x, y)) = mouse else { return Ok(()) };
                match picker.hit(x, y) {
                    Hit::Row(row) => {
                        if picker.browser.select_row(row).is_some() {
                            return self.toggle_region().await;
                        }
                    }
                    Hit::Scrollbar(f) => picker.browser.scroll_to_fraction(f),
                    Hit::Panel => {}
                    Hit::Outside => return self.close_picker().await,
                }
            }
            ("key", Some("space")) => {
                picker.query.push(' ');
                picker.refilter();
            }
            ("key", Some(c)) | ("digit", Some(c)) => {
                picker.query.push_str(c);
                picker.refilter();
            }
            ("prev", _) => {
                picker.query.pop();
                picker.refilter();
            }
            ("list", _) => picker.switch_tab(&self.tuner),
            ("clear", _) => return self.clear_regions().await,
            ("back", _) => {
                if picker.query.is_empty() {
                    return self.close_picker().await;
                }
                picker.query.clear();
                picker.refilter();
            }
            _ => return Ok(()),
        }
        self.redraw().await
    }

    /// Opens the channel list (if needed) and starts typing a search,
    /// continuing the current one if any.
    async fn start_search(&mut self) -> Result<()> {
        if self.panel.is_none() {
            self.open_panel().await?;
        }
        self.query = self.tuner.search_text().unwrap_or_default().to_string();
        self.typing = true;
        self.redraw().await
    }

    /// Keys while typing a search.
    async fn search_key(&mut self, name: &str, arg: Option<&str>) -> Result<()> {
        match (name, arg) {
            ("key", Some("space")) => self.query.push(' '),
            ("key", Some(c)) | ("digit", Some(c)) => self.query.push_str(c),
            ("prev", _) => {
                self.query.pop();
            }
            ("ok", _) => {
                self.typing = false; // keep the results; arrows / Enter pick a channel
                return self.redraw().await;
            }
            ("back", _) => {
                self.typing = false;
                self.query.clear();
            }
            ("list", _) => {
                self.typing = false;
                return self.close_panel().await;
            }
            // Moving in the list finishes typing.
            (
                "up" | "down" | "page-up" | "page-down" | "home" | "end" | "wheel-up"
                | "wheel-down" | "click",
                _,
            ) => {
                self.typing = false;
                return self.panel_key(name, arg).await;
            }
            _ => return Ok(()),
        }
        self.tuner.set_search(&self.query);
        let view = self.tuner.view();
        if let Some(panel) = self.panel.as_mut() {
            panel.browser.set_items(view, 0); // select the first match
        }
        self.redraw().await
    }

    async fn click(&mut self) -> Result<()> {
        let Some((x, y)) = self.mouse_canvas() else {
            return Ok(());
        };
        let Some(panel) = self.panel.as_mut() else {
            return Ok(());
        };
        match panel.hit(x, y) {
            Hit::Row(row) => {
                if let Some(ch) = panel.browser.select_row(row) {
                    let fx = self.tuner.tune_to(ch);
                    return self.apply(fx).await;
                }
            }
            Hit::Scrollbar(f) => {
                panel.browser.scroll_to_fraction(f);
                self.hover_select();
            }
            Hit::Panel => {}
            Hit::Outside => return self.close_panel().await,
        }
        self.redraw().await
    }

    /// Selects the row under the mouse. Returns true if the selection changed.
    fn hover_select(&mut self) -> bool {
        let Some((x, y)) = self.mouse_canvas() else {
            return false;
        };
        if let Some(picker) = self.picker.as_mut() {
            return match picker.hit(x, y) {
                Hit::Row(row) => {
                    let before = picker.browser.selected_channel();
                    picker.browser.select_row(row);
                    picker.browser.selected_channel() != before
                }
                _ => false,
            };
        }
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        match panel.hit(x, y) {
            Hit::Row(row) => {
                let before = panel.browser.selected_channel();
                panel.browser.select_row(row);
                panel.browser.selected_channel() != before
            }
            _ => false,
        }
    }

    fn mouse_canvas(&self) -> Option<(f64, f64)> {
        let (x, y) = self.mouse?;
        let scale = panel::H / self.osd_h;
        Some((x * scale, y * scale))
    }

    async fn open_panel(&mut self) -> Result<()> {
        self.panel = Some(Panel::new(&self.tuner));
        // Keep channel banners from covering the panel, keep the cursor visible.
        self.player
            .set_property("osd-align-x", json!("right"))
            .await?;
        self.player
            .set_property("cursor-autohide", json!("no"))
            .await?;
        self.player.show_text("", 1).await?;
        self.hover_select();
        self.redraw().await
    }

    async fn close_panel(&mut self) -> Result<()> {
        self.panel = None;
        self.typing = false;
        self.picker = None;
        self.player.remove_overlay(PANEL_OVERLAY_ID).await?;
        self.player
            .set_property("osd-align-x", json!("left"))
            .await?;
        self.player
            .set_property("cursor-autohide", json!(1000))
            .await
    }

    async fn redraw(&mut self) -> Result<()> {
        if let Some(picker) = &self.picker {
            let ass = picker.render(&self.tuner);
            return self.player.overlay(PANEL_OVERLAY_ID, &ass, panel::H).await;
        }
        if let Some(panel) = &self.panel {
            let typing = self.typing.then_some(self.query.as_str());
            let ass = panel.render(&self.tuner, self.check_status().as_deref(), typing);
            self.player
                .overlay(PANEL_OVERLAY_ID, &ass, panel::H)
                .await?;
        }
        Ok(())
    }

    async fn run(&mut self, action: Action) -> Result<()> {
        let fx = self.tuner.handle(action);
        self.apply(fx).await
    }

    async fn apply(&mut self, effects: Vec<Effect>) -> Result<()> {
        let before = self.tuner.current();
        for effect in effects {
            match effect {
                Effect::Tune(idx) => {
                    self.tune_seq += 1;
                    self.current_loaded = false;
                    self.tracks.clear();
                    if self.hint_until.take().is_some() {
                        self.player.remove_overlay(HINT_OVERLAY_ID).await?;
                    }
                    let (audio, subs) = self.tuner.track_prefs();
                    let codes =
                        |l: Option<String>| l.map(|l| language_codes(&l)).unwrap_or_default();
                    self.player
                        .set_track_prefs(&codes(audio), &codes(subs))
                        .await?;
                    let ch = &self.tuner.channels()[idx];
                    self.player
                        .play(
                            &ch.url,
                            &format!("{}  {}", idx + 1, ch.name),
                            ch.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT),
                            ch.referrer.as_deref().unwrap_or(""),
                        )
                        .await?;
                }
                Effect::Osd { text, ms } => self.player.show_text(&text, ms).await?,
                Effect::StartDigitTimer => {
                    self.digit_deadline =
                        Some(Instant::now() + Duration::from_millis(DIGIT_TIMEOUT_MS));
                }
                Effect::Save => storage::save_state(&self.tuner.state()),
            }
        }
        // Keep the panel in sync with group/favorite changes and zapping.
        if let Some(panel) = self.panel.as_mut() {
            let view = self.tuner.view();
            if view != panel.browser.items() {
                panel.refresh(&self.tuner);
            } else if self.tuner.current() != before {
                panel.browser.focus(self.tuner.current(), false);
            }
            self.redraw().await?;
        }
        self.sync_remote();
        Ok(())
    }
}
