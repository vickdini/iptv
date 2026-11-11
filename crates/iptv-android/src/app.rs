//! Android app state behind the JNI bridge. Independent of JNI so it can be
//! unit-tested on the host.

use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Result};
use iptv_core::{tuner::DIGIT_TIMEOUT_MS, Action, Channel, Effect, SavedState, Tuner};
use iptv_net::{
    health::{self, CheckResult, HealthCache, Probe},
    playlist, DEFAULT_USER_AGENT,
};
use iptv_remote::{Command, Remote, Snapshot, TrackInfo};
use serde::Deserialize;
use serde_json::{json, Value};

struct Dirs {
    cache: PathBuf,
    config: PathBuf,
}

static DIRS: Mutex<Option<Dirs>> = Mutex::new(None);
static APP: Mutex<Option<App>> = Mutex::new(None);
/// The phone remote's server lives for the whole process (it survives the
/// playlist being reloaded); its commands are read by the UI's remote thread.
static REMOTE: Mutex<Option<Remote>> = Mutex::new(None);
static REMOTE_RX: Mutex<Option<std::sync::mpsc::Receiver<Command>>> = Mutex::new(None);

pub fn init(cache: PathBuf, config: PathBuf) {
    *DIRS.lock().unwrap() = Some(Dirs { cache, config });
}

/// Downloads/refreshes the playlist and (re)creates the app state.
pub fn load_playlist(url: &str) -> Result<String> {
    let (cache, config) = {
        let dirs = DIRS.lock().unwrap();
        let d = dirs
            .as_ref()
            .ok_or_else(|| anyhow!("Core.init was not called"))?;
        (d.cache.clone(), d.config.clone())
    };
    let (channels, freshness) = playlist::load(url, &cache)?;
    if channels.is_empty() {
        bail!("The playlist contains no channels.");
    }
    let n = channels.len();
    let app = App::new(channels, cache, config, true);
    if let Some(remote) = REMOTE.lock().unwrap().as_ref() {
        let fresh = Snapshot::from_tuner(&app.tuner);
        remote.update(|s| *s = fresh);
    }
    // Replacing the old app (if any) drops it, which stops its checks.
    *APP.lock().unwrap() = Some(app);
    Ok(json!({ "channels": n, "freshness": format!("{freshness:?}") }).to_string())
}

/// Whether a playlist is loaded (the app survives the Android activity
/// being recreated, e.g. on configuration changes).
pub fn is_loaded() -> bool {
    APP.lock().map(|a| a.is_some()).unwrap_or(false)
}

/// Runs `f` with the loaded app.
pub fn with<T>(f: impl FnOnce(&mut App) -> Result<T>) -> Result<T> {
    let mut guard = APP.lock().unwrap_or_else(|e| e.into_inner());
    let app = guard
        .as_mut()
        .ok_or_else(|| anyhow!("playlist not loaded yet"))?;
    f(app)
}

pub struct App {
    tuner: Tuner,
    health: HealthCache,
    config_dir: PathBuf,
    /// Results delivered by checker threads, merged by `poll`.
    results: Arc<Mutex<Vec<CheckResult>>>,
    /// Cleared on drop so checker threads stop.
    alive: Arc<AtomicBool>,
    check_total: usize,
    check_done: usize,
    /// Number of tunes so far; identifies the stream the player reports on.
    tune_seq: u64,
    /// What the player reports for the phone remote (tracks, volume).
    player: PlayerInfo,
    /// Check results are saved periodically: Android may kill the app
    /// without warning, losing anything not on disk.
    last_save: Instant,
}

