//! Smith-owned ChatGPT browser and device-code login surfaces.

use std::convert::Infallible;
use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ratatui::layout::Rect;
use sha2::{Digest, Sha256};
use smith_runtime::chatgpt::{
    BrowserAuthorization, ChatGptOAuthClient, ChatGptTokenBundle, browser_authorization_url,
};
use smith_tui::picker::ScreenFooter;
use smith_tui::theme::Theme;
use smith_tui::{FlowOutcome, ResourceEntry, ResourcePicker, Screen, ScreenEvent, Step};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::browser::open_browser;
use crate::screen_runner::{ScreenContext, ScreenResult, ScreenSession};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_CALLBACK_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginMethod {
    Browser,
    DeviceCode,
}

#[derive(Debug, Clone)]
pub(super) struct LoginDisplay {
    pub(super) destination: String,
    pub(super) user_code: Option<String>,
    pub(super) browser_opened: bool,
}

/// Runs Smith's experimental direct ChatGPT login and returns a token bundle
/// only after the complete ceremony succeeds.
pub(super) async fn login(
    session: &mut ScreenSession<'_>,
    picker: &mut ResourcePicker,
    no_motion: bool,
    from_setup: bool,
) -> Result<FlowOutcome<ChatGptTokenBundle>> {
    loop {
        let method = match choose_login_method(session, picker).await? {
            FlowOutcome::Completed(method) => method,
            FlowOutcome::Back => return Ok(FlowOutcome::Back),
            FlowOutcome::Cancelled => return Ok(FlowOutcome::Cancelled),
        };
        let oauth =
            ChatGptOAuthClient::new().context("initializing Smith's ChatGPT OAuth client")?;
        let outcome = match method {
            LoginMethod::Browser => browser_login(oauth, session, no_motion, from_setup).await?,
            LoginMethod::DeviceCode => device_login(oauth, session, no_motion, from_setup).await?,
        };
        match outcome {
            // Dropping the raced callback/poll future closes the backend before
            // returning to the same picker with its selected method retained.
            FlowOutcome::Back => continue,
            outcome => return Ok(outcome),
        }
    }
}

