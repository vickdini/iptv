//! Small OS integration helpers for running as a GUI application.

/// On Windows, release builds have no console of their own. When launched
/// from a terminal, attach to it so `--help`, `--list` etc. still print.
/// Returns true if a console is available.
#[cfg(windows)]
pub fn attach_parent_console() -> bool {
    use windows_sys::Win32::System::Console::{AttachConsole, GetConsoleWindow, ATTACH_PARENT_PROCESS};
    // SAFETY: plain Win32 calls without pointer arguments.
    unsafe { !GetConsoleWindow().is_null() || AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

#[cfg(not(windows))]
pub fn attach_parent_console() -> bool {
    true
}

/// Shows an error dialog (used when there is no console to print to).
#[cfg(windows)]
pub fn error_box(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("IPTV"));
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(windows))]
pub fn error_box(_message: &str) {}
