//! Small OS integration helpers for running as a GUI application.

/// On Windows, release builds have no console of their own. When launched
/// from a terminal, attach to it so `--help`, `--list` etc. still print.
/// Returns true if a console is available.
#[cfg(windows)]
pub fn attach_parent_console() -> bool {
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetConsoleWindow, ATTACH_PARENT_PROCESS,
    };
    // SAFETY: plain Win32 calls without pointer arguments.
    unsafe { !GetConsoleWindow().is_null() || AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

/// Elsewhere: whether errors printed to stderr can be seen (started from a
/// terminal rather than the Dock, Finder or an app menu).
#[cfg(not(windows))]
pub fn attach_parent_console() -> bool {
    use std::io::IsTerminal;
    std::io::stderr().is_terminal()
}

/// Shows an error dialog (used when there is no console to print to).
#[cfg(windows)]
pub fn error_box(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("IPTV"));
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// macOS: a standard alert (via AppleScript, always available).
#[cfg(target_os = "macos")]
pub fn error_box(message: &str) {
    let quoted = message.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display alert \"IPTV\" message \"{quoted}\" as critical");
    let _ = std::process::Command::new("osascript")
        .args(["-e", &script])
        .status();
}

/// Linux: whichever dialog tool the desktop has (GNOME, KDE...); nothing if
/// none is installed.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn error_box(message: &str) {
    use std::process::Command;
    let tries: [(&str, Vec<&str>); 3] = [
        (
            "zenity",
            vec!["--error", "--title=IPTV", "--no-markup", "--text", message],
        ),
        ("kdialog", vec!["--title", "IPTV", "--error", message]),
        ("xmessage", vec!["-center", message]),
    ];
    for (tool, args) in tries {
        if Command::new(tool).args(&args).status().is_ok() {
            return;
        }
    }
}
