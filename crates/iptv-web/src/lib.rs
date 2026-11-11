//! WebAssembly build of the IPTV core for web front-ends (the Samsung Tizen
//! TV app). Same TV logic as the desktop and Android apps: the [`Tuner`]
//! state machine with channel numbers, zapping, groups, favorites and
//! hidden dead channels.
//!
//! Networking (playlist download, channel checks) and playback are done by
//! the JavaScript side with the TV's own APIs; this module only decides
//! what to do. Effects are returned as JSON, like the Android bridge:
//! `{"type":"tune","seq","idx","number","name","url","userAgent","referrer"}`,
//! `{"type":"osd","text","ms"}`, `{"type":"digitTimer","ms"}`, `{"type":"save"}`.

use iptv_core::{
    parse_m3u, tuner::DIGIT_TIMEOUT_MS, Action, Effect, SavedState, Tuner, DEFAULT_USER_AGENT,
};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WebCore {
    tuner: Tuner,
    /// Number of tunes so far; identifies the stream the player reports on.
    tune_seq: u32,
}

#[wasm_bindgen]
impl WebCore {
    /// Parses the playlist text and restores the saved state (JSON, may be
    /// empty). `by_country` / `by_language` are iptv-org's playlists grouped
    /// by country / language (empty if unavailable), for that filter.
    #[wasm_bindgen(constructor)]
    pub fn new(
        playlist: &str,
        state_json: &str,
        by_country: &str,
        by_language: &str,
    ) -> Result<WebCore, JsError> {
        let mut channels = parse_m3u(playlist);
        iptv_core::m3u::tag_regions(&mut channels, by_country, by_language);
        let state: SavedState = serde_json::from_str(state_json).unwrap_or_default();
        let tuner = Tuner::new(channels, &state)
            .ok_or_else(|| JsError::new("The playlist contains no channels."))?;
        Ok(WebCore { tuner, tune_seq: 0 })
    }

    #[wasm_bindgen(js_name = channelCount)]
    pub fn channel_count(&self) -> u32 {
        self.tuner.channels().len() as u32
    }

    /// Stream URL of a channel (used by the channel checker).
    pub fn url(&self, idx: u32) -> String {
        self.tuner
            .channels()
            .get(idx as usize)
            .map(|c| c.url.clone())
            .unwrap_or_default()
    }

