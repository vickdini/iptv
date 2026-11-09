//! Self-update support: finds out whether a newer release exists and
//! downloads it, verifying its SHA-256.
//!
//! Every GitHub release publishes `SHA256SUMS.txt`, listing each download
//! with its checksum, e.g. `81cf…95f3  IPTV-0.3.0.apk`. One small request to
//! the latest release's copy tells us the newest version of a given file
//! (from its name) and how to verify it, without the GitHub API (which is
//! rate limited) and without parsing JSON.

use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// Where releases are published.
pub const RELEASES: &str = "https://github.com/vickdini/iptv/releases";

/// A newer release available for download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub file_name: String,
    pub url: String,
    pub sha256: String,
}

/// Parses "1.2.3" (optionally with a leading "v") into comparable numbers.
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Finds the newest file named `{prefix}{version}{suffix}` in a
/// SHA256SUMS listing, if it is newer than `current`.
pub fn find_update(sums: &str, prefix: &str, suffix: &str, current: &str, base_url: &str) -> Option<Update> {
    let current = parse_version(current)?;
    sums.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let sha = it.next()?;
            let name = it.next()?.trim_start_matches('*');
            let version = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
            let parsed = parse_version(version)?;
            (sha.len() == 64 && sha.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| (parsed, version.to_string(), name.to_string(), sha.to_ascii_lowercase()))
        })
        .filter(|(parsed, ..)| *parsed > current)
        .max_by_key(|(parsed, ..)| *parsed)
        .map(|(_, version, file_name, sha256)| Update {
            url: format!("{base_url}/download/v{version}/{file_name}"),
            version,
            file_name,
            sha256,
        })
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("iptv/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Checks the latest release at `base_url` (normally [`RELEASES`]) for a
/// newer `{prefix}{version}{suffix}` file than `current`.
pub fn check(base_url: &str, prefix: &str, suffix: &str, current: &str) -> Result<Option<Update>> {
    let sums = agent()
        .get(&format!("{base_url}/latest/download/SHA256SUMS.txt"))
        .call()
        .context("could not reach the release server")?
        .into_string()?;
    Ok(find_update(&sums, prefix, suffix, current, base_url))
}

/// Downloads `update` to `dest`, verifying its SHA-256. Writes to a
/// temporary file first, so `dest` only ever holds a complete, verified file.
/// Dropped connections (flaky Wi-Fi) are retried a few times.
pub fn download(update: &Update, dest: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(5 * attempt));
        }
        match download_once(update, dest) {
            Ok(()) => return Ok(()),
            // A wrong checksum won't fix itself by retrying.
            Err(e) if e.to_string().contains("checksum mismatch") => return Err(e),
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("at least one attempt"))
}

fn download_once(update: &Update, dest: &Path) -> Result<()> {
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = dest.with_extension("part");
    let resp = agent().get(&update.url).call().context("download failed")?;
    let mut reader = resp.into_reader();
    let mut file = fs::File::create(&tmp)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.sync_all()?;
    drop(file);
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if actual != update.sha256 {
        let _ = fs::remove_file(&tmp);
        bail!("checksum mismatch for {} (expected {}, got {actual})", update.file_name, update.sha256);
    }
    let _ = fs::remove_file(dest);
    fs::rename(&tmp, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS: &str = "\
81cf38bec006b6fe5ca2f669d09b3f3bcfc13e572ace36333016e771be8095f3  IPTV-0.3.0.apk
145e44ff7654dbd2dbf79006e8436ef678ce306fa857adf424e4d881481e9b6d  IPTV-Samsung-TV-0.3.0.zip
b1c5a0be00569533c8ff06e273179818e34a1b475b4e40097434740f200132b3  IPTV-Setup-0.3.0.exe
";

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("x.y.z"), None);
    }

    #[test]
    fn finds_newer_file_for_each_platform() {
        let u = find_update(SUMS, "IPTV-Setup-", ".exe", "0.2.0", RELEASES).unwrap();
        assert_eq!(u.version, "0.3.0");
        assert_eq!(u.url, "https://github.com/vickdini/iptv/releases/download/v0.3.0/IPTV-Setup-0.3.0.exe");
        assert_eq!(u.sha256, "b1c5a0be00569533c8ff06e273179818e34a1b475b4e40097434740f200132b3");

        // "IPTV-" must not match "IPTV-Samsung-TV-..." or "IPTV-Setup-...".
        let apk = find_update(SUMS, "IPTV-", ".apk", "0.2.0", RELEASES).unwrap();
        assert_eq!(apk.file_name, "IPTV-0.3.0.apk");

        assert_eq!(find_update(SUMS, "IPTV-Setup-", ".exe", "0.3.0", RELEASES), None); // same
        assert_eq!(find_update(SUMS, "IPTV-Setup-", ".exe", "0.4.0", RELEASES), None); // older
    }

    #[test]
    fn download_verifies_checksum() {
        use std::{io::BufRead, net::TcpListener, thread};
        let body = b"installer bytes".to_vec();
        let sha: String = Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let served = body.clone();
        thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 2 {
                    line.clear();
                }
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", served.len());
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&served).unwrap();
            }
        });
        let dir = std::env::temp_dir().join(format!("iptv-update-test-{}", std::process::id()));
        let dest = dir.join("IPTV-Setup-9.9.9.exe");
        let mut update = Update {
            version: "9.9.9".into(),
            file_name: "IPTV-Setup-9.9.9.exe".into(),
            url: format!("http://{addr}/x"),
            sha256: sha,
        };
        download(&update, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), body);

        update.sha256 = "0".repeat(64);
        assert!(download(&update, &dir.join("bad.exe")).is_err());
        assert!(!dir.join("bad.exe").exists() && !dir.join("bad.part").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
