//! A TV-like channel tuner.
//!
//! Channels are numbered by their position in the playlist (1-based), just
//! like the channel numbers on a TV. The tuner is a pure state machine: the
//! front-end feeds it [`Action`]s (remote-control buttons) and executes the
//! returned [`Effect`]s (tune the player, show an on-screen message, persist
//! state, arm the number-entry timer).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::m3u::Channel;

/// How long typed digits wait before tuning, in milliseconds.
pub const DIGIT_TIMEOUT_MS: u64 = 2000;

const BANNER_MS: u32 = 3500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ChannelUp,
    ChannelDown,
    /// A number key 0-9.
    Digit(u8),
    /// OK / Enter: tune to the typed number immediately.
    Confirm,
    /// The number-entry timer fired.
    DigitTimeout,
    /// Jump back to the previously watched channel.
    Previous,
    Info,
    NextGroup,
    PrevGroup,
    ToggleFavorite,
    ToggleFavoritesOnly,
    Help,
}

impl Action {
    /// Parses the action names used by front-ends' key bindings.
    pub fn from_name(name: &str, arg: Option<&str>) -> Option<Self> {
        Some(match name {
            "up" => Self::ChannelUp,
            "down" => Self::ChannelDown,
            "digit" => Self::Digit(arg?.parse().ok().filter(|d| *d <= 9)?),
            "ok" => Self::Confirm,
            "prev" => Self::Previous,
            "info" => Self::Info,
            "group-next" => Self::NextGroup,
            "group-prev" => Self::PrevGroup,
            "fav" => Self::ToggleFavorite,
            "fav-only" => Self::ToggleFavoritesOnly,
            "help" => Self::Help,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Start playing the channel at this index.
    Tune(usize),
    /// Show a message on screen.
    Osd { text: String, ms: u32 },
    /// (Re)start the number-entry timer; send [`Action::DigitTimeout`] after
    /// [`DIGIT_TIMEOUT_MS`].
    StartDigitTimer,
    /// Persist [`Tuner::state`].
    Save,
}

/// Which channels Up/Down zap through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    /// Index into [`Tuner::groups`].
    Group(usize),
    Favorites,
}

/// What survives a restart. Channels are identified by URL so favorites
/// survive playlist reordering.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedState {
    pub last_url: Option<String>,
    pub last_index: usize,
    #[serde(default)]
    pub favorites: Vec<String>,
}

pub struct Tuner {
    channels: Vec<Channel>,
    groups: Vec<String>,
    /// Channels known not to work; hidden from zapping and lists.
    dead: Vec<bool>,
    /// Direction of the last zap, used to skip past failing channels.
    forward: bool,
    current: usize,
    previous: Option<usize>,
    digits: String,
    max_digits: usize,
    filter: Filter,
    favorites: HashSet<String>,
}

impl Tuner {
    /// Returns `None` for an empty playlist.
    pub fn new(channels: Vec<Channel>, state: &SavedState) -> Option<Self> {
        if channels.is_empty() {
            return None;
        }
        let mut groups: Vec<String> = channels
            .iter()
            .flat_map(|c| c.groups.iter().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        groups.sort_by_key(|g| g.to_lowercase());

        let current = state
            .last_url
            .as_ref()
            .and_then(|url| channels.iter().position(|c| &c.url == url))
            .unwrap_or(state.last_index.min(channels.len() - 1));

        Some(Self {
            max_digits: channels.len().to_string().len(),
            dead: vec![false; channels.len()],
            forward: true,
            channels,
            groups,
            current,
            previous: None,
            digits: String::new(),
            filter: Filter::All,
            favorites: state.favorites.iter().cloned().collect(),
        })
    }

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }
    pub fn groups(&self) -> &[String] {
        &self.groups
    }
    pub fn current(&self) -> usize {
        self.current
    }
    pub fn current_channel(&self) -> &Channel {
        &self.channels[self.current]
    }
    pub fn filter(&self) -> Filter {
        self.filter
    }
    pub fn is_favorite(&self, idx: usize) -> bool {
        self.favorites.contains(&self.channels[idx].url)
    }

    pub fn state(&self) -> SavedState {
        let mut favorites: Vec<String> = self.favorites.iter().cloned().collect();
        favorites.sort();
        SavedState {
            last_url: Some(self.current_channel().url.clone()),
            last_index: self.current,
            favorites,
        }
    }

