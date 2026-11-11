//! Downloads channel logos (small images listed in the playlist).

use std::{io::Read, time::Duration};

use anyhow::{bail, Context, Result};

/// Logos larger than this are not worth showing in a list.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Fetches a logo image's bytes.
pub fn fetch(url: &str, user_agent: &str) -> Result<Vec<u8>> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("not a web address");
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(8))
        .timeout_read(Duration::from_secs(10))
        .user_agent(user_agent)
        .build();
    let resp = agent.get(url).call().context("logo download failed")?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        bail!("logo too large");
    }
    Ok(bytes)
}
