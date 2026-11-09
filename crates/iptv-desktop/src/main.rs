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
use iptv_core::{tuner::DIGIT_TIMEOUT_MS, Action, Effect, Tuner};
use iptv_net::{health, playlist, DEFAULT_USER_AGENT};
use serde_json::json;
use tokio::time::Instant;

use crate::{
    health::{CheckResult, HealthCache, Probe},
    mpv::{Event, Mpv},
    panel::{Hit, Panel},
};

const DEFAULT_PLAYLIST: &str = "https://iptv-org.github.io/iptv/index.m3u";
const PANEL_OVERLAY_ID: u32 = 1;
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
        tokio::task::spawn_blocking(move || playlist::load(&source, &storage::cache_dir())).await??;
    let what = match freshness {
        playlist::Freshness::UpToDate => "already up to date",
        playlist::Freshness::Downloaded => "downloaded new version",
        playlist::Freshness::Offline => "offline, using saved copy",
        playlist::Freshness::Local => "local file",
    };
    eprintln!("Playlist: {what} ({} channels, {:.0?})", channels.len(), started.elapsed());
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
                let n = tuner.channels().iter().filter(|c| c.groups.contains(g)).count();
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
    eprintln!("Loaded {} channels. Using mpv at {}", tuner.channels().len(), mpv_path.display());
    eprintln!("Press 'h' in the player window for the remote-control keys, 'l' for the channel list.");

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
        let jobs = jobs.into_iter().map(|i| (i, tuner.channels()[i].clone())).collect();
        health::spawn_checker(jobs, DEFAULT_USER_AGENT, move |r| check_tx.send(r).is_ok());
    } else {
        drop(check_tx);
    }
    eprintln!(
        "Channel check: {} known dead, {} to check",
        (0..n).filter(|&i| tuner.is_dead(i)).count(),
        if args.no_check { 0 } else { check_total }
    );

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
    };
    let fx = app.tuner.start();
    app.apply(fx).await?;

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
            _ = tick.tick() => {
                if app.view_dirty || (app.panel.is_some() && checks_open) {
                    app.view_dirty = false;
                    app.refresh_panel().await?;
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
            format!("checking channels {}%", self.check_done * 100 / self.check_total.max(1))
        })
    }

    async fn on_key(&mut self, name: &str, arg: Option<&str>) -> Result<()> {
        if self.debug {
            eprintln!("key: {name} {}", arg.unwrap_or_default());
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
        let Some(panel) = self.panel.as_mut() else { return Ok(()) };
        let b = &mut panel.browser;
        match name {
            "up" => b.move_by(-1),
            "down" => b.move_by(1),
            "page-up" => b.page(-1),
            "page-down" => b.page(1),
            "home" => b.home(),
            "end" => b.end(),
            "wheel-up" | "wheel-down" => {
                b.scroll_by(if name == "wheel-up" { -WHEEL_ROWS } else { WHEEL_ROWS });
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

    async fn click(&mut self) -> Result<()> {
        let Some((x, y)) = self.mouse_canvas() else { return Ok(()) };
        let Some(panel) = self.panel.as_mut() else { return Ok(()) };
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
        let Some((x, y)) = self.mouse_canvas() else { return false };
        let Some(panel) = self.panel.as_mut() else { return false };
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
        self.player.set_property("osd-align-x", json!("right")).await?;
        self.player.set_property("cursor-autohide", json!("no")).await?;
        self.player.show_text("", 1).await?;
        self.hover_select();
        self.redraw().await
    }

    async fn close_panel(&mut self) -> Result<()> {
        self.panel = None;
        self.player.remove_overlay(PANEL_OVERLAY_ID).await?;
        self.player.set_property("osd-align-x", json!("left")).await?;
        self.player.set_property("cursor-autohide", json!(1000)).await
    }

    async fn redraw(&mut self) -> Result<()> {
        if let Some(panel) = &self.panel {
            let ass = panel.render(&self.tuner, self.check_status().as_deref());
            self.player.overlay(PANEL_OVERLAY_ID, &ass, panel::H).await?;
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
        Ok(())
    }
}
