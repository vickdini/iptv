//! Networking shared by the IPTV front-ends (desktop and Android): playlist
//! download with conditional requests + local copy, and background channel
//! health checks. Callers pass the directory to store data in.

pub mod health;
pub mod playlist;

/// Browser-like user agent for streams that don't specify one; some servers
/// reject unknown clients.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
