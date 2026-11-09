//! Automatic updates for the installed Windows app.
//!
//! On every launch: check the latest GitHub release for a newer
//! `IPTV-Setup-<version>.exe`, download and verify it in the background, then
//! run it silently and exit; the installer restarts IPTV on the new version.
//!
//! Only installed copies update themselves (the installer's uninstaller sits
//! next to iptv.exe); copies run from source or a zip don't. `IPTV_UPDATE_URL`
//! points the check at another release location (for testing).

use std::path::{Path, PathBuf};

use anyhow::Result;
use iptv_net::update::{self, Update};

use crate::storage;

/// Whether this copy should update itself.
pub fn enabled() -> bool {
    if std::env::var_os("IPTV_UPDATE_URL").is_some() {
        return true;
    }
    cfg!(windows)
        && std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("unins000.exe").is_file()))
            .unwrap_or(false)
}

fn releases_url() -> String {
    std::env::var("IPTV_UPDATE_URL").unwrap_or_else(|_| update::RELEASES.to_string())
}

/// Checks for and downloads a newer installer. Blocking; returns the update
/// and the verified installer's path, or None if this is the latest version.
pub fn fetch() -> Result<Option<(Update, PathBuf)>> {
    let Some(update) = update::check(&releases_url(), "IPTV-Setup-", ".exe", env!("CARGO_PKG_VERSION"))? else {
        return Ok(None);
    };
    let path = storage::cache_dir().join("updates").join(&update.file_name);
    update::download(&update, &path)?;
    Ok(Some((update, path)))
}

/// Starts the downloaded installer silently; it closes IPTV's files, installs
/// over the current version and restarts IPTV. The caller exits right after.
pub fn install(installer: &Path) -> Result<()> {
    std::process::Command::new(installer)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CURRENTUSER", "/RELAUNCH"])
        .spawn()?;
    Ok(())
}

/// Deletes installers from earlier updates.
pub fn clean_old_downloads() {
    let dir = storage::cache_dir().join("updates");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