    /// Every channel name by index (fetched once for the channel list).
    pub fn names(&self) -> Vec<String> {
        self.tuner
            .channels()
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    /// Every channel's logo address by index ("" when it has none).
    pub fn logos(&self) -> Vec<String> {
        self.tuner
            .channels()
            .iter()
            .map(|c| {
                c.logo
                    .clone()
                    .filter(|l| l.starts_with("http"))
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Every stream URL by index (fetched once for the channel checker).
    pub fn urls(&self) -> Vec<String> {
        self.tuner
            .channels()
            .iter()
            .map(|c| c.url.clone())
            .collect()
    }

    /// Hides (or shows again) a channel found not to work. Returns true if
    /// that changed anything. Does not retune.
    #[wasm_bindgen(js_name = setDead)]
    pub fn set_dead(&mut self, idx: u32, dead: bool) -> bool {
        let changed = self.tuner.is_dead(idx as usize) != dead;
        self.tuner.set_dead(idx as usize, dead);
        changed
    }

    pub fn current(&self) -> u32 {
        self.tuner.current() as u32
    }

    pub fn start(&mut self) -> String {
        let fx = self.tuner.start();
        self.effects(fx)
    }

    /// Remote-control action ("up", "down", "digit" + "0".."9", "ok",
    /// "digit-timeout", "prev", "info", "group-next", "group-prev", "fav",
    /// "fav-only", "help").
    pub fn action(&mut self, name: &str, arg: Option<String>) -> String {
        let action = match name {
            "digit-timeout" => Some(Action::DigitTimeout),
            _ => Action::from_name(name, arg.as_deref()),
        };
        match action {
            Some(a) => {
                let fx = self.tuner.handle(a);
                self.effects(fx)
            }
            None => "[]".into(),
        }
    }

    #[wasm_bindgen(js_name = tuneTo)]
    pub fn tune_to(&mut self, idx: u32) -> String {
        let fx = self.tuner.tune_to(idx as usize);
        self.effects(fx)
    }

    #[wasm_bindgen(js_name = toggleFavorite)]
    pub fn toggle_favorite(&mut self, idx: u32) -> String {
        let fx = self.tuner.toggle_favorite_at(idx as usize);
        self.effects(fx)
    }

    /// The player failed on tune `seq`: hide the channel and move on.
    /// Late errors for streams the user already left are ignored.
    #[wasm_bindgen(js_name = playbackFailed)]
    pub fn playback_failed(&mut self, seq: u32) -> String {
        if seq != self.tune_seq {
            return "[]".into();
        }
        let fx = self.tuner.current_failed();
        self.effects(fx)
    }

    /// Tune `seq` is playing. Returns the channel index if it is the current
    /// stream (so the UI can remember it works), or -1.
    #[wasm_bindgen(js_name = playbackStarted)]
    pub fn playback_started(&mut self, seq: u32) -> i32 {
        if seq != self.tune_seq {
            return -1;
        }
        let cur = self.tuner.current();
        self.tuner.set_dead(cur, false);
        cur as i32
    }

    /// Shows only channels whose name matches `text` (ignoring case and
    /// accents; every word must match). Empty text: all channels again.
    #[wasm_bindgen(js_name = setSearch)]
    pub fn set_search(&mut self, text: &str) {
        self.tuner.set_search(text);
    }

    /// `{"countries":[[name,count]...],"languages":[...],"chosenCountries":[...],"chosenLanguages":[...]}`
    #[wasm_bindgen(js_name = regionsInfo)]
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

    /// Shows only channels from these countries or in these languages
    /// (JSON arrays of names; both empty: all channels). Returns effects.
    #[wasm_bindgen(js_name = setRegions)]
    pub fn set_regions(&mut self, countries_json: &str, languages_json: &str) -> String {
        let countries: Vec<String> = serde_json::from_str(countries_json).unwrap_or_default();
        let languages: Vec<String> = serde_json::from_str(languages_json).unwrap_or_default();
        let fx = self.tuner.set_regions(countries, languages);
        self.effects(fx)
    }

    /// Preferred track languages: `{"audio": code|null, "subtitles": code|null}`.
    #[wasm_bindgen(js_name = trackPrefs)]
    pub fn track_prefs(&self) -> String {
        let (audio, subtitles) = self.tuner.track_prefs();
        json!({ "audio": audio, "subtitles": subtitles }).to_string()
    }

    /// Remembers the audio / subtitle language picked (null: stream default
    /// / subtitles off). Returns effects (save).
    #[wasm_bindgen(js_name = setTrackPrefs)]
    pub fn set_track_prefs(&mut self, audio: Option<String>, subtitles: Option<String>) -> String {
        let fx = self.tuner.set_track_prefs(audio, subtitles);
        self.effects(fx)
    }

    /// "spa" -> "Spanish".
    #[wasm_bindgen(js_name = languageName)]
    pub fn language_name(&self, code: &str) -> String {
        iptv_core::lang::language_name(code)
    }

    /// Whether two track language codes mean the same language ("es" / "spa").
    #[wasm_bindgen(js_name = sameLanguage)]
    pub fn same_language(&self, a: &str, b: &str) -> bool {
        iptv_core::lang::same_language(a, b)
    }

    /// "Italy · English" or empty.
    #[wasm_bindgen(js_name = regionLabel)]
    pub fn region_label(&self) -> String {
        self.tuner.region_label().unwrap_or_default()
    }

    /// The search in effect, or empty.
    #[wasm_bindgen(js_name = searchText)]
    pub fn search_text(&self) -> String {
        self.tuner.search_text().unwrap_or_default().to_string()
    }

    /// Channel list header: `{"title","current"}`.
    #[wasm_bindgen(js_name = listInfo)]
    pub fn list_info(&self) -> String {
        json!({ "title": self.tuner.view_title(), "current": self.tuner.current() }).to_string()
    }

    /// Channel indices in the current view, ascending.
    pub fn view(&self) -> Vec<u32> {
        self.tuner.view().into_iter().map(|i| i as u32).collect()
    }

    /// Indices of favorite channels.
    pub fn favorites(&self) -> Vec<u32> {
        (0..self.tuner.channels().len())
            .filter(|&i| self.tuner.is_favorite(i))
            .map(|i| i as u32)
            .collect()
    }

    /// Last channel and favorites, as JSON to persist.
    pub fn state(&self) -> String {
        serde_json::to_string(&self.tuner.state()).unwrap_or_default()
    }

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
                Effect::Save => out.push(json!({ "type": "save" })),
            }
        }
        Value::Array(out).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYLIST: &str = "#EXTM3U\n#EXTINF:-1 group-title=\"News\",A\nhttp://a\n#EXTINF:-1 http-user-agent=\"UA\" group-title=\"News\",B\nhttp://b\n#EXTINF:-1,C\nhttp://c\n";

    fn fx(s: &str) -> Vec<Value> {
        serde_json::from_str::<Value>(s)
            .unwrap()
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn tunes_and_skips_failures() {
        let mut c = WebCore::new(PLAYLIST, "", "", "").unwrap();
        assert_eq!(c.channel_count(), 3);
        let start = fx(&c.start());
        assert_eq!(start[0]["seq"], 1);
        assert_eq!(start[0]["url"], "http://a");

        let up = fx(&c.action("up", None));
        assert_eq!(up[0]["userAgent"], "UA");
        assert!(up.iter().any(|e| e["type"] == "save"));

        assert_eq!(c.playback_failed(1), "[]"); // stale report
        let next = fx(&c.playback_failed(2));
        assert_eq!(next[0]["idx"], 2);
        assert_eq!(c.view(), vec![0, 2]);
        assert_eq!(c.playback_started(3), 2);
    }

    #[test]
    fn state_round_trips() {
        let mut c = WebCore::new(PLAYLIST, "", "", "").unwrap();
        c.start();
        c.action("up", None);
        c.toggle_favorite(1);
        let state = c.state();
        let c2 = WebCore::new(PLAYLIST, &state, "", "").unwrap();
        assert_eq!(c2.current(), 1);
        assert_eq!(c2.favorites(), vec![1]);
    }
}
