//! Recordings of the production standalone terminal screens.
//!
//! Keep the complete terminal height to reveal top-left placement and unused rows.

use std::path::PathBuf;

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::ids::SessionId;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, CellWidth};
use ratatui::{Frame, Terminal};
use smith_runtime::session::{SNAPSHOT_SCHEMA_VERSION, SessionListing};
use smith_tui::Screen;
use smith_tui::picker::draw_resource_picker;
use smith_tui::setup::{SetupApp, SetupEffect, SetupMode, draw_setup};
use smith_tui::{ResourcePicker, Theme};

use super::fixture_support;
use crate::{chatgpt, connection, resources, setup};

const SIZES: [(u16, u16); 2] = [(44, 16), (100, 32)];
const DESTINATION: &str = "/tmp/smith-home/.smith/config.toml";

fn fixture_screens(screen: &str, mut draw: impl FnMut(&mut Frame<'_>, Theme)) {
    for (width, height) in SIZES {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal
            .draw(|frame| draw(frame, Theme::new().without_color()))
            .expect("standalone frame");
        fixture_support::compare_or_update(
            &format!("standalone/{screen}.{width}x{height}.txt"),
            &fixture_screen(terminal.backend().buffer()),
        );
    }
}

fn fixture_screen(buffer: &Buffer) -> String {
    let rows = (buffer.area.y..buffer.area.bottom())
        .map(|y| {
            let mut row = String::new();
            let mut x = buffer.area.x;
            while x < buffer.area.right() {
                let symbol = buffer[(x, y)].symbol();
                row.push_str(symbol);
                // Skip continuation cells of wide glyphs, as setup_screen does.
                x = x.saturating_add(symbol.cell_width().max(1));
            }
            row.trim_end_matches(' ').to_owned()
        })
        .collect::<Vec<_>>();
    // One newline per terminal row, including leading and trailing blank rows.
    format!("{}\n", rows.join("\n"))
}

fn fixture_picker(screen: &str, picker: &ResourcePicker) {
    fixture_screens(screen, |frame, theme| {
        let area = frame.area();
        draw_resource_picker(frame, area, picker, theme);
    });
}

fn resume_picker(sessions: Vec<SessionListing>) -> ResourcePicker {
    let now = time::OffsetDateTime::from_unix_timestamp(1_750_000_000).expect("fixed clock");
    // This is session_resource_entries' shared body, with its current formatter
    // pinned to UTC and a fixed instant instead of consulting the wall clock.
    let entries = resources::session_resource_entries_with_updated(sessions, None, |updated| {
        resources::format_session_updated_at(updated, time::UtcOffset::UTC, now)
    });
    ResourcePicker::new(
        "Resume session",
        entries,
        "No sessions to resume in this project · esc exits",
    )
    .with_empty_guidance_keys()
}

fn resume_sessions() -> Vec<SessionListing> {
    [
        (
            "session-2042b4df-6465-42d4-be69-180feb01a926",
            "What does lib.rs define? One sentence.",
            1_749_999_880_000,
        ),
        (
            "session-5382b89c-874a-40de-b7c2-7b9a26a7d420",
            "Reply with the single word: alpha",
            1_749_999_760_000,
        ),
    ]
    .into_iter()
    .map(|(id, preview, updated)| SessionListing {
        id: SessionId::new(id),
        path: PathBuf::from(format!(
            "/tmp/smith-home/.smith/sessions/{id}/snapshot.json"
        )),
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        updated: Some(Timestamp(updated)),
        turn_count: Some(1),
        provider: Some("zai".into()),
        model: Some("glm-5.3".into()),
        user_preview: Some(preview.into()),
    })
    .collect()
}

fn first_run_setup() -> SetupApp {
    SetupApp::new(
        SetupMode::FirstRun,
        Vec::new(),
        Vec::new(),
        setup::glm_quick_start(),
        setup::setup_action_entries(&SetupMode::FirstRun),
        setup::setup_prompts(),
    )
    .with_destination(DESTINATION)
}

fn setup_key(app: &mut SetupApp, code: KeyCode) {
    assert!(matches!(
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE)),
        SetupEffect::None
    ));
}

fn glm_credentials() -> SetupApp {
    let mut app = first_run_setup();
    setup_key(&mut app, KeyCode::Enter);
    assert!(!app.is_choosing_action());
    app
}

#[test]
fn fixtures_standalone_resume_two_sessions() {
    fixture_picker("resume_two_sessions", &resume_picker(resume_sessions()));
}

#[test]
fn fixtures_standalone_resume_none() {
    fixture_picker("resume_none", &resume_picker(Vec::new()));
}

#[test]
fn fixtures_standalone_setup_first_step() {
    let app = first_run_setup();
    fixture_screens("setup_first_step", |frame, theme| {
        draw_setup(frame, &app, theme)
    });
}

#[test]
fn fixtures_standalone_setup_credential_methods() {
    let app = glm_credentials();
    fixture_screens("setup_credential_methods", |frame, theme| {
        draw_setup(frame, &app, theme);
    });
}

#[test]
fn fixtures_standalone_setup_key_field() {
    let mut app = glm_credentials();
    // Protected storage is initially selected. Type a fixed dummy key, but do
    // not submit it or perform any credential enrollment.
    setup_key(&mut app, KeyCode::Enter);
    for character in "sk-test".chars() {
        setup_key(&mut app, KeyCode::Char(character));
    }
    fixture_screens("setup_key_field", |frame, theme| {
        draw_setup(frame, &app, theme)
    });
}

#[test]
fn fixtures_standalone_setup_review() {
    let mut app = glm_credentials();
    setup_key(&mut app, KeyCode::Char('4'));
    for character in "ZAI_API_KEY".chars() {
        setup_key(&mut app, KeyCode::Char(character));
    }
    setup_key(&mut app, KeyCode::Enter);
    fixture_screens("setup_review", |frame, theme| {
        draw_setup(frame, &app, theme)
    });
}

#[test]
fn fixtures_standalone_chatgpt_login_method() {
    fixture_picker("chatgpt_login_method", &chatgpt::login_method_picker());
}

#[test]
fn fixtures_standalone_chatgpt_account_choice() {
    let picker = ResourcePicker::choices(
        "Connect ChatGPT · already connected",
        connection::connect_mode_entries(1),
        "No connection choices",
    );
    fixture_picker("chatgpt_account_choice", &picker);
}

#[test]
fn fixtures_standalone_chatgpt_login_progress() {
    let display = chatgpt::LoginDisplay {
        destination: "https://auth.openai.com/authorize?state=fixture-state".into(),
        user_code: None,
        browser_opened: true,
    };
    fixture_screens("chatgpt_login_progress", |frame, _theme| {
        chatgpt::draw_login_progress(frame, &display, 0, true);
    });
}

#[test]
fn fixtures_standalone_xai_login_progress() {
    let progress = crate::xai::login_progress(
        "ABCD-1234",
        "https://auth.x.ai/activate?user_code=ABCD-1234",
        true,
        true,
    );
    fixture_screens("xai_login_progress", |frame, theme| {
        progress.draw(frame, frame.area(), theme);
    });
}
