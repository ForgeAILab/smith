//! Baseline-compatible timing of public transcript drawing while text streams.

use smith_client::NoticeKind;
use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use smith_client::message_report::MessageReport;
use smith_tui::{App, LocalResult, Theme, ToolStatus};

// Keep this harness on the public APIs available before the rendering refactor
// so the identical file can measure the baseline commit.
#[test]
#[ignore]
fn streaming_transcript_mean_frame_time() {
    let mut app = App::new("model", "project");
    for index in 0..4_999 {
        match index % 7 {
            0 => app.transcript.push_user(format!(
                "Inspect module {index} and explain its retry policy."
            )),
            1 => {
                app.transcript.push_text_delta("## Retry policy\n\nUse **bounded retries** and `cancel` between attempts.\n\n```rust\nretry().await?;\n```\n");
                app.transcript.close_open();
            }
            2 => {
                let call = format!("call-{index}");
                app.transcript.push_tool_call(&call, "shell", None, &[]);
                app.transcript.complete_tool_call(&call, ToolStatus::Ok);
                app.transcript
                    .set_tool_result_preview(&call, "one\ntwo\nthree\nfour\nfive\nsix");
            }
            3 => app
                .transcript
                .push_notice(NoticeKind::Monitor, format!("Finished check {index}")),
            4 => app
                .transcript
                .push_error("A bounded check failed; retry is available."),
            5 => app
                .transcript
                .push_local(LocalResult::Message(Box::new(MessageReport::Notice {
                    title: "status".to_owned(),
                    message: format!("Local result {index}\nNo provider request was needed."),
                }))),
            _ => {
                let echo = app
                    .transcript
                    .push_shell_shortcut("cargo check -p smith-tui");
                app.transcript
                    .finish_shell_shortcut(echo, None, false, "Finished dev profile");
            }
        }
    }
    app.transcript
        .push_text_delta("The streaming answer grows here.");
    assert_eq!(app.transcript.len(), 5_000);
    let theme = Theme::new().without_color().without_motion();
    let mut terminal = Terminal::new(TestBackend::new(100, 32)).expect("test terminal");
    terminal
        .draw(|frame| smith_tui::render::draw(frame, &app, theme))
        .expect("warm frame");
    const FRAMES: u32 = 100;
    let started = Instant::now();
    for _ in 0..FRAMES {
        app.transcript
            .push_text_delta(" Another streamed text delta.");
        terminal
            .draw(|frame| smith_tui::render::draw(frame, &app, theme))
            .expect("frame");
    }
    eprintln!(
        "5,000 mixed blocks, {FRAMES} streaming frames at 100x32: {:.3} ms mean frame time",
        started.elapsed().as_secs_f64() * 1_000.0 / f64::from(FRAMES)
    );
}