impl App {
    pub fn new(
        channels: Vec<Channel>,
        cache_dir: PathBuf,
        config_dir: PathBuf,
        run_checks: bool,
    ) -> Self {
        let state: SavedState = fs::read_to_string(config_dir.join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut tuner = Tuner::new(channels, &state).expect("non-empty playlist");

        // Hide channels known to be dead; queue the rest, nearest first.
        let health = HealthCache::load(&cache_dir);
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

        let results = Arc::new(Mutex::new(Vec::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let check_total = if run_checks { jobs.len() } else { 0 };
        if run_checks {
            let jobs = jobs
                .into_iter()
                .map(|i| (i, tuner.channels()[i].clone()))
                .collect();
            let (results, alive) = (results.clone(), alive.clone());
            health::spawn_checker(jobs, DEFAULT_USER_AGENT, move |r| {
                if !alive.load(Ordering::Relaxed) {
                    return false;
                }
                results.lock().unwrap().push(r);
                true
            });
        }

        Self {
            tuner,
            health,
            config_dir,
            results,
            alive,
            check_total,
            check_done: 0,
            tune_seq: 0,
            player: PlayerInfo::default(),
            last_save: Instant::now(),
        }
    }

    pub fn start(&mut self) -> String {
        let fx = self.tuner.start();
        self.effects(fx)
    }

    pub fn action(&mut self, name: &str, arg: Option<&str>) -> String {
        let action = match name {
            "digit-timeout" => Some(Action::DigitTimeout),
            _ => Action::from_name(name, arg),
        };
        match action {
            Some(a) => {
                let fx = self.tuner.handle(a);
                self.effects(fx)
            }
            None => "[]".into(),
        }
    }

    pub fn tune_to(&mut self, idx: usize) -> String {
        let fx = self.tuner.tune_to(idx);
        self.effects(fx)
    }

    pub fn toggle_favorite(&mut self, idx: usize) -> String {
        let fx = self.tuner.toggle_favorite_at(idx);
        self.effects(fx)
    }

    /// The player failed on tune `seq`. Late errors for streams the user has
    /// already left are ignored.
    pub fn playback_failed(&mut self, seq: u64) -> String {
        if seq != self.tune_seq {
            return "[]".into();
        }
        let url = self.tuner.current_channel().url.clone();
        self.health.record(&url, false);
        let fx = self.tuner.current_failed();
        self.effects(fx)
    }

    pub fn playback_started(&mut self, seq: u64) {
        if seq != self.tune_seq {
            return;
        }
        let cur = self.tuner.current();
        let url = self.tuner.current_channel().url.clone();
        self.health.record(&url, true);
        self.tuner.set_dead(cur, false);
    }

    /// Merges finished background checks. `changed` tells the UI to refresh
    /// the channel list.
    pub fn poll(&mut self) -> String {
        let results = std::mem::take(&mut *self.results.lock().unwrap());
        let mut changed = false;
        for r in results {
            self.check_done += 1;
            let url = self.tuner.channels()[r.idx].url.clone();
            match r.probe {
                Probe::Ok => {
                    self.health.record(&url, true);
                    changed |= self.tuner.is_dead(r.idx);
                    self.tuner.set_dead(r.idx, false);
                }
                // The channel on screen is judged by the player, not the probe.
                Probe::Dead if r.idx == self.tuner.current() => {}
                Probe::Dead => {
                    self.health.record(&url, false);
                    changed |= !self.tuner.is_dead(r.idx);
                    self.tuner.set_dead(r.idx, true);
                }
                Probe::Unknown => {}
            }
        }
        if self.last_save.elapsed() > Duration::from_secs(20) {
            self.health.save_if_dirty();
            self.last_save = Instant::now();
        }
        if changed {
            self.sync_remote();
        }
        json!({ "changed": changed, "status": self.status() }).to_string()
    }

    // --- second picture (split screen) ------------------------------------------

    /// The next / previous channel after `from` in the current view, or -1.
    pub fn neighbor(&self, from: usize, forward: bool) -> i64 {
        self.tuner.neighbor(from, forward).map_or(-1, |i| i as i64)
    }

    /// The channel watched before the current one, or -1.
    pub fn previous_channel(&self) -> i64 {
        self.tuner.previous().map_or(-1, |i| i as i64)
    }

    /// How to play a channel without tuning to it (for the second picture):
    /// `{"idx","number","name","url","userAgent","referrer"}`.
    pub fn stream_info(&self, idx: usize) -> String {
        let Some(ch) = self.tuner.channels().get(idx) else {
            return "{}".into();
        };
        json!({
            "idx": idx,
            "number": idx + 1,
            "name": ch.name,
            "url": ch.url,
            "userAgent": ch.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT),
            "referrer": ch.referrer,
        })
        .to_string()
    }

    // --- phone remote -----------------------------------------------------------

    /// Whether the phone remote was turned on before (it then starts with the app).
    pub fn remote_enabled(&self) -> bool {
        self.tuner.remote_pin().is_some()
    }

    /// Starts the phone remote (once) and returns how to connect:
    /// `{"address","pin","qrSize","qr":"0101..."}` (address null without a network).
    pub fn remote_start(&mut self) -> Result<String> {
        let mut slot = REMOTE.lock().unwrap();
        if slot.is_none() {
            let pin = match self.tuner.remote_pin() {
                Some(p) => p.to_string(),
                None => {
                    let pin = iptv_remote::new_pin();
                    self.tuner.set_remote_pin(Some(pin.clone()));
                    self.save_state();
                    pin
                }
            };
            let (server, rx) = Remote::start(pin, Snapshot::from_tuner(&self.tuner))?;
            *REMOTE_RX.lock().unwrap() = Some(rx);
            *slot = Some(server);
        }
        let server = slot.as_ref().expect("started");
        let qr = server.pairing_url().as_deref().and_then(iptv_remote::qr);
        drop(slot);
        self.sync_remote();
        let slot = REMOTE.lock().unwrap();
        let server = slot.as_ref().expect("started");
        Ok(json!({
            "address": server.address(),
            "pin": server.pin(),
            "qrSize": qr.as_ref().map(|q| q.0),
            "qr": qr.map(|(_, dark)| dark.iter().map(|d| if *d { '1' } else { '0' }).collect::<String>()),
        })
        .to_string())
    }

    /// The player's tracks and volume: `{"audio":[{id,label,selected}],"subs":[...],"volume":n,"muted":b}`.
    pub fn remote_player(&mut self, info: &str) {
        if let Ok(p) = serde_json::from_str(info) {
            self.player = p;
            self.sync_remote();
        }
    }

    /// Tells phones what is on now.
    fn sync_remote(&self) {
        let slot = REMOTE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(server) = slot.as_ref() else { return };
        server.update(|s| {
            s.refresh(&self.tuner);
            s.audio = self.player.audio.clone();
            s.subs = self.player.subs.clone();
            s.volume = self.player.volume;
            s.muted = self.player.muted;
        });
    }

    /// Shows only channels matching `text` (empty: all channels).
    pub fn set_search(&mut self, text: &str) {
        self.tuner.set_search(text);
    }

    /// `{"countries":[[name,count]...],"languages":[...],"chosenCountries":[...],"chosenLanguages":[...]}`
    pub fn regions_info(&self) -> String {
        let (c, l) = self.tuner.regions();
        json!({
            "countries": self.tuner.countries(),
            "languages": self.tuner.languages(),
            "chosenCountries": c,
            "chosenLanguages": l,
        })
        .to_string()
    }

    /// Shows only channels from these countries or in these languages.
    pub fn set_regions(&mut self, countries: Vec<String>, languages: Vec<String>) -> String {
        let fx = self.tuner.set_regions(countries, languages);
        self.effects(fx)
    }

    /// `{"audio": code|null, "subtitles": code|null}` (subtitles null = off).
    pub fn track_prefs(&self) -> String {
        let (audio, subtitles) = self.tuner.track_prefs();
        json!({ "audio": audio, "subtitles": subtitles }).to_string()
    }

    /// Remembers the audio / subtitle language the user picked.
    pub fn set_track_prefs(&mut self, audio: Option<String>, subtitles: Option<String>) {
        let fx = self.tuner.set_track_prefs(audio, subtitles);
        self.effects(fx);
    }

    pub fn search_text(&self) -> String {
        self.tuner.search_text().unwrap_or_default().to_string()
    }

    /// Small header for the channel list: `{"title","status","current"}`.
    pub fn list_info(&self) -> String {
        json!({
            "title": self.tuner.view_title(),
            "current": self.tuner.current(),
            "status": self.status(),
        })
        .to_string()
    }

    /// Channel indices in the current view (ascending), for the list rows.
    pub fn view_indices(&self) -> Vec<i32> {
        self.tuner.view().into_iter().map(|i| i as i32).collect()
    }

    /// Indices of favorite channels.
    pub fn favorite_indices(&self) -> Vec<i32> {
        (0..self.tuner.channels().len())
            .filter(|&i| self.tuner.is_favorite(i))
            .map(|i| i as i32)
            .collect()
    }

    /// Every channel name, by index (fetched once by the UI).
    pub fn names(&self) -> Vec<String> {
        self.tuner
            .channels()
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    pub fn save(&mut self) {
        self.save_state();
        self.health.save_if_dirty();
    }

    fn status(&self) -> Option<String> {
        (self.check_done < self.check_total).then(|| {
            format!(
                "Checking channels {}%",
                self.check_done * 100 / self.check_total.max(1)
            )
        })
    }

    fn save_state(&self) {
        let _ = fs::create_dir_all(&self.config_dir);
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.tuner.state()) {
            let tmp = self.config_dir.join("state.json.tmp");
            if fs::write(&tmp, bytes).is_ok() {
                let _ = fs::rename(&tmp, self.config_dir.join("state.json"));
            }
        }
    }

    /// Converts tuner effects into JSON instructions for the Android UI.
    fn effects(&mut self, fx: Vec<Effect>) -> String {
        let mut out = Vec::new();
        for e in fx {
            match e {
                Effect::Tune(idx) => {
                    self.tune_seq += 1;
                    let ch = &self.tuner.channels()[idx];
                    out.push(json!({
                        "type": "tune",
                        "seq": self.tune_seq,
                        "idx": idx,
                        "number": idx + 1,
                        "name": ch.name,
                        "url": ch.url,
                        "userAgent": ch.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT),
                        "referrer": ch.referrer,
                    }));
                }
                Effect::Osd { text, ms } => {
                    out.push(json!({ "type": "osd", "text": text, "ms": ms }))
                }
                Effect::StartDigitTimer => {
                    out.push(json!({ "type": "digitTimer", "ms": DIGIT_TIMEOUT_MS }))
                }
                Effect::Save => self.save_state(),
            }
        }
        self.sync_remote();
        Value::Array(out).to_string()
    }
}

#[derive(Debug, Default, Deserialize)]
struct PlayerInfo {
    #[serde(default)]
    audio: Vec<TrackInfo>,
    #[serde(default)]
    subs: Vec<TrackInfo>,
    #[serde(default)]
    volume: Option<u32>,
    #[serde(default)]
    muted: bool,
}

/// Waits up to `timeout` for a phone's request: its JSON, or "" if none.
/// Called in a loop by the app's remote thread (not under the app lock).
pub fn remote_next(timeout: Duration) -> String {
    let rx = REMOTE_RX.lock().unwrap_or_else(|e| e.into_inner());
    let Some(rx) = rx.as_ref() else {
        drop(rx);
        std::thread::sleep(timeout);
        return String::new();
    };
    match rx.recv_timeout(timeout) {
        Ok(cmd) => serde_json::to_string(&cmd).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
        self.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(dir: &str) -> App {
        let base =
            std::env::temp_dir().join(format!("iptv-android-test-{dir}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let chs = (0..5)
            .map(|i| Channel {
                name: format!("C{i}"),
                url: format!("http://c{i}"),
                user_agent: (i == 2).then(|| "UA2".into()),
                ..Default::default()
            })
            .collect();
        App::new(chs, base.join("cache"), base.join("config"), false)
    }

    fn parse(s: &str) -> Vec<Value> {
        serde_json::from_str::<Value>(s)
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn effects_carry_stream_details_and_sequence() {
        let mut a = app("fx");
        let fx = parse(&a.start());
        assert_eq!(fx[0]["type"], "tune");
        assert_eq!(fx[0]["seq"], 1);
        assert_eq!(fx[0]["userAgent"], DEFAULT_USER_AGENT);
        assert_eq!(fx[1]["type"], "osd");

        let fx = parse(&a.action("up", None));
        let fx = if fx[0]["idx"] == 1 {
            parse(&a.action("up", None))
        } else {
            fx
        };
        assert_eq!(fx[0]["idx"], 2);
        assert_eq!(fx[0]["userAgent"], "UA2");

        assert_eq!(parse(&a.action("digit", Some("4")))[0]["idx"], 3);
        assert_eq!(a.action("nonsense", None), "[]");
    }

    #[test]
    fn playback_failure_skips_and_ignores_stale_reports() {
        let mut a = app("fail");
        a.start(); // seq 1, channel 0
        a.action("up", None); // seq 2, channel 1
        assert_eq!(a.playback_failed(1), "[]"); // stale
        let fx = parse(&a.playback_failed(2));
        assert_eq!(fx[0]["idx"], 2);
        assert_eq!(a.view_indices(), vec![0, 2, 3, 4]);
        let info: Value = serde_json::from_str(&a.list_info()).unwrap();
        assert_eq!(info["title"], "All channels (4)");
        assert_eq!(info["current"], 2);
        assert_eq!(a.names()[3], "C3");
    }
}
