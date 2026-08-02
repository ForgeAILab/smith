//! Smith-owned ChatGPT browser and device-code login surfaces.

use std::future::Future;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use crossterm::event::{Event as TermEvent, EventStream, KeyCode, KeyModifiers};
use futures_util::StreamExt;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use sha2::{Digest, Sha256};
use smith_runtime::chatgpt::{
    BrowserAuthorization, ChatGptOAuthClient, ChatGptTokenBundle, browser_authorization_url,
};
use smith_tui::theme::Theme;
use smith_tui::{PickerOutcome, ResourceEntry, ResourcePicker, draw_resource_picker};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;
use zeroize::Zeroizing;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_CALLBACK_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoginMethod {
    Browser,
    DeviceCode,
}

#[derive(Debug, Clone)]
struct LoginDisplay {
    destination: String,
    user_code: Option<String>,
    browser_opened: bool,
}

/// Runs Smith's experimental direct ChatGPT login and returns a token bundle
/// only after the complete ceremony succeeds.
pub(super) async fn login(no_color: bool, no_motion: bool) -> Result<Option<ChatGptTokenBundle>> {
    let Some(method) = choose_login_method(no_color, no_motion).await? else {
        return Ok(None);
    };
    let oauth = ChatGptOAuthClient::new().context("initializing Smith's ChatGPT OAuth client")?;
    match method {
        LoginMethod::Browser => browser_login(oauth, no_motion).await.map(Some),
        LoginMethod::DeviceCode => device_login(oauth, no_motion).await.map(Some),
    }
}

async fn browser_login(oauth: ChatGptOAuthClient, no_motion: bool) -> Result<ChatGptTokenBundle> {
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
    wait_for_login_surface(display, completion, no_motion).await
}

async fn device_login(oauth: ChatGptOAuthClient, no_motion: bool) -> Result<ChatGptTokenBundle> {
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
    wait_for_login_surface(display, completion, no_motion).await
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
    display: LoginDisplay,
    completion: F,
    no_motion: bool,
) -> Result<ChatGptTokenBundle>
where
    F: Future<Output = Result<ChatGptTokenBundle>>,
{
    let mut terminal = crate::terminal::enter().context("entering ChatGPT login progress")?;
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let result = {
        let completion = tokio::time::timeout(LOGIN_TIMEOUT, completion);
        tokio::pin!(completion);
        let mut frame_number = 0_usize;
        loop {
            terminal
                .draw(|frame| draw_login_progress(frame, &display, frame_number, no_motion))
                .context("drawing ChatGPT login progress")?;
            tokio::select! {
                result = &mut completion => {
                    break result
                        .context("ChatGPT login timed out")?;
                }
                event = events.next() => {
                    let Some(event) = event else {
                        anyhow::bail!("ChatGPT login input ended");
                    };
                    match event.context("reading ChatGPT login progress input")? {
                        TermEvent::Key(key)
                            if key.code == KeyCode::Esc
                                || (key.code == KeyCode::Char('c')
                                    && key.modifiers.contains(KeyModifiers::CONTROL)) =>
                        {
                            anyhow::bail!("ChatGPT login cancelled");
                        }
                        _ => {}
                    }
                }
                _ = tick.tick() => frame_number = frame_number.wrapping_add(1),
            }
        }
    };
    terminal
        .restore()
        .context("restoring the terminal after ChatGPT login progress")?;
    result
}

fn draw_login_progress(
    frame: &mut ratatui::Frame<'_>,
    display: &LoginDisplay,
    frame_number: usize,
    no_motion: bool,
) {
    let area = crate::resources::standalone_picker_area(frame.area(), 9);
    frame.render_widget(Clear, area);
    let text = login_progress_lines(display, frame_number, no_motion).join("\n");
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Connect ChatGPT · experimental "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn login_progress_lines(display: &LoginDisplay, frame: usize, no_motion: bool) -> Vec<String> {
    let mut lines = vec![
        "Smith owns this OAuth login and will call ChatGPT directly.".to_owned(),
        "Experimental: this is not a supported OpenAI Platform API contract.".to_owned(),
        String::new(),
        "Open this URL:".to_owned(),
        display.destination.clone(),
    ];
    if let Some(code) = &display.user_code {
        lines.push(format!("Enter code: {code}"));
    } else if display.browser_opened {
        lines.push("A browser window was requested; the URL remains copyable.".to_owned());
    } else {
        lines.push("Copy the URL; the browser could not be opened automatically.".to_owned());
    }
    let dots = if no_motion { 1 } else { frame % 3 + 1 };
    lines.push(String::new());
    lines.push(format!("Waiting for ChatGPT{}", ".".repeat(dots)));
    lines.push("Esc or Ctrl+C cancels without saving credentials.".to_owned());
    lines
}

async fn choose_login_method(no_color: bool, no_motion: bool) -> Result<Option<LoginMethod>> {
    let mut picker = ResourcePicker::new(
        "Connect ChatGPT · experimental",
        vec![
            ResourceEntry::new(
                "browser",
                "Browser login",
                "Smith PKCE callback · owner-only auth.json · direct Responses calls",
            ),
            ResourceEntry::new(
                "device",
                "Device-code login",
                "one-time code for callback-limited environments",
            ),
        ],
        "No supported ChatGPT login method",
    );
    let mut theme = Theme::from_env();
    if no_color {
        theme = theme.without_color();
    }
    if no_motion {
        theme = theme.without_motion();
    }
    let mut terminal = crate::terminal::enter().context("entering ChatGPT login picker")?;
    let mut events = EventStream::new();
    let result = async {
        loop {
            terminal
                .draw(|frame| {
                    let area = crate::resources::standalone_picker_area(
                        frame.area(),
                        picker.entries.len(),
                    );
                    draw_resource_picker(frame, area, &picker, theme);
                })
                .context("drawing ChatGPT login picker")?;
            let Some(event) = events.next().await else {
                return Ok(None);
            };
            match event.context("reading ChatGPT login picker input")? {
                TermEvent::Key(key) => match picker.on_key(key) {
                    PickerOutcome::Pending => {}
                    PickerOutcome::Cancelled => return Ok(None),
                    PickerOutcome::Selected(method) => {
                        return Ok(match method.as_str() {
                            "browser" => Some(LoginMethod::Browser),
                            "device" => Some(LoginMethod::DeviceCode),
                            _ => None,
                        });
                    }
                },
                TermEvent::Paste(text) => picker.paste(&text),
                TermEvent::Resize(_, _) => {}
                _ => {}
            }
        }
    }
    .await;
    terminal
        .restore()
        .context("restoring the terminal after ChatGPT login selection")?;
    result
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

    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(rendered.contains("Smith owns"));
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
