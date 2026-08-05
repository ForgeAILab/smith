//! Best-effort platform browser launch for login ceremonies.

use std::process::Stdio;

/// Asks the platform to open `url`, reporting whether the request was handed
/// off.
///
/// Spawn-and-detach on purpose: the opener commands return once the browser
/// has the URL, and a login surface must keep rendering its copyable fallback
/// rather than wait on a process it does not control.
pub(crate) fn open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return false;

    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        command
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
}