async fn browser_login(
    oauth: ChatGptOAuthClient,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
    from_setup: bool,
) -> Result<FlowOutcome<ChatGptTokenBundle>> {
    let (listener, port) = bind_callback().await?;
    let redirect_uri = format!("http://localhost:{port}/auth/callback");
    let verifier = Zeroizing::new(format!(
        "{}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    ));
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = Zeroizing::new(format!(
        "{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    ));
    let destination = browser_authorization_url(BrowserAuthorization {
        redirect_uri: &redirect_uri,
        code_challenge: &challenge,
        state: &state,
    });
    let browser_opened = open_browser(&destination);
    let display = LoginDisplay {
        destination,
        user_code: None,
        browser_opened,
    };
    let completion = async {
        let code = wait_for_callback(listener, &state).await?;
        oauth
            .exchange_authorization_code(&code, &redirect_uri, &verifier)
            .await
            .context("exchanging the ChatGPT browser authorization")
    };
    wait_for_login_surface(session, display, completion, no_motion, from_setup).await
}

async fn device_login(
    oauth: ChatGptOAuthClient,
    session: &mut ScreenSession<'_>,
    no_motion: bool,
    from_setup: bool,
) -> Result<FlowOutcome<ChatGptTokenBundle>> {
    let authorization = oauth
        .request_device_code()
        .await
        .context("starting ChatGPT device-code login")?;
    let display = LoginDisplay {
        destination: authorization.verification_url().to_owned(),
        user_code: Some(authorization.user_code().to_owned()),
        browser_opened: open_browser(authorization.verification_url()),
    };
    let completion = async {
        oauth
            .complete_device_code(&authorization)
            .await
            .context("completing ChatGPT device-code login")
    };
    wait_for_login_surface(session, display, completion, no_motion, from_setup).await
}

async fn bind_callback() -> Result<(TcpListener, u16)> {
    for port in [1455_u16, 1457_u16] {
        match TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {}
            Err(_) => anyhow::bail!("Smith could not bind the ChatGPT localhost callback"),
        }
    }
    anyhow::bail!("ChatGPT login needs localhost port 1455 or 1457; both are in use")
}

async fn wait_for_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<Zeroizing<String>> {
    let (mut stream, peer) = listener
        .accept()
        .await
        .context("accepting the ChatGPT localhost callback")?;
    if !peer.ip().is_loopback() {
        respond(&mut stream, 403, "Callback rejected.").await;
        anyhow::bail!("ChatGPT login callback was rejected")
    }
    let request = read_request_head(&mut stream).await?;
    let first_line = request
        .lines()
        .next()
        .context("ChatGPT login callback was malformed")?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next();
    let target = parts.next();
    if method != Some("GET") || parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        respond(&mut stream, 400, "Callback rejected.").await;
        anyhow::bail!("ChatGPT login callback was rejected")
    }
    let target = target.context("ChatGPT login callback was malformed")?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/auth/callback" || query.contains('#') {
        respond(&mut stream, 404, "Not found.").await;
        anyhow::bail!("ChatGPT login callback target was rejected")
    }
    let mut callback_state = Zeroizing::new(String::new());
    let mut code = Zeroizing::new(String::new());
    let mut denied = false;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "state" => callback_state.push_str(&value),
            "code" => code.push_str(&value),
            "error" => denied = true,
            _ => {}
        }
    }
    if callback_state.as_str() != expected_state {
        respond(
            &mut stream,
            400,
            "State mismatch. Return to Smith and retry.",
        )
        .await;
        anyhow::bail!("ChatGPT login callback state did not match")
    }
    if denied {
        respond(&mut stream, 403, "ChatGPT login was denied.").await;
        anyhow::bail!("ChatGPT login was denied")
    }
    if code.is_empty() || code.len() > 2_048 {
        respond(&mut stream, 400, "Missing authorization code.").await;
        anyhow::bail!("ChatGPT login callback did not contain a usable authorization code")
    }
    respond(
        &mut stream,
        200,
        "ChatGPT login received. You can close this window and return to Smith.",
    )
    .await;
    Ok(code)
}

async fn read_request_head(stream: &mut TcpStream) -> Result<Zeroizing<String>> {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0_u8; 1_024];
        let count = stream
            .read(&mut chunk)
            .await
            .context("reading the ChatGPT localhost callback")?;
        if count == 0 {
            anyhow::bail!("ChatGPT login callback ended before its headers")
        }
        if bytes.len().saturating_add(count) > MAX_CALLBACK_BYTES {
            anyhow::bail!("ChatGPT login callback exceeded Smith's size limit")
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8(bytes)
        .map(Zeroizing::new)
        .context("ChatGPT login callback was not valid UTF-8")
}

async fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn wait_for_login_surface<F>(
    session: &mut ScreenSession<'_>,
    display: LoginDisplay,
    completion: F,
    no_motion: bool,
    from_setup: bool,
) -> Result<FlowOutcome<ChatGptTokenBundle>>
where
    F: Future<Output = Result<ChatGptTokenBundle>>,
{
    let mut progress = LoginProgress {
        display,
        frame_number: 0,
        no_motion,
        from_setup,
    };
    match session
        .run_screen(
            &mut progress,
            Some(tokio::time::timeout(LOGIN_TIMEOUT, completion)),
            ScreenContext {
                draw: Some("drawing ChatGPT login progress"),
                input: "reading ChatGPT login progress input",
            },
        )
        .await?
    {
        ScreenResult::Completed(result) => result
            .context("ChatGPT login timed out")?
            .map(FlowOutcome::Completed),
        ScreenResult::Outcome(outcome) => Ok(outcome),
        ScreenResult::InputEnded => Ok(FlowOutcome::Cancelled),
        ScreenResult::Effect(never) => match never {},
    }
}

/// Presentation-only OAuth wait state; the runner owns the completion future.
struct LoginProgress {
    display: LoginDisplay,
    frame_number: usize,
    // This surface historically uses the explicit flag rather than the theme's
    // environment-derived motion setting. Preserve that during the loop merge.
    no_motion: bool,
    from_setup: bool,
}

