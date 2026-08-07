// modal behavior tests.

    #[test]
    fn a_completed_tool_row_shows_its_bounded_result_preview() {
        // `search`, not `registry.search`: the latter is in the reviewed
        // suppression set once it succeeds (see the suppression tests in
        // `transcript.rs`), so it cannot exercise "a completed row shows its
        // preview" on its own — this needs a tool the suppression set never
        // touches.
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("call-search"),
            name: "search".to_owned(),
            argument_keys: vec!["pattern".to_owned()],
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("arguments"),
            arguments: None,
        }));
        app.set_tool_display(
            "call-search",
            smith_tools::project_tool_call_display(
                "search",
                &serde_json::json!({"pattern": "browser automation", "path": "src"}),
            )
            .expect("reviewed search projection"),
        );
        app.set_tool_result_preview("call-search", "card one\ncard two");

        let running = render(&app, 74, 14, Theme::new());
        assert!(
            running.contains("Search(\"browser automation\" · src)"),
            "{running}"
        );
        assert!(
            !running.contains("card one"),
            "a running row must not show result lines yet: {running}"
        );

        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("call-search"),
            name: "search".to_owned(),
            is_error: false,
        }));
        let completed = render(&app, 74, 14, Theme::new());
        assert!(completed.contains(" · ok"), "{completed}");
        assert!(completed.contains("    card one"), "{completed}");
        assert!(completed.contains("    card two"), "{completed}");
    }

    #[test]
    fn reverse_history_search_is_anchored_labelled_and_bounded_when_narrow() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.composer.replace("fix history\nsecond line");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.composer.replace("scratch draft");
        app.on_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        for character in "HISTORY".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        for (width, height) in [(MIN_WIDTH, MIN_HEIGHT), (74, 16), (120, 24)] {
            let screen = render(&app, width, height, Theme::new().without_color());
            insta_like(
                &screen,
                &[
                    "reverse search",
                    "HISTORY",
                    "fix history",
                    "ctrl+r older",
                    "enter use",
                    "esc cancel",
                ],
            );
            assert!(
                screen.contains("scratch draft"),
                "{width}×{height}:\n{screen}"
            );
            assert!(
                screen
                    .lines()
                    .all(|line| line.width() <= usize::from(width)),
                "{width}×{height} overflowed:\n{screen}"
            );
            assert!(
                screen.lines().count() <= usize::from(height),
                "{width}×{height} overflowed vertically:\n{screen}"
            );
        }
    }

    #[test]
    fn runtime_resource_picker_is_a_bounded_pane_above_the_composer() {
        let mut app = conversation();
        let history_len = app.transcript.blocks().len();
        let entries = (0..12)
            .map(|index| {
                crate::picker::ResourceEntry::new(
                    format!("provider/model-{index:02}"),
                    format!("provider/model-{index:02}"),
                    "trusted limits",
                )
                .active(index == 0)
            })
            .collect();
        app.overlay = Some(Overlay::ResourcePicker {
            picker: crate::picker::ResourcePicker::new("Choose model", entries, "run setup"),
            target: crate::app::ResourceTarget::Model,
            restore_on_escape: "/model".into(),
        });
        let rendered = render(&app, 64, 18, Theme::from_env().without_color());
        assert!(rendered.contains("Choose model"), "{rendered}");
        assert!(rendered.contains("provider/model-00"), "{rendered}");
        assert!(rendered.contains("current"), "{rendered}");
        assert!(rendered.contains("1/12"), "{rendered}");
        assert!(
            rendered.contains("The retry policy classifies failures."),
            "{rendered}"
        );
        assert!(
            !rendered.contains("provider/model-05"),
            "the pane expanded past five results:\n{rendered}"
        );
        assert!(
            !rendered.contains('╭'),
            "runtime choices should not draw a modal border:\n{rendered}"
        );
        let lines = rendered.lines().collect::<Vec<_>>();
        let picker_y = lines
            .iter()
            .position(|line| line.contains("Choose model"))
            .expect("picker row");
        let composer_y = lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("composer row");
        assert!(picker_y < composer_y, "{rendered}");
        assert!(
            composer_y - picker_y <= 7,
            "pane grew too tall:\n{rendered}"
        );
        assert_eq!(
            app.transcript.blocks().len(),
            history_len,
            "picker metadata entered canonical history"
        );
    }

    #[test]
    fn reference_picker_uses_plain_rows_and_visible_composer_mentions_without_color() {
        let mut app = App::new("glm-5.2", "/Volumes/Data/codes/ai/agent-runtime:main");
        app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
        app.status.set_agent("build");
        app.set_resources(crate::app::RuntimeResources {
            files: vec![crate::picker::ResourceEntry::new(
                "file:src/lib.rs",
                "src/lib.rs",
                "file · 42 bytes",
            )],
            child_agents: vec![crate::picker::ResourceEntry::new(
                "agent:review",
                "review",
                "child profile · review · zai/glm-5.2",
            )],
            ..crate::app::RuntimeResources::default()
        });
        let before = render(&app, 120, 24, Theme::new().without_color().without_motion());
        let before_lines = before.lines().collect::<Vec<_>>();
        let before_identity = before_lines
            .iter()
            .position(|line| line.contains("build · zai/glm-5.2"))
            .expect("idle identity row");
        let before_composer = before_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("idle composer row");

        app.on_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));

        let screen = render(&app, 120, 24, Theme::new().without_color().without_motion());
        insta_like(
            &screen,
            &[
                "Attach file or invoke agent",
                "review",
                "child profile · review",
                "src/lib.rs",
                "file · 42 bytes",
                "build · zai/glm-5.2 · /Volumes/Data/codes/ai/agent-runtime:main · ? ctx",
                "type to filter · ↑↓ choose · enter confirm · esc cancel",
            ],
        );
        assert!(!screen.contains("@review"), "{screen}");
        assert!(!screen.contains("@src/lib.rs"), "{screen}");
        let open_lines = screen.lines().collect::<Vec<_>>();
        let open_identity = open_lines
            .iter()
            .position(|line| line.contains("build · zai/glm-5.2"))
            .expect("picker identity row");
        let open_composer = open_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("picker composer row");
        assert_eq!(
            open_identity.saturating_add(1),
            before_identity,
            "picker controls should reserve exactly one temporary footer row:\n{screen}"
        );
        assert_eq!(
            open_composer.saturating_add(1),
            before_composer,
            "picker controls should move the composer by exactly one row:\n{screen}"
        );

        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
        assert_eq!(app.composer.text(), "@src/lib.rs ");
    }

    #[test]
    fn compact_picker_replaces_todo_pane_with_one_temporary_control_row() {
        let mut app = App::new("glm-5.2", "api:main");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::PlanUpdated {
            revision: 1,
            sensitivity: PlanSensitivity::Public,
            counts: std::collections::BTreeMap::from([
                ("cancelled".to_owned(), 0),
                ("completed".to_owned(), 1),
                ("in_progress".to_owned(), 1),
                ("pending".to_owned(), 1),
            ]),
            items: Some(vec![
                PlanItemProjection {
                    id: "inspect".to_owned(),
                    text: "Inspect relevant code".to_owned(),
                    status: PlanItemStatus::Completed,
                    reason: None,
                },
                PlanItemProjection {
                    id: "change".to_owned(),
                    text: "Implement the change".to_owned(),
                    status: PlanItemStatus::InProgress,
                    reason: None,
                },
                PlanItemProjection {
                    id: "verify".to_owned(),
                    text: "Run focused tests".to_owned(),
                    status: PlanItemStatus::Pending,
                    reason: None,
                },
            ]),
        }));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));
        app.set_resources(crate::app::RuntimeResources {
            files: vec![crate::picker::ResourceEntry::new(
                "file:src/lib.rs",
                "src/lib.rs",
                "file · 42 bytes",
            )],
            ..crate::app::RuntimeResources::default()
        });

        let theme = Theme::new().without_color().without_motion();
        let before = render(&app, 80, 14, theme);
        insta_like(
            &before,
            &[
                "Todo",
                "[x] Inspect relevant code",
                "[>] Implement the change",
                "[ ] Run focused tests",
            ],
        );
        assert!(!before.contains("work ·"), "{before}");
        assert!(!before.contains("plan 0 active"), "{before}");
        let before_lines = before.lines().collect::<Vec<_>>();
        let before_composer = before_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("composer row");

        app.on_key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));
        let open = render(&app, 80, 14, theme);
        let open_lines = open.lines().collect::<Vec<_>>();
        let picker = open_lines
            .iter()
            .position(|line| line.contains("Attach file or invoke agent"))
            .expect("picker row");
        let open_composer = open_lines
            .iter()
            .position(|line| line.contains("Ask Smith to do anything"))
            .expect("open composer row");
        assert!(picker < open_composer, "{open}");
        assert!(!open.contains("Todo"), "{open}");
        assert!(!open.contains("Inspect relevant code"), "{open}");
        assert!(!open.contains("Implement the change"), "{open}");
        assert!(!open.contains("Run focused tests"), "{open}");
        assert_eq!(
            open_composer.saturating_add(1),
            before_composer,
            "picker controls should move the composer by exactly one row:\n{open}"
        );

        app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let closed = render(&app, 80, 14, theme);
        insta_like(
            &closed,
            &[
                "Todo",
                "[x] Inspect relevant code",
                "[>] Implement the change",
                "[ ] Run focused tests",
            ],
        );
    }

    #[test]
    fn recovery_and_review_modals_name_the_action_without_a_default() {
        let mut undo = App::new("gpt-5.3", "~/work/api");
        undo.confirm_undo("--- current\n+++ restore\n-old\n+new");
        let undo_screen = render(&undo, 74, 20, Theme::new().without_color());
        assert!(undo_screen.contains("No action is selected by default"));
        assert!(undo_screen.contains("apply undo"));

        let mut review = App::new("gpt-5.3", "~/work/api");
        review.confirm_review(
            "all",
            "provider-backed: yes\nworkspace authority: read-only",
        );
        let review_screen = render(&review, 74, 20, Theme::new().without_color());
        assert!(review_screen.contains("read-only review"));
        assert!(review_screen.contains("provider-backed: yes"));
    }

    #[test]
    fn child_follow_up_and_resume_confirmations_are_clear_without_color_at_supported_sizes() {
        for (width, height) in [(44, 16), (74, 20), (120, 28)] {
            let mut follow_up = App::new("glm-5.2", "~/work/api");
            follow_up.overlay = Some(Overlay::AgentFollowUpConfirm {
                child_id: "child-1".to_owned(),
                task: "check the parser".to_owned(),
                content: "child: child-1\noperation: new follow-up turn\ncontinuity: reuse prior child history\nprovider spend: yes".to_owned(),
            });
            let follow_up_screen = render(&follow_up, width, height, Theme::new().without_color());
            assert!(
                follow_up_screen.contains("existing child follow-up"),
                "{width}×{height}:\n{follow_up_screen}"
            );
            assert!(
                follow_up_screen.contains("new follow-up"),
                "{width}×{height}:\n{follow_up_screen}"
            );

            let mut resume = App::new("glm-5.2", "~/work/api");
            resume.overlay = Some(Overlay::AgentResumeConfirm {
                child_id: "child-1".to_owned(),
                content: "child: child-1\noperation: continue exact interrupted checkpoint\nturn slot consumed: no\nside effects: committed work is not replayed".to_owned(),
            });
            let resume_screen = render(&resume, width, height, Theme::new().without_color());
            assert!(
                resume_screen.contains("resume interrupted child"),
                "{width}×{height}:\n{resume_screen}"
            );
            assert!(
                resume_screen.contains("exact interrupted"),
                "{width}×{height}:\n{resume_screen}"
            );
        }
    }

    #[tokio::test]
    async fn the_approval_panel_names_the_tool_and_its_keys() {
        let mut app = conversation();
        app.present_approval(prompt("shell", serde_json::json!({"command": "rm -rf build"})).await);
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(
            &screen,
            &[
                "approval required",
                "shell",
                "rm -rf build",
                "process execution",
                "y allow once",
                "allow this target",
            ],
        );
    }

    #[test]
    fn approval_warnings_follow_typed_authority_not_only_scheduler_effects() {
        let prepared = PreparedToolCall::new(
            ToolCallId::new("sensitive-call"),
            "broker",
            serde_json::json!({"reference": "provider"}),
            [
                Permission::CredentialUse,
                Permission::DataEgress,
                Permission::FsDelete,
            ]
            .into_iter()
            .collect::<PermissionSet>(),
            SecurityResource::credential("provider"),
            ToolEffects::new(Vec::new()),
            ToolCallDisplay::new("Use a protected broker"),
        );

        let warning = authority_warning(&prepared).expect("sensitive authority warning");
        assert!(warning.contains("credential use"), "{warning}");
        assert!(warning.contains("data egress"), "{warning}");
        assert!(warning.contains("file deletion"), "{warning}");
    }

    #[tokio::test]
    async fn an_edit_approval_shows_a_diff_instead_of_raw_json() {
        let app = edit_approval(
            "fn retry() {\n    once();\n}\n",
            "fn retry(limit: u32) {\n    once();\n}\n",
        )
        .await;
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(
            &screen,
            &[
                "approval required",
                "src/retry.rs",
                "1 removed · 1 added",
                "- fn retry() {",
                "+ fn retry(limit: u32) {",
                "    once();",
                "y allow once",
            ],
        );
        assert!(
            !screen.contains("old_string"),
            "the raw arguments must give way to the diff:\n{screen}"
        );
    }

    #[tokio::test]
    async fn a_non_edit_approval_falls_back_to_its_arguments() {
        let mut app = conversation();
        app.present_approval(
            prompt(
                "shell",
                serde_json::json!({"command": "rm -rf build", "cwd": "/repo"}),
            )
            .await,
        );
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(&screen, &["\"command\"", "rm -rf build"]);
        assert!(
            !screen.contains("change  "),
            "a shell call has no diff to summarize:\n{screen}"
        );
    }

    #[tokio::test]
    async fn a_diff_too_tall_for_the_panel_says_how_much_it_hid() {
        let old: String = (0..60).map(|n| format!("let x{n} = {n};\n")).collect();
        let new = old.replace("let x", "let y");
        let app = edit_approval(&old, &new).await;
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(&screen, &["more lines not shown", "y allow once"]);
    }

    #[tokio::test]
    async fn a_change_buried_in_context_still_reaches_the_top_of_the_modal() {
        let old: String = (0..20).map(|n| format!("let x{n} = {n};\n")).collect();
        let new = old.replace("let x10 = 10;", "let x10 = 11;");
        let app = edit_approval(&old, &new).await;
        let screen = render(&app, 74, 40, Theme::new());

        // The collapsed context is counted, not silently dropped.
        insta_like(
            &screen,
            &[
                "unchanged lines",
                "- let x10 = 10;",
                "+ let x10 = 11;",
                "y allow once",
            ],
        );
    }

    #[tokio::test]
    async fn a_short_terminal_still_renders_an_answerable_approval() {
        let app = edit_approval("once();\n", "twice();\n").await;
        for (width, height) in [(MIN_WIDTH, MIN_HEIGHT), (44, 12), (52, 14)] {
            let screen = render(&app, width, height, Theme::new());
            insta_like(&screen, &["approval required", "src/retry.rs"]);
            assert!(
                screen.contains("allow") && screen.contains("deny"),
                "{width}×{height} left the approval unanswerable:\n{screen}"
            );
            for line in screen.lines() {
                assert!(
                    line.width() <= usize::from(width),
                    "{width}×{height} overflowed the viewport:\n{screen}"
                );
            }
            assert!(
                screen.lines().count() <= usize::from(height),
                "{width}×{height} overflowed the viewport:\n{screen}"
            );
        }
    }

    #[test]
    fn questionnaire_is_answerable_when_narrow_and_masks_sensitive_drafts() {
        let mut app = conversation();
        let form = QuestionnaireForm::new(
            "interaction-1",
            vec![
                QuestionnaireQuestion::new(
                    "token",
                    "Credential",
                    "Which secret token should be used?",
                    vec![QuestionnaireChoice::new("configured", "Configured token")],
                )
                .with_free_form(true),
            ],
            Deadline::never(),
        )
        .expect("valid questionnaire")
        .restored(true);
        app.present_questionnaire(form);
        for character in "supersecret".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        let normal = render(&app, 74, 24, Theme::new().without_color());
        insta_like(
            &normal,
            &[
                "answer required",
                "Which secret token should be used?",
                "restored pending question",
                "(masked)",
                "[Submit]",
                "esc cancel",
            ],
        );
        assert!(!normal.contains("supersecret"), "{normal}");

        let narrow = render(&app, MIN_WIDTH, MIN_HEIGHT, Theme::new().without_color());
        insta_like(
            &narrow,
            &[
                "answer required",
                "Which secret token should be used?",
                "Submit",
                "cancel",
            ],
        );
        assert!(!narrow.contains("supersecret"), "{narrow}");
        assert!(
            narrow
                .lines()
                .all(|line| line.width() <= usize::from(MIN_WIDTH)),
            "{narrow}"
        );
    }

    #[test]
    fn exit_confirmation_names_a_running_background_task_by_id() {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.set_running_tasks(vec![crate::app::RunningTaskSummary {
            task_id: "task:7".to_owned(),
            command_hint: "cargo build".to_owned(),
        }]);
        app.composer.replace("/quit");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let screen = render(&app, 74, 20, Theme::new().without_color());
        assert!(screen.contains("task:7"), "{screen}");
    }
