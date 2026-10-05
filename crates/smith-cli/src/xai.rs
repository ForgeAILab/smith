//! Smith's xAI browser login surface.
//!
//! The device grant only, deliberately. A loopback redirect needs a local
//! listener on a fixed port, which fails on a remote shell and in a container —
//! exactly where a terminal agent tends to run. The device grant needs nothing
//! but a browser somewhere the user can reach.

use anyhow::{Context, Result};
use smith_runtime::xai::{XaiDeviceAuthorization, XaiEndpoints, XaiOAuthClient, XaiTokenBundle};
use smith_tui::FlowOutcome;

use crate::browser::open_browser;
use crate::login_progress::LoginProgress;
use crate::screen_runner::{ScreenContext, ScreenResult, ScreenSession};

/// Runs the xAI login and returns a session only after it completes.
pub(super) async fn login(
    session: &mut ScreenSession<'_>,
    no_motion: bool,
) -> Result<FlowOutcome<XaiTokenBundle>> {
    let oauth = XaiOAuthClient::new().context("initializing Smith's xAI OAuth client")?;
    let starting = LoginProgress::new(
        "Connect xAI",
        vec!["Getting sign-in instructions from xAI…".to_owned()],
        "Waiting for xAI",
        no_motion,
    );
    let request = async {
        let endpoints = oauth
            .discover()
            .await
            .context("reading xAI's published OAuth configuration")?;
        let authorization = oauth
            .request_device_code(&endpoints)
            .await
            .context("starting xAI device-code login")?;
        Ok((endpoints, authorization))
    };
    let (endpoints, authorization) = match wait_for_xai(session, starting, request).await? {
        FlowOutcome::Completed(ready) => ready,
        FlowOutcome::Back | FlowOutcome::Cancelled => return Ok(FlowOutcome::Cancelled),
    };
    device_login(&oauth, &endpoints, authorization, session, no_motion).await
}

async fn device_login(
    oauth: &XaiOAuthClient,
    endpoints: &XaiEndpoints,
    authorization: XaiDeviceAuthorization,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
) -> Result<FlowOutcome<XaiTokenBundle>> {
    // Prefer the pre-filled URL when the issuer offers one: it removes the
    // step where a user mistypes the code.
    let destination = authorization
        .verification_url_complete
        .as_deref()
        .unwrap_or(&authorization.verification_url);
    let opened = open_browser(destination);

    let progress = login_progress(&authorization.user_code, destination, opened, no_motion);
    let completion = async {
        oauth
            .complete_device_code(endpoints, &authorization, now_ms())
            .await
            .context("completing xAI device-code login")
    };
    wait_for_xai(session, progress, completion).await
}

/// Both instruction discovery and device polling remain cancellable on the shared runner.
async fn wait_for_xai<T, F>(
    session: &mut ScreenSession<'_>,
    mut progress: LoginProgress,
    completion: F,
) -> Result<FlowOutcome<T>>
where
    F: std::future::Future<Output = Result<T>>,
{
    match session
        .run_screen(
            &mut progress,
            Some(tokio::time::timeout(
                std::time::Duration::from_secs(10 * 60),
                completion,
            )),
            ScreenContext {
                draw: Some("drawing xAI login progress"),
                input: "reading xAI login progress input",
            },
        )
        .await?
    {
        ScreenResult::Completed(result) => result
            .context("xAI login timed out")?
            .map(FlowOutcome::Completed),
        ScreenResult::Outcome(_) | ScreenResult::InputEnded => Ok(FlowOutcome::Cancelled),
        ScreenResult::Effect(never) => match never {},
    }
}

/// Keep code and URL first so small inline panes preserve actionable instructions.
pub(super) fn login_progress(
    code: &str,
    destination: &str,
    opened: bool,
    no_motion: bool,
) -> LoginProgress {
    LoginProgress::new(
        "Connect xAI",
        vec![
            format!("Enter code: {code}"),
            format!("Open: {destination}"),
            if opened {
                "A browser window was requested.".to_owned()
            } else {
                "Open that URL yourself; Smith could not launch a browser.".to_owned()
            },
        ],
        "Waiting for xAI",
        no_motion,
    )
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

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use smith_tui::{FlowOutcome, Screen, ScreenEvent, Step, Theme};

    #[test]
    fn xai_progress_shows_public_instructions_and_cancels() {
        for (width, height) in [(44, 16), (100, 32)] {
            for opened in [false, true] {
                let mut progress = super::login_progress(
                    "ABCD-1234",
                    "https://auth.x.ai/activate?user_code=ABCD-1234",
                    opened,
                    true,
                );
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                        .expect("terminal");
                terminal
                    .draw(|frame| progress.draw(frame, frame.area(), Theme::new().without_color()))
                    .expect("draw");
                let buffer = terminal.backend().buffer();
                let text = (0..height)
                    .map(|y| {
                        (0..width)
                            .map(|x| buffer[(x, y)].symbol())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                for instruction in [
                    "Connect xAI",
                    "ABCD-1234",
                    "https://auth.x.ai",
                    "Waiting for xAI",
                    "esc cancel",
                ] {
                    assert!(text.contains(instruction), "{text}");
                }
                for status in if opened {
                    vec!["browser window was requested"]
                } else {
                    vec!["could not", "launch a browser"]
                } {
                    assert!(text.contains(status), "{text}");
                }
                assert!(matches!(
                    progress.on_event(ScreenEvent::Key(KeyEvent::new(
                        KeyCode::Esc,
                        KeyModifiers::NONE
                    ))),
                    Step::Outcome(FlowOutcome::Cancelled)
                ));
                assert!(matches!(
                    progress.on_event(ScreenEvent::Key(KeyEvent::new(
                        KeyCode::Char('c'),
                        KeyModifiers::CONTROL
                    ))),
                    Step::Outcome(FlowOutcome::Cancelled)
                ));
            }
        }
    }
}