    /// Effects to perform at startup: tune the restored channel (or the next
    /// working one if it is known to be dead).
    pub fn start(&mut self) -> Vec<Effect> {
        if self.dead[self.current] {
            if let Some(idx) = self.step(true) {
                self.current = idx;
            }
        }
        vec![Effect::Tune(self.current), self.banner()]
    }

    /// Marks a channel as (not) working. Dead channels are skipped by
    /// zapping and left out of [`Tuner::view`]. Does not retune.
    pub fn set_dead(&mut self, idx: usize, dead: bool) {
        if let Some(d) = self.dead.get_mut(idx) {
            *d = dead;
        }
    }

    /// Channels not known to be dead.
    pub fn working_count(&self) -> usize {
        self.dead.iter().filter(|d| !**d).count()
    }

    pub fn is_dead(&self, idx: usize) -> bool {
        self.dead.get(idx).copied().unwrap_or(false)
    }

    /// The current channel failed to play: hide it and move on to the next
    /// working channel in the direction the user was zapping.
    pub fn current_failed(&mut self) -> Vec<Effect> {
        let failed = self.current;
        self.dead[failed] = true;
        let previous = self.previous;
        match self.step(self.forward) {
            Some(idx) => {
                let fx = self.tune(idx);
                // "Previous channel" should not lead back to the dead one.
                self.previous = previous.filter(|&p| p != idx);
                fx
            }
            None => vec![self.osd("No working channels in this view")],
        }
    }

    /// Selects a group by (case-insensitive) name. Returns false if unknown.
    pub fn set_group(&mut self, name: &str) -> bool {
        match self.groups.iter().position(|g| g.eq_ignore_ascii_case(name)) {
            Some(g) => {
                self.filter = Filter::Group(g);
                if !self.matches(self.current) {
                    if let Some(idx) = self.step(true) {
                        self.current = idx;
                    }
                }
                true
            }
            None => false,
        }
    }

    pub fn handle(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::ChannelUp | Action::ChannelDown => {
                self.digits.clear();
                self.forward = action == Action::ChannelUp;
                match self.step(self.forward) {
                    Some(idx) => self.tune(idx),
                    None => vec![self.osd("No channels in this view")],
                }
            }
            Action::Digit(d) => {
                if self.digits.len() >= self.max_digits {
                    return vec![];
                }
                self.digits.push(char::from(b'0' + d));
                if self.digits.len() == self.max_digits {
                    return self.confirm_digits();
                }
                let pad = "-".repeat(self.max_digits - self.digits.len());
                vec![
                    Effect::Osd {
                        text: format!("{}{}", self.digits, pad),
                        ms: DIGIT_TIMEOUT_MS as u32 + 500,
                    },
                    Effect::StartDigitTimer,
                ]
            }
            Action::Confirm => {
                if self.digits.is_empty() {
                    vec![self.banner()]
                } else {
                    self.confirm_digits()
                }
            }
            Action::DigitTimeout => self.confirm_digits(),
            Action::Previous => match self.previous {
                Some(p) if p < self.channels.len() => self.tune(p),
                _ => vec![self.osd("No previous channel")],
            },
            Action::Info => vec![self.info()],
            Action::NextGroup | Action::PrevGroup => {
                self.cycle_group(action == Action::NextGroup)
            }
            Action::ToggleFavorite => self.toggle_favorite_at(self.current),
            Action::ToggleFavoritesOnly => {
                if self.filter == Filter::Favorites {
                    self.filter = Filter::All;
                    return vec![self.osd(&self.view_title())];
                }
                if self.favorites.is_empty() {
                    return vec![self.osd("No favorites yet — press 's' to add the current channel")];
                }
                self.filter = Filter::Favorites;
                self.enter_filter(self.view_title())
            }
            Action::Help => vec![Effect::Osd { text: HELP.to_string(), ms: 8000 }],
        }
    }

    /// Tunes directly to a channel index (e.g. picked from a list panel).
    pub fn tune_to(&mut self, idx: usize) -> Vec<Effect> {
        if idx >= self.channels.len() {
            return vec![];
        }
        self.digits.clear();
        self.tune(idx)
    }