impl Screen for LoginProgress {
    type Outcome = FlowOutcome<ChatGptTokenBundle>;
    type Effect = Infallible;

    /// Browser and device-code instructions are distinct steps; ticks are not.
    fn step_key(&self) -> u64 {
        u64::from(self.display.user_code.is_some())
    }

    fn draw(&self, frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
        draw_login_progress_in_area(
            frame,
            area,
            &self.display,
            self.frame_number,
            self.no_motion,
            self.from_setup,
            theme,
        );
    }

    fn draw_embedded(&self, frame: &mut ratatui::Frame<'_>, area: Rect, theme: Theme) {
        crate::login_progress::draw_progress(
            frame,
            area,
            "Connect ChatGPT · experimental",
            embedded_login_progress_lines(&self.display, self.frame_number, self.no_motion),
            self.from_setup,
            true,
            theme,
        );
    }

    fn content_height(&self, width: u16) -> u16 {
        crate::login_progress::progress_height(
            embedded_login_progress_lines(&self.display, self.frame_number, self.no_motion),
            width,
        )
    }

    fn footer(&self) -> Option<ScreenFooter> {
        Some(ScreenFooter::Progress {
            back: self.from_setup,
        })
    }

    fn on_event(&mut self, event: ScreenEvent) -> Step<Self::Outcome, Self::Effect> {
        crate::login_progress::progress_event(event, &mut self.frame_number, self.from_setup)
    }

    fn tick_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(250))
    }
}

#[cfg(test)]
/// Keeps fixture recordings on the production progress renderer without a terminal runner.
pub(super) fn draw_login_progress(
    frame: &mut ratatui::Frame<'_>,
    display: &LoginDisplay,
    frame_number: usize,
    no_motion: bool,
) {
    let area = frame.area();
    draw_login_progress_in_area(
        frame,
        area,
        display,
        frame_number,
        no_motion,
        false,
        Theme::new().without_color(),
    );
}

fn draw_login_progress_in_area(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    display: &LoginDisplay,
    frame_number: usize,
    no_motion: bool,
    from_setup: bool,
    theme: Theme,
) {
    crate::login_progress::draw_progress(
        frame,
        area,
        "Connect ChatGPT · experimental",
        login_progress_lines(display, frame_number, no_motion),
        from_setup,
        false,
        theme,
    );
}

fn login_progress_lines(display: &LoginDisplay, frame: usize, no_motion: bool) -> Vec<String> {
    login_progress_lines_for_surface(display, frame, no_motion, false)
}

/// Put actionable instructions first because the embedded pane shares height with a transcript.
fn embedded_login_progress_lines(
    display: &LoginDisplay,
    frame: usize,
    no_motion: bool,
) -> Vec<String> {
    login_progress_lines_for_surface(display, frame, no_motion, true)
}

fn login_progress_lines_for_surface(
    display: &LoginDisplay,
    frame: usize,
    no_motion: bool,
    embedded: bool,
) -> Vec<String> {
    let mut introduction = vec![
        "Sign in to ChatGPT in your browser.".to_owned(),
        "Experimental: ChatGPT sign-in is not an official API for Smith.".to_owned(),
        String::new(),
    ];
    let mut instructions = vec!["Open this URL:".to_owned(), display.destination.clone()];
    if let Some(code) = &display.user_code {
        let code = format!("Enter code: {code}");
        if embedded {
            // A wrapping URL must not consume the code's only visible row.
            instructions.insert(0, code);
        } else {
            instructions.push(code);
        }
    } else if display.browser_opened {
        instructions.push("A browser window was requested; the URL remains copyable.".to_owned());
    } else {
        instructions
            .push("Copy the URL; the browser could not be opened automatically.".to_owned());
    }
    let mut lines = if embedded {
        instructions.append(&mut introduction);
        instructions
    } else {
        introduction.append(&mut instructions);
        introduction
    };
    let dots = if no_motion {
        "…".to_owned()
    } else {
        ".".repeat(frame % 3 + 1)
    };
    lines.push(String::new());
    lines.push(format!("Waiting for ChatGPT{dots}"));
    lines
}

