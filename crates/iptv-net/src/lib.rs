//! Networking shared by the IPTV front-ends (desktop and Android): playlist
//! download with conditional requests + local copy, and background channel
//! health checks. Callers pass the directory to store data in.

pub mod health;
pub mod playlist;
pub mod update;

pub use iptv_core::DEFAULT_USER_AGENT;
