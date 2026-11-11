//! Platform-independent IPTV logic.
//!
//! This crate deliberately has no I/O and no player dependency so it can be
//! reused by every front-end (desktop via mpv today, Android later).
//!
//! * [`m3u`] parses extended M3U playlists into [`Channel`]s.
//! * [`tuner`] is a pure state machine that turns remote-control style
//!   [`Action`]s into [`Effect`]s the front-end must perform.

pub mod browser;
pub mod favorites;
pub mod i18n;
pub mod lang;
pub mod m3u;
pub mod tuner;

pub use browser::Browser;
pub use m3u::{parse as parse_m3u, Channel};
pub use tuner::{Action, Effect, Filter, SavedState, Tuner};

/// Browser-like user agent for streams that don't specify one; some servers
/// reject unknown clients.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
