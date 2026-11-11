//! Automatic updates for the installed desktop app.
//!
//! On every launch: check the latest GitHub release for a newer version of
//! this platform's download, fetch and verify it in the background, then
//! install it and restart IPTV:
//!
//! * Windows: `IPTV-Setup-<version>.exe`, run silently; the installer
//!   restarts IPTV. Only installed copies update (the uninstaller sits next
//!   to iptv.exe).
//! * Linux: `IPTV-<version>-x86_64.AppImage` replaces the running AppImage
//!   file (same path, so menu entries and shortcuts keep working).
//! * macOS: `IPTV-<version>-macOS-<arch>.dmg`; its IPTV.app replaces the
//!   current one. Not while running from the disk image itself, or from the
//!   read-only copy macOS makes of apps opened straight from Downloads.
//!
//! Copies run from source don't update. `IPTV_UPDATE_URL` points the check
//! at another release location (for testing).

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;

use anyhow::{Context, Result};
use iptv_net::update::{self, Update};

use crate::storage;

/// How this copy of IPTV was installed, if it can update itself.
enum Install {
    #[cfg_attr(not(windows), allow(dead_code))]
    WindowsInstaller,
    /// The AppImage file being run.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    AppImage(PathBuf),
    /// The IPTV.app bundle being run.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MacApp(PathBuf),
}

fn install_kind() -> Option<Install> {
    let exe = std::env::current_exe().ok()?;
    #[cfg(windows)]
    {
        let installed = exe.parent()?.join("unins000.exe").is_file();
        (installed || std::env::var_os("IPTV_UPDATE_URL").is_some())
            .then_some(Install::WindowsInstaller)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = exe;
        // Set by the AppImage runtime to the .AppImage file's path.
        let file = PathBuf::from(std::env::var_os("APPIMAGE")?);
        (file.is_file() && writable_dir(file.parent()?)).then_some(Install::AppImage(file))
    }
    #[cfg(target_os = "macos")]
    {
        // .../IPTV.app/Contents/MacOS/iptv
        let app = exe.parent()?.parent()?.parent()?.to_path_buf();
        let path = app.to_string_lossy();
        let movable = path.ends_with(".app")
            && !path.starts_with("/Volumes/")
            && !path.contains("/AppTranslocation/")
            && writable_dir(app.parent()?);
        movable.then_some(Install::MacApp(app))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = exe;
        None
    }
}

/// Whether we may create files in `dir` (to put the new version there).
#[cfg(unix)]
fn writable_dir(dir: &Path) -> bool {
    let probe = dir.join(format!(".iptv-write-test-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// The release file for this platform: name prefix and suffix around the
/// version.
fn asset() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("IPTV-Setup-", ".exe")
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            ("IPTV-", "-macOS-arm64.dmg")
        } else {
            ("IPTV-", "-macOS-x86_64.dmg")
        }
    } else {
        ("IPTV-", "-x86_64.AppImage")
    }
}

/// Whether this copy should update itself.
pub fn enabled() -> bool {
    install_kind().is_some()
}

fn releases_url() -> String {
    std::env::var("IPTV_UPDATE_URL").unwrap_or_else(|_| update::RELEASES.to_string())
}

/// Checks for and downloads a newer version. Blocking; returns the update
/// and the verified download's path, or None if this is the latest version.
pub fn fetch() -> Result<Option<(Update, PathBuf)>> {
    let (prefix, suffix) = asset();
    let Some(update) = update::check(&releases_url(), prefix, suffix, env!("CARGO_PKG_VERSION"))?
    else {
        return Ok(None);
    };
    let path = storage::cache_dir().join("updates").join(&update.file_name);
    update::download(&update, &path)?;
    Ok(Some((update, path)))
}

/// Installs the downloaded update and starts the new version (on Windows
/// the installer does that). The caller exits right after.
pub fn install(download: &Path) -> Result<()> {
    match install_kind().context("this copy of IPTV can't update itself")? {
        Install::WindowsInstaller => {
            std::process::Command::new(download)
                .args([
                    "/VERYSILENT",
                    "/SUPPRESSMSGBOXES",
                    "/NORESTART",
                    "/CURRENTUSER",
                    "/RELAUNCH",
                ])
                .spawn()?;
            Ok(())
        }
        #[cfg(unix)]
        Install::AppImage(file) => {
            use std::os::unix::fs::PermissionsExt;
            // Copy next to the old file, then swap it in at once: the running
            // AppImage keeps working until we exit.
            let tmp = file.with_file_name(".iptv-update.AppImage");
            std::fs::copy(download, &tmp).context("could not write the update")?;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&tmp, &file).context("could not replace the AppImage")?;
            relaunch(Command::new(&file))
        }
        #[cfg(unix)]
        Install::MacApp(app) => {
            let mount = storage::cache_dir().join("updates").join("mount");
            std::fs::create_dir_all(&mount)?;
            run(Command::new("hdiutil")
                .args([
                    "attach",
                    "-nobrowse",
                    "-readonly",
                    "-noautoopen",
                    "-mountpoint",
                ])
                .arg(&mount)
                .arg(download))?;
            let parent = app.parent().context("app has no folder")?;
            let staged = parent.join(".IPTV-update.app");
            let old = parent.join(".IPTV-old.app");
            let _ = std::fs::remove_dir_all(&staged);
            let copied = run(Command::new("ditto")
                .arg(mount.join("IPTV.app"))
                .arg(&staged));
            let _ = run(Command::new("hdiutil")
                .arg("detach")
                .arg(&mount)
                .arg("-quiet"));
            copied?;
            let _ = std::fs::remove_dir_all(&old);
            std::fs::rename(&app, &old).context("could not move the old version aside")?;
            if let Err(e) = std::fs::rename(&staged, &app) {
                let _ = std::fs::rename(&old, &app);
                return Err(e).context("could not put the new version in place");
            }
            let _ = std::fs::remove_dir_all(&old);
            let mut open = Command::new("open");
            open.arg("-n").arg(&app);
            relaunch(open)
        }
        #[allow(unreachable_patterns)]
        _ => anyhow::bail!("unsupported install"),
    }
}

/// Runs a helper command, failing if it does.
#[cfg(unix)]
fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("could not run {cmd:?}"))?;
    anyhow::ensure!(status.success(), "{cmd:?} failed ({status})");
    Ok(())
}

/// Starts the new version a moment after we exit (so it reads the settings
/// we save on the way out).
#[cfg(unix)]
fn relaunch(cmd: Command) -> Result<()> {
    let program = cmd.get_program().to_owned();
    let mut sh = Command::new("/bin/sh");
    sh.arg("-c")
        .arg(r#"sleep 2; exec "$@""#)
        .arg("sh")
        .arg(program)
        .args(cmd.get_args());
    sh.stdin(std::process::Stdio::null());
    sh.spawn().context("could not restart IPTV")?;
    Ok(())
}

/// Deletes downloads from earlier updates.
pub fn clean_old_downloads() {
    let dir = storage::cache_dir().join("updates");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                let _ = std::fs::remove_dir(&path); // an empty mount point
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}
