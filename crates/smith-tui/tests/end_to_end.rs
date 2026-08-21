//! End-to-end: a real runtime, a real session, the real client state.
//!
//! The unit tests feed [`App`] hand-built events, which proves the reducer but
//! not that Smith and the shared runtime agree on the event vocabulary. These
//! tests drive an actual `Runtime` over its deterministic fake provider and
//! assert on rendered output, so a change to the runtime's event stream that
//! Smith fails to handle shows up here rather than in a terminal.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime::prelude::*;
use agent_runtime::runtime::RuntimeEventStream;
use agent_runtime::runtime::{RuntimeBuilder, StartSession};
use agent_runtime_core::approval::AllowAll;
use agent_runtime_testkit::scenarios::fake_model_profile;
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use smith_runtime::client::SmithEvent;
use smith_tui::app::{Action, App, SubmissionTarget};
use smith_tui::theme::Theme;

/// Drains every event the session has emitted so far into the client.
///
/// The stream is a live broadcast rather than a queue, so draining is bounded
/// by a short idle timeout: once nothing new arrives, the turn's events have
/// all been applied.
async fn drain(events: &mut RuntimeEventStream, app: &mut App) {
    while let Ok(Some(envelope)) =
        tokio::time::timeout(Duration::from_millis(100), events.next()).await
    {
        app.apply(&SmithEvent::project_or_unknown(&envelope));
    }
}

/// Renders the app to a plain-text screen.
fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    terminal
        .draw(|frame| smith_tui::draw(frame, app, Theme::new().without_color()))
        .expect("a frame");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Builds a runtime the way `smith-cli` does, over the fake provider.
///
/// The model profile comes from the shared testkit rather than a local
/// literal, so Smith's fixtures cannot drift from the limits, context policy,
/// or revision identity that the upstream Smith conformance fixture plans
/// against.
fn runtime(reply: &str) -> Runtime {
    RuntimeBuilder::new(ModelId::new("fake"))
        .model_profile(fake_model_profile())
        .provider(Arc::new(FakeProvider::text_reply(reply)))
        .system_prompt("You are Smith, a terminal coding assistant.")
        .approval(Arc::new(AllowAll))
        .build()
        .expect("a runtime")
}

#[tokio::test]
async fn a_typed_message_reaches_the_model_and_its_reply_reaches_the_screen() {
    let runtime = runtime("The retry policy classifies provider failures.");
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let mut events = session.subscribe();
    let mut app = App::new("fake", "~/work/api");

    // The user types and presses Enter; the host loop forwards the action.
    for ch in "explain the retry policy".chars() {
        app.on_key(key(ch));
    }
    let action = app.on_key(enter());
    let Some(Action::Submit {
        submission,
        target: SubmissionTarget::WholeTurn,
    }) = action
    else {
        panic!("expected a prepared whole-turn action, got {action:?}");
    };
    let handle = session
        .send(submission.input_without_files())
        .expect("the turn is accepted");
    app.whole_turn_dispatched(handle.id().clone(), &submission);
    handle.completed().await;

    // Drain everything the turn produced into the client.
    drain(&mut events, &mut app).await;

    let rendered = screen(&app, 74, 16);
    assert!(
        rendered.contains("› explain the retry policy"),
        "the user's message is missing:\n{rendered}"
    );
    assert!(
        rendered.contains("The retry policy classifies provider failures."),
        "the model's reply is missing:\n{rendered}"
    );
}

#[tokio::test]
async fn usage_reported_by_the_runtime_reaches_the_header() {
    let runtime = runtime("done");
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let mut events = session.subscribe();
    let mut app = App::new("fake", "~/work/api");

    assert_eq!(
        app.status.context.render(),
        "?",
        "before any turn, context is unknown rather than zero"
    );

    session
        .run(UserInput::text("hi"))
        .await
        .expect("the turn runs");
    drain(&mut events, &mut app).await;

    // Whatever the fake reports, the client must not still be claiming
    // ignorance after a completed turn.
    assert!(
        app.status.has_reported_usage(),
        "a completed turn must leave provider-reported usage behind"
    );
    assert_ne!(app.status.context.render(), "?");
}

#[tokio::test]
async fn the_client_returns_to_idle_after_a_turn_completes() {
    let runtime = runtime("done");
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let mut events = session.subscribe();
    let mut app = App::new("fake", "~/work/api");

    session
        .run(UserInput::text("hi"))
        .await
        .expect("the turn runs");
    drain(&mut events, &mut app).await;

    assert!(!app.is_busy(), "the client is stuck in a working state");
    assert!(
        !app.has_live_work(),
        "quitting now would warn about nothing"
    );
}

#[tokio::test]
async fn shutdown_marks_the_session_ended() {
    let runtime = runtime("done");
    let session = runtime
        .start_session(StartSession::new())
        .await
        .expect("a session");
    let mut events = session.subscribe();
    let mut app = App::new("fake", "~/work/api");

    session.shutdown().await.expect("a clean shutdown");
    drain(&mut events, &mut app).await;

    assert_eq!(app.status.activity, smith_tui::status::Activity::Ended);
}

fn key(ch: char) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char(ch),
        crossterm::event::KeyModifiers::NONE,
    )
}

fn enter() -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    )
}