/// Initial picker shared by the login loop and its terminal fixtures.
pub(super) fn login_method_picker() -> ResourcePicker {
    ResourcePicker::choices(
        "Connect ChatGPT · experimental",
        vec![
            ResourceEntry::new("browser", "Browser login", "sign in in your browser"),
            ResourceEntry::new(
                "device",
                "Device-code login",
                "enter a one-time code on any device",
            ),
        ],
        "No supported ChatGPT login method",
    )
}

async fn choose_login_method(
    session: &mut ScreenSession<'_>,
    picker: &mut ResourcePicker,
) -> Result<FlowOutcome<LoginMethod>> {
    match session
        .run(
            picker,
            ScreenContext {
                draw: Some("drawing ChatGPT login picker"),
                input: "reading ChatGPT login picker input",
            },
        )
        .await?
    {
        ScreenResult::Outcome(FlowOutcome::Completed(method)) => match method.as_str() {
            "browser" => Ok(FlowOutcome::Completed(LoginMethod::Browser)),
            "device" => Ok(FlowOutcome::Completed(LoginMethod::DeviceCode)),
            _ => Ok(FlowOutcome::Cancelled),
        },
        ScreenResult::Outcome(FlowOutcome::Back) => Ok(FlowOutcome::Back),
        ScreenResult::Outcome(FlowOutcome::Cancelled) | ScreenResult::InputEnded => {
            Ok(FlowOutcome::Cancelled)
        }
        ScreenResult::Effect(never) | ScreenResult::Completed(never) => match never {},
    }
}

