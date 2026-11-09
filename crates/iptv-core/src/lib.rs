//! Platform-independent IPTV logic.
//!
//! This crate deliberately has no I/O and no player dependency so it can be
//! reused by every front-end (desktop via mpv today, Android later).
//!
//! * [`m3u`] parses extended M3U playlists into [`Channel`]s.
//! * [`tuner`] is a pure state machine that turns remote-control style
//!   [`Action`]s into [`Effect`]s the front-end must perform.

pub mod browser;
pub mod m3u;
pub mod tuner;

pub use browser::Browser;
pub use m3u::{parse as parse_m3u, Channel};
pub use tuner::{Action, Effect, Filter, SavedState, Tuner};
