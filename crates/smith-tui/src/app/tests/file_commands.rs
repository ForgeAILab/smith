// file command submission behavior tests.

#[test]
fn file_command_submission_displays_the_invocation_and_commits_the_expansion() {
    let mut app = app();
    let submission = PreparedSubmission::from_file_command(
        "/audit src/lib.rs".to_owned(),
        "Audit src/lib.rs for bugs.".to_owned(),
    );
    assert_eq!(submission.display_text(), "/audit src/lib.rs");
    assert_eq!(submission.committed_text(), "Audit src/lib.rs for bugs.");
    assert_eq!(
        submission.input_without_files(),
        agent_runtime_core::content::UserInput::text("Audit src/lib.rs for bugs.")
    );
    app.whole_turn_dispatched(TurnId::new("turn-1"), &submission);
    assert_eq!(
        app.transcript.blocks()[0],
        Block::User {
            text: "Audit src/lib.rs for bugs.".to_owned()
        }
    );
}

#[test]
fn file_command_expansion_reuses_reference_validation_and_typed_queue_preview() {
    let mut app = app();
    app.resources.files.push(ResourceEntry::new(
        "file:src/lib.rs",
        "src/lib.rs",
        "workspace file",
    ));
    let submission = app
        .prepare_file_command_submission(
            "/audit src/lib.rs".to_owned(),
            "Inspect @file:src/lib.rs and @@literal",
        )
        .expect("prepared reference");
    assert_eq!(submission.files(), &["src/lib.rs"]);
    assert_eq!(
        submission.committed_text(),
        "Inspect @file:src/lib.rs and @literal"
    );
    app.queue_prepared(submission);
    assert_eq!(
        app.pending_input_previews()[0].entries,
        ["/audit src/lib.rs"]
    );
    let error = app
        .prepare_file_command_submission("/audit".to_owned(), "Inspect @file:missing.rs")
        .expect_err("unknown identity is refused before dispatch");
    assert!(error.contains("missing.rs"), "{error}");
}