#[cfg(test)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn embedded_login_keeps_code_url_waiting_and_session_visible() {
        for (width, height) in [(44, 16), (100, 32)] {
            for user_code in [None, Some("ABCD-1234".to_owned())] {
                let progress = LoginProgress {
                    display: LoginDisplay {
                        destination: "https://auth.openai.com/codex/device".into(),
                        user_code: user_code.clone(),
                        browser_opened: true,
                    },
                    frame_number: 0,
                    no_motion: true,
                    from_setup: false,
                };
                let mut app = smith_tui::App::new("gpt-5.3", "~/work/api");
                app.transcript.push_user("retained transcript");
                app.composer.insert_str("retained draft");
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                        .expect("terminal");
                terminal
                    .draw(|frame| {
                        smith_tui::render::draw_with_screen(
                            frame,
                            &app,
                            &progress,
                            Theme::new().without_color(),
                        );
                    })
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
                for expected in [
                    "retained transcript",
                    "retained draft",
                    "Connect ChatGPT · experimental",
                    "https://auth.openai.com",
                    "Waiting for ChatGPT…",
                    "esc cancel",
                ] {
                    assert!(text.contains(expected), "{text}");
                }
                if let Some(code) = user_code {
                    assert!(text.contains(&format!("Enter code: {code}")), "{text}");
                }
                assert!(!text.contains("Smith setup"), "{text}");
            }
        }
    }

    #[test]
    fn login_progress_keeps_waiting_and_navigation_visible_at_44_by_16() {
        for user_code in [None, Some("ABCD-1234".to_owned())] {
            let display = LoginDisplay {
                destination: "https://auth.openai.com/authorize?state=fixture-state".into(),
                user_code,
                browser_opened: true,
            };
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(44, 16))
                .expect("terminal");
            terminal
                .draw(|frame| {
                    draw_login_progress_in_area(
                        frame,
                        frame.area(),
                        &display,
                        0,
                        true,
                        true,
                        Theme::new().without_color(),
                    )
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            let screen = (0..16)
                .map(|y| (0..44).map(|x| buffer[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                screen
                    .lines()
                    .next()
                    .expect("heading")
                    .starts_with("  Connect ChatGPT · experimental"),
                "{screen}"
            );
            for text in [
                "Open this URL:",
                "https://auth.openai.com",
                "Waiting for ChatGPT…",
                "esc back",
                "ctrl+c cancel",
            ] {
                assert!(screen.contains(text), "{screen}");
            }
            if display.user_code.is_some() {
                assert!(screen.contains("Enter code: ABCD-1234"), "{screen}");
            }
            assert!(!screen.contains('┌') && !screen.contains('│'), "{screen}");
        }
    }

    #[test]
    fn setup_login_escape_backs_out_and_ctrl_c_cancels_every_step() {
        let mut methods = login_method_picker().with_back(true);
        assert!(matches!(
            methods.on_event(ScreenEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE
            ))),
            Step::Outcome(FlowOutcome::Back)
        ));
        let mut progress = LoginProgress {
            display: LoginDisplay {
                destination: "https://auth.openai.com".into(),
                user_code: None,
                browser_opened: false,
            },
            frame_number: 0,
            no_motion: true,
            from_setup: true,
        };
        let browser_step = progress.step_key();
        progress.on_event(ScreenEvent::Tick);
        assert_eq!(progress.step_key(), browser_step);
        progress.display.user_code = Some("ABCD-1234".into());
        assert_ne!(progress.step_key(), browser_step);
        assert!(matches!(
            progress.on_event(ScreenEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE
            ))),
            Step::Outcome(FlowOutcome::Back)
        ));
        for from_setup in [true, false] {
            progress.from_setup = from_setup;
            assert!(matches!(
                progress.on_event(ScreenEvent::Key(crossterm::event::KeyEvent::new(
                    KeyCode::Char('c'),
                    KeyModifiers::CONTROL
                ))),
                Step::Outcome(FlowOutcome::Cancelled)
            ));
        }
        let mut first = login_method_picker();
        assert!(matches!(
            first.on_event(ScreenEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE
            ))),
            Step::Outcome(FlowOutcome::Cancelled)
        ));
        assert!(matches!(
            methods.on_event(ScreenEvent::Key(crossterm::event::KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Step::Outcome(FlowOutcome::Cancelled)
        ));
    }

    #[test]
    fn login_progress_keeps_only_public_ceremony_material() {
        let rendered = login_progress_lines(
            &LoginDisplay {
                destination: "https://auth.openai.com/codex/device".into(),
                user_code: Some("ABCD-1234".into()),
                browser_opened: true,
            },
            0,
            true,
        )
        .join("\n");
        assert!(rendered.contains("ABCD-1234"));
        assert!(rendered.contains("Sign in to ChatGPT in your browser."));
        assert!(
            rendered.contains("Experimental: ChatGPT sign-in is not an official API for Smith.")
        );
        assert!(rendered.contains("Waiting for ChatGPT…"));
        let methods = login_method_picker();
        assert_eq!(methods.entries[0].description, "sign in in your browser");
        assert_eq!(
            methods.entries[1].description,
            "enter a one-time code on any device"
        );
        let methods = methods
            .entries
            .iter()
            .map(|entry| format!("{} {}", entry.description, entry.detail))
            .collect::<Vec<_>>()
            .join("\n");
        for internal in ["PKCE", "auth.json", "public API boundary", "OAuth"] {
            assert!(!rendered.contains(internal), "{rendered}");
            assert!(!methods.contains(internal), "{methods}");
        }
        assert!(!rendered.contains("access-token-canary"));
    }

    #[test]
    fn callback_target_parser_rejects_non_callback_paths() {
        let parsed = url::Url::parse("http://localhost/not-auth?code=x&state=y").expect("url");
        assert_ne!(parsed.path(), "/auth/callback");
    }

    #[tokio::test]
    async fn forged_callback_state_is_rejected_without_returning_a_code() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let address = listener.local_addr().expect("address");
        let callback = tokio::spawn(async move { wait_for_callback(listener, "expected").await });
        let mut client = TcpStream::connect(address).await.expect("client");
        client
            .write_all(
                b"GET /auth/callback?code=secret-code-canary&state=forged HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .await
            .expect("request");
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("response");
        let error = callback
            .await
            .expect("callback task")
            .expect_err("forged state");
        assert!(error.to_string().contains("state did not match"));
        let response = String::from_utf8(response).expect("response text");
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(!response.contains("secret-code-canary"));
    }
}