    pub fn toggle_favorite_at(&mut self, idx: usize) -> Vec<Effect> {
        let Some(ch) = self.channels.get(idx) else { return vec![] };
        let url = ch.url.clone();
        let msg = if self.favorites.remove(&url) {
            format!("Removed from favorites: {}", ch.name)
        } else {
            let msg = format!("★ Added to favorites: {}", ch.name);
            self.favorites.insert(url);
            msg
        };
        vec![self.osd(&msg), Effect::Save]
    }

    pub fn has_pending_digits(&self) -> bool {
        !self.digits.is_empty()
    }

    /// Channel indices Up/Down currently zap through, ascending.
    pub fn view(&self) -> Vec<usize> {
        (0..self.channels.len()).filter(|&i| self.matches(i)).collect()
    }

    /// e.g. "All channels (7012)", "News (630)", "Favorites (3)", counting
    /// only channels not known to be dead.
    pub fn view_title(&self) -> String {
        let name = self.filter_name().unwrap_or_else(|| "All channels".into());
        let count = (0..self.channels.len()).filter(|&i| self.matches(i)).count();
        format!("{name} ({count})")
    }

    /// Human readable "number  name" label.
    pub fn label(&self, idx: usize) -> String {
        let star = if self.is_favorite(idx) { " ★" } else { "" };
        format!("{}  {}{}", idx + 1, self.channels[idx].name, star)
    }

    fn matches(&self, idx: usize) -> bool {
        if self.dead[idx] {
            return false;
        }
        match self.filter {
            Filter::All => true,
            Filter::Group(g) => self.channels[idx].groups.iter().any(|x| *x == self.groups[g]),
            Filter::Favorites => self.favorites.contains(&self.channels[idx].url),
        }
    }

    /// Next/previous channel in the current filter, wrapping around.
    fn step_from(&self, from: usize, forward: bool) -> Option<usize> {
        let n = self.channels.len();
        (1..=n)
            .map(|k| if forward { (from + k) % n } else { (from + n - k) % n })
            .find(|&idx| self.matches(idx))
    }

    fn step(&self, forward: bool) -> Option<usize> {
        self.step_from(self.current, forward)
    }

    fn tune(&mut self, idx: usize) -> Vec<Effect> {
        // Typed numbers / previous channel may point at a dead channel:
        // land on the next working one instead.
        let idx = if self.dead[idx] {
            match self.step_from(idx, self.forward) {
                Some(i) => i,
                None => return vec![self.osd("No working channels in this view")],
            }
        } else {
            idx
        };
        if idx != self.current {
            self.previous = Some(self.current);
            self.current = idx;
        }
        vec![Effect::Tune(idx), self.banner(), Effect::Save]
    }

    fn confirm_digits(&mut self) -> Vec<Effect> {
        let typed = std::mem::take(&mut self.digits);
        if typed.is_empty() {
            return vec![];
        }
        self.forward = true;
        match typed.parse::<usize>() {
            Ok(n) if (1..=self.channels.len()).contains(&n) => self.tune(n - 1),
            _ => vec![self.osd(&format!("No channel {typed}"))],
        }
    }

    fn filter_name(&self) -> Option<String> {
        match self.filter {
            Filter::All => None,
            Filter::Group(g) => Some(self.groups[g].clone()),
            Filter::Favorites => Some("Favorites".into()),
        }
    }

    fn banner(&self) -> Effect {
        let ch = self.current_channel();
        let mut text = self.label(self.current);
        if !ch.groups.is_empty() {
            text.push('\n');
            text.push_str(&ch.groups.join(" · "));
        }
        if let Some(f) = self.filter_name() {
            text.push_str(&format!("\n[{f}]"));
        }
        Effect::Osd { text, ms: BANNER_MS }
    }

    fn info(&self) -> Effect {
        let ch = self.current_channel();
        let mut text = format!(
            "{}\nChannel {} of {}",
            self.label(self.current),
            self.current + 1,
            self.channels.len()
        );
        if !ch.groups.is_empty() {
            text.push_str(&format!("\nGroups: {}", ch.groups.join(", ")));
        }
        if let Some(id) = &ch.tvg_id {
            text.push_str(&format!("\nID: {id}"));
        }
        text.push_str(&format!("\nView: {}", self.filter_name().unwrap_or("All channels".into())));
        text.push_str(&format!("\n{}", ch.url));
        Effect::Osd { text, ms: 6000 }
    }

