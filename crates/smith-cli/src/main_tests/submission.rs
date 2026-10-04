use super::*;

// submission behavior tests.

#[test]
fn tool_display_enrichment_runs_at_request_and_completion_boundaries() {
    let call = agent_runtime_core::ids::ToolCallId::new("call-display");
    let requested = RuntimeEvent::ToolCallRequested {
        call: call.clone(),
        name: "read".to_owned(),
        argument_keys: vec!["path".to_owned()],
        argument_fingerprint: serde_json::from_value(serde_json::json!(
            "0123456789abcdef0123456789abcdef"
        ))
        .expect("fingerprint"),
        arguments: None,
    };
    let completed = RuntimeEvent::ToolCallCompleted {
        call: call.clone(),
        name: "read".to_owned(),
        is_error: false,
    };

    assert_eq!(tool_call_for_display(&requested), Some(call.clone()));
    assert_eq!(tool_call_for_display(&completed), Some(call));
    assert!(tool_call_for_display(&RuntimeEvent::TurnStarted).is_none());
}

#[tokio::test]
async fn prepared_file_attachment_reads_exactly_without_provider_spend() {
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join("src")).expect("source directory");
    std::fs::write(
        project.path().join("src/lib.rs"),
        "pub fn answer() -> u8 { 1 }\n",
    )
    .expect("source file");
    let mut app = App::new("example-model", project.path().display().to_string());
    app.set_resources(RuntimeResources {
        files: vec![ResourceEntry::new(
            "file:src/lib.rs",
            "src/lib.rs",
            "workspace file",
        )],
        ..RuntimeResources::default()
    });
    app.composer.replace("explain @src/lib.rs");
    let Some(Action::Submit { submission, .. }) =
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a prepared attachment submission");
    };

    // The identity is frozen at keypress; bytes are read at dispatch.
    std::fs::write(
        project.path().join("src/lib.rs"),
        "pub fn answer() -> u8 { 42 }\n",
    )
    .expect("updated source file");
    let input = materialize_prepared_submission(project.path(), &submission)
        .await
        .expect("prepared attachment");
    assert_eq!(input.parts.len(), 2);
    let wire = serde_json::to_string(&input).expect("input wire");
    assert!(wire.contains("source=\\\"prepared_read\\\""), "{wire}");
    assert!(wire.contains("42"), "{wire}");
    assert!(!wire.contains("{ 1 }"), "{wire}");

    let outside = tempfile::NamedTempFile::new().expect("outside file");
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), project.path().join("src/outside-link"))
        .expect("outside symlink");
    app.set_resources(RuntimeResources {
        files: vec![ResourceEntry::new(
            "file:src/outside-link",
            "src/outside-link",
            "workspace entry",
        )],
        ..RuntimeResources::default()
    });
    app.composer.replace("do not send @src/outside-link");
    let Some(Action::Submit { submission, .. }) =
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
    else {
        panic!("expected a prepared symlink submission");
    };
    let error = materialize_prepared_submission(project.path(), &submission)
        .await
        .expect_err("workspace escape must fail locally");
    assert!(error.contains("was not sent"), "{error}");
}

#[test]
fn clipboard_no_ops_are_hint_feedback_and_a_successful_keypress_clears_it() {
    let mut app = App::new("model", "project");
    for (content, expected) in [
        (Ok(ClipboardContent::Empty), "nothing to attach"),
        (
            Err("clipboard unavailable".to_owned()),
            "clipboard unavailable",
        ),
    ] {
        apply_clipboard_content(&mut app, content);
        assert_eq!(app.feedback_notice().unwrap().text, expected);
        assert!(app.transcript.is_empty());
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20))
            .expect("test terminal");
        terminal
            .draw(|frame| smith_tui::render::draw(frame, &app, smith_tui::Theme::new()))
            .expect("frame");
        let buffer = terminal.backend().buffer();
        let hint = (0..100)
            .map(|x| buffer[(x, 19)].symbol())
            .collect::<String>();
        assert!(hint.contains(expected), "{hint}");
    }
    apply_clipboard_content(
        &mut app,
        Ok(ClipboardContent::Text("pasted draft".to_owned())),
    );
    assert!(app.feedback_notice().is_none());
    assert_eq!(app.composer.text(), "pasted draft");
    assert!(app.transcript.is_empty());

    app.open_overlay(smith_tui::Overlay::Shortcuts);
    apply_clipboard_content(
        &mut app,
        Ok(ClipboardContent::Image {
            data_uri: "data:image/png;base64,unused".to_owned(),
            width: 1,
            height: 1,
        }),
    );
    assert_eq!(
        app.feedback_notice().unwrap().text,
        "close the current panel before attaching an image"
    );
    assert_eq!(app.composer.text(), "pasted draft");
    assert!(matches!(app.overlay, Some(smith_tui::Overlay::Shortcuts)));
    assert!(app.transcript.is_empty());
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.feedback_notice().is_none());
}
