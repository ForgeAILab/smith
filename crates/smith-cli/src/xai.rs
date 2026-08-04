//! Smith's xAI browser login surface.
//!
//! The device grant only, deliberately. A loopback redirect needs a local
//! listener on a fixed port, which fails on a remote shell and in a container —
//! exactly where a terminal agent tends to run. The device grant needs nothing
//! but a browser somewhere the user can reach.

use anyhow::{Context, Result};
use smith_runtime::xai::{XaiEndpoints, XaiOAuthClient, XaiTokenBundle};

/// Runs the xAI login and returns a session only after it completes.
pub(super) async fn login(no_motion: bool) -> Result<XaiTokenBundle> {
    let oauth = XaiOAuthClient::new().context("initializing Smith's xAI OAuth client")?;
    let endpoints = oauth
        .discover()
        .await
        .context("reading xAI's published OAuth configuration")?;
    device_login(&oauth, &endpoints, no_motion).await
}

async fn device_login(
    oauth: &XaiOAuthClient,
    endpoints: &XaiEndpoints,
    _no_motion: bool,
) -> Result<XaiTokenBundle> {
    let authorization = oauth
        .request_device_code(endpoints)
        .await
        .context("starting xAI device-code login")?;

    // Prefer the pre-filled URL when the issuer offers one: it removes the
    // step where a user mistypes the code.
    let destination = authorization
        .verification_url_complete
        .as_deref()
        .unwrap_or(&authorization.verification_url);
    let opened = open_browser(destination);

    println!("Sign in to xAI to authorize Smith.");
    println!("  code: {}", authorization.user_code);
    println!("  open: {destination}");
    if !opened {
        println!("  (open that URL yourself — Smith could not launch a browser)");
    }
    println!("Waiting for approval…");

    let bundle = oauth
        .complete_device_code(endpoints, &authorization, now_ms())
        .await
        .context("completing xAI device-code login")?;
    println!("Signed in to xAI.");
    Ok(bundle)
}

/// Milliseconds since the Unix epoch.
///
/// Token lifetimes are wall-clock facts from the issuer, so they are stored
/// against wall-clock time rather than a monotonic instant that resets.
pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn open_browser(url: &str) -> bool {
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
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}