    fn cycle_group(&mut self, forward: bool) -> Vec<Effect> {
        // Cycle order: All, group 0, group 1, ..., group n-1, All, ...
        let n = self.groups.len();
        if n == 0 {
            return vec![self.osd("This playlist has no groups")];
        }
        let pos = match self.filter {
            Filter::All | Filter::Favorites => 0,
            Filter::Group(g) => g + 1,
        };
        let next = if forward { (pos + 1) % (n + 1) } else { (pos + n) % (n + 1) };
        self.filter = if next == 0 { Filter::All } else { Filter::Group(next - 1) };
        self.enter_filter(self.view_title())
    }

    /// After changing the filter: stay on the current channel if it belongs
    /// to the view, otherwise tune the next one that does.
    fn enter_filter(&mut self, title: String) -> Vec<Effect> {
        if self.matches(self.current) {
            return vec![self.osd(&title)];
        }
        match self.step(true) {
            Some(idx) => {
                let mut fx = self.tune(idx);
                if let Some(Effect::Osd { text, .. }) = fx.get_mut(1) {
                    *text = format!("{title}\n\n{text}");
                }
                fx
            }
            None => vec![self.osd(&title)],
        }
    }

    fn osd(&self, text: &str) -> Effect {
        Effect::Osd { text: text.to_string(), ms: BANNER_MS }
    }
}

