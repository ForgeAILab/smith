// file command submission behavior tests.

fn skill_app() -> App {
    let mut app = app();
    app.set_command_catalog(
        smith_client::file_commands::CommandCatalog::empty().with_skills(vec![
            smith_client::skill_commands::SkillCommand {
                name: "rust-review".into(),
                alias: "rust-review".into(),
                description: "Review changed code".into(),
                source: "skill · user".into(),
                disabled_reason: None,
            },
        ]),
    );
    app
}

#[test]
fn skill_shortcuts_complete_without_activation_then_route_exact_task_text() {
    let mut app = skill_app();
    for ch in "/rust-r".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .is_none()
    );
    assert_eq!(app.composer.text(), "/rust-review ");
    assert!(app.transcript.is_empty());
    app.composer.replace("/rust-review inspect  src/lib.rs");
    assert!(
        matches!(app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Some(Action::UseSkill { typed, name, arguments })
        if typed == "/rust-review inspect  src/lib.rs" && name == "rust-review" && arguments == "inspect  src/lib.rs")
    );
    app.composer.replace("/skills use rust-review");
    assert!(
        matches!(app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Some(Action::UseSkill { name, arguments, .. }) if name == "rust-review" && arguments.is_empty())
    );
}

#[test]
fn skill_submissions_preserve_authored_instructions_and_parse_only_task_references() {
    let mut app = app();
    app.resources.files.push(ResourceEntry::new(
        "file:src/lib.rs",
        "src/lib.rs",
        "workspace file",
    ));
    let body = "Read @file:guide.md; keep @@literal.\n";
    for arguments in ["", " \t\n", "Inspect @file:src/lib.rs and @@literal"] {
        let submission = app
            .prepare_skill_submission("/inspect".into(), body, arguments)
            .unwrap();
        let expected = if arguments.trim().is_empty() {
            body.to_owned()
        } else {
            format!("{body}\n\nInspect @file:src/lib.rs and @literal")
        };
        assert_eq!(submission.display_text(), "/inspect");
        assert_eq!(submission.committed_text(), expected);
        assert_eq!(
            submission.input_without_files(),
            agent_runtime_core::content::UserInput::text(expected)
        );
        if arguments.trim().is_empty() {
            assert!(submission.files().is_empty());
        } else {
            assert_eq!(submission.files(), &["src/lib.rs"]);
        }
    }
    assert!(
        app.prepare_skill_submission("/inspect".into(), body, "@file:missing.rs")
            .is_err()
    );
}

#[test]
fn unavailable_skill_shortcuts_preserve_the_draft() {
    let mut app = skill_app();
    let mut skills = app.command_catalog.skills().to_vec();
    skills[0].disabled_reason = Some("awaiting trust".into());
    app.set_command_catalog(app.command_catalog.clone().with_skills(skills));
    app.composer.replace("/rust-review");
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .is_none()
    );
    assert_eq!(app.composer.text(), "/rust-review");
    assert!(!app.transcript.is_empty());
}

#[test]
fn skill_completion_escape_returns_to_the_composer_without_activation() {
    let mut app = skill_app();
    for ch in "/rust-r".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    assert!(app.overlay.is_some());
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .is_none()
    );
    assert!(app.overlay.is_none());
    assert!(app.transcript.is_empty());
}

#[test]
fn busy_skill_shortcuts_leave_the_running_turn_and_draft_untouched() {
    let mut app = skill_app();
    app.whole_turn_dispatched(
        TurnId::new("active"),
        &PreparedSubmission::from_file_command("work".into(), "work".into()),
    );
    app.composer.replace("/rust-review");
    assert!(
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .is_none()
    );
    assert_eq!(app.composer.text(), "/rust-review");
    assert!(app.is_busy());
}

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