const HELP: &str = "\
Up / Down / PgUp / PgDn / wheel   change channel
0-9 then wait or Enter             go to channel number
Backspace                          previous channel
Left / Right                       volume      m  mute
i  info      l / Tab  channel panel (wheel, click, Esc)
g / G        next / previous group
s  toggle favorite    S  favorites only
f  fullscreen         q  quit";

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(name: &str, group: &str) -> Channel {
        Channel {
            name: name.into(),
            url: format!("http://{name}"),
            groups: vec![group.into()],
            ..Default::default()
        }
    }

    fn tuner(n: usize) -> Tuner {
        let chs = (0..n)
            .map(|i| ch(&format!("c{i}"), if i % 2 == 0 { "Even" } else { "Odd" }))
            .collect();
        Tuner::new(chs, &SavedState::default()).unwrap()
    }

    fn tuned(fx: &[Effect]) -> Option<usize> {
        fx.iter().find_map(|e| match e {
            Effect::Tune(i) => Some(*i),
            _ => None,
        })
    }

    #[test]
    fn empty_playlist_is_rejected() {
        assert!(Tuner::new(vec![], &SavedState::default()).is_none());
    }

    #[test]
    fn up_down_wrap() {
        let mut t = tuner(3);
        assert_eq!(tuned(&t.handle(Action::ChannelDown)), Some(2));
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(0));
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(1));
    }

    #[test]
    fn digit_entry_and_auto_confirm() {
        let mut t = tuner(150); // 3 digits
        let fx = t.handle(Action::Digit(4));
        assert!(fx.contains(&Effect::StartDigitTimer));
        assert_eq!(tuned(&fx), None);
        assert_eq!(tuned(&t.handle(Action::Digit(2))), None);
        assert_eq!(tuned(&t.handle(Action::DigitTimeout)), Some(41));

        // Max digits typed -> tunes without waiting.
        t.handle(Action::Digit(1));
        t.handle(Action::Digit(0));
        assert_eq!(tuned(&t.handle(Action::Digit(0))), Some(99));

        // Out of range.
        t.handle(Action::Digit(9));
        t.handle(Action::Digit(9));
        assert_eq!(tuned(&t.handle(Action::Digit(9))), None);
        assert_eq!(t.current(), 99);

        // Channel 0 does not exist.
        t.handle(Action::Digit(0));
        assert_eq!(tuned(&t.handle(Action::Confirm)), None);
    }

    #[test]
    fn previous_channel_toggles() {
        let mut t = tuner(50);
        t.handle(Action::Digit(3));
        t.handle(Action::Digit(0));
        assert_eq!(t.current(), 29);
        assert_eq!(tuned(&t.handle(Action::Previous)), Some(0));
        assert_eq!(tuned(&t.handle(Action::Previous)), Some(29));
    }

    #[test]
    fn groups_restrict_zapping() {
        let mut t = tuner(6);
        assert_eq!(t.groups(), ["Even", "Odd"]);
        // All -> Even: current (0) is even, stays.
        assert_eq!(tuned(&t.handle(Action::NextGroup)), None);
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(2));
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(4));
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(0));
        // Even -> Odd: must move to an odd channel.
        assert_eq!(tuned(&t.handle(Action::NextGroup)), Some(1));
        // Odd -> All -> back to Odd via PrevGroup.
        t.handle(Action::NextGroup);
        assert_eq!(t.filter(), Filter::All);
        t.handle(Action::PrevGroup);
        assert_eq!(t.filter(), Filter::Group(1));
    }

    #[test]
    fn favorites_persist_and_filter() {
        let mut t = tuner(10);
        assert!(!t.handle(Action::ToggleFavoritesOnly).iter().any(|e| matches!(e, Effect::Tune(_))));
        t.handle(Action::ChannelUp);
        t.handle(Action::ChannelUp); // 2
        t.handle(Action::ToggleFavorite);
        t.handle(Action::Digit(7));
        t.handle(Action::Confirm); // 6
        t.handle(Action::ToggleFavorite);
        t.handle(Action::ChannelUp); // 7, not a favorite
        assert_eq!(tuned(&t.handle(Action::ToggleFavoritesOnly)), Some(2));
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(6));
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(2));

        let state = t.state();
        assert_eq!(state.favorites, vec!["http://c2", "http://c6"]);
        let t2 = Tuner::new(t.channels().to_vec(), &state).unwrap();
        assert_eq!(t2.current(), 2);
        assert!(t2.is_favorite(6));
    }

    #[test]
    fn restore_by_url_survives_reordering() {
        let state = SavedState { last_url: Some("http://c3".into()), last_index: 0, favorites: vec![] };
        let mut chs: Vec<Channel> = (0..5).map(|i| ch(&format!("c{i}"), "G")).collect();
        chs.reverse();
        let t = Tuner::new(chs, &state).unwrap();
        assert_eq!(t.current_channel().name, "c3");
    }

    #[test]
    fn dead_channels_are_skipped_and_hidden() {
        let mut t = tuner(6);
        t.set_dead(1, true);
        t.set_dead(2, true);
        assert_eq!(t.view(), vec![0, 3, 4, 5]);
        assert_eq!(t.view_title(), "All channels (4)");
        assert_eq!(tuned(&t.handle(Action::ChannelUp)), Some(3));
        assert_eq!(tuned(&t.handle(Action::ChannelDown)), Some(0));
        // Typing a dead channel's number lands on the next working one.
        // (6 channels -> 1 digit, so typing tunes immediately.)
        assert_eq!(tuned(&t.handle(Action::Digit(2))), Some(3));
    }

    #[test]
    fn failure_skips_in_zap_direction() {
        let mut t = tuner(6);
        t.handle(Action::ChannelUp); // 1
        t.handle(Action::ChannelUp); // 2, prev = 1
        assert_eq!(tuned(&t.current_failed()), Some(3));
        assert!(t.is_dead(2));
        // Previous goes back to 1, not to the dead 2.
        assert_eq!(tuned(&t.handle(Action::Previous)), Some(1));

        t.handle(Action::ChannelDown); // 0
        assert_eq!(tuned(&t.handle(Action::ChannelDown)), Some(5));
        assert_eq!(tuned(&t.current_failed()), Some(4)); // keeps going down

        for i in 0..6 {
            t.set_dead(i, true);
        }
        assert_eq!(tuned(&t.current_failed()), None);
    }

    #[test]
    fn start_skips_dead_restored_channel() {
        let state = SavedState { last_url: Some("http://c2".into()), ..Default::default() };
        let chs = (0..5).map(|i| ch(&format!("c{i}"), "G")).collect();
        let mut t = Tuner::new(chs, &state).unwrap();
        t.set_dead(2, true);
        assert_eq!(tuned(&t.start()), Some(3));
    }

    #[test]
    fn action_names() {
        assert_eq!(Action::from_name("digit", Some("7")), Some(Action::Digit(7)));
        assert_eq!(Action::from_name("digit", Some("12")), None);
        assert_eq!(Action::from_name("nope", None), None);
    }
}
