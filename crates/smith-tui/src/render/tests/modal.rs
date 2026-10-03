// modal behavior tests.

    #[test]
    fn advisor_row_shows_the_label_and_advice_preview() {
        let mut app = App::new("main-model", "~/work/api");
        app.apply(&event(RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("advisor-call"),
            name: "advisor".to_owned(),
            argument_keys: Vec::new(),
            argument_fingerprint: agent_runtime_registry::Fingerprint::of("{}"),
            arguments: Some(serde_json::json!({})),
        }));
        app.set_tool_result_preview("advisor-call", "Check the cancellation path.\nRun the focused test.");
        app.apply(&event(RuntimeEvent::ToolCallCompleted {
            call: ToolCallId::new("advisor-call"),
            name: "advisor".to_owned(),
            is_error: false,
        }));
        let rendered = render(&app, 74, 14, Theme::new());
        assert!(rendered.contains("● Advisor()"), "{rendered}");
        assert!(rendered.contains("Check the cancellation path."), "{rendered}");
        assert!(rendered.contains("Run the focused test."), "{rendered}");
    }

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
        assert!(!completed.contains("running") && !completed.contains(" · ok"), "{completed}");
        assert!(completed.contains("  ⎿  card one"), "{completed}");
        assert!(completed.contains("     card two"), "{completed}");
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
        let working_y = lines
            .iter()
            .position(|line| line.contains("Working… ("))
            .expect("working row");
        assert!(picker_y < composer_y, "{rendered}");
        assert!(picker_y < working_y && working_y < composer_y, "{rendered}");
        // Include the working row in the pane-to-composer span.
        assert!(
            composer_y - picker_y <= 8,
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
                "agent",
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
        assert_eq!(app.composer.text(), "@review ");
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
        undo.confirm_undo(recovery_preview("--- current\n+++ restore\n-old\n+new"));
        let undo_screen = render(&undo, 74, 20, Theme::new().without_color());
        assert!(undo_screen.contains("No action is selected by default"));
        assert!(undo_screen.contains("apply undo"));

        let mut review = App::new("gpt-5.3", "~/work/api");
        review.confirm_review(smith_client::review_report::ReviewPreview {
            scope: "all".to_owned(),
            title: "diff · all uncommitted".to_owned(),
            patch: Vec::new(),
        });
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
                "Bash command",
                "rm -rf build",
                "process execution",
                "y  Yes",
                "within this target",
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

    #[test]
    fn external_service_authority_is_not_presented_as_project_filesystem_access() {
        let prepared = PreparedToolCall::new(
            ToolCallId::new("remote-call"),
            "mcp__github__search",
            serde_json::json!({"query": "smith"}),
            [
                Permission::ExternalRead,
                Permission::ExternalWrite,
                Permission::NetHttp,
                Permission::DataEgress,
            ]
            .into_iter()
            .collect::<PermissionSet>(),
            SecurityResource::other(
                "external-service",
                "mcp:github@revision#https://api.github.test/mcp",
            ),
            ToolEffects::new(Vec::new()),
            ToolCallDisplay::new("Search GitHub"),
        );

        assert_eq!(
            security_resource_text(prepared.resource()),
            "external service mcp:github@revision#https://api.github.test/mcp"
        );
        let warning = authority_warning(&prepared).expect("external authority warning");
        assert!(warning.contains("external service read"), "{warning}");
        assert!(warning.contains("possible external service mutation"), "{warning}");
        assert!(!warning.contains("file deletion"), "{warning}");
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
                "Edit file",
                "/repo/src/retry.rs",
                "1 removed · 1 added",
                "- fn retry() {",
                "+ fn retry(limit: u32) {",
                "    once();",
                "y  Yes",
            ],
        );
        assert!(
            !screen.contains("old_string"),
            "the raw arguments must give way to the diff:\n{screen}"
        );
    }

    #[tokio::test]
    async fn a_non_edit_approval_shows_plain_material_arguments() {
        let mut app = conversation();
        app.present_approval(
            prompt(
                "shell",
                serde_json::json!({"command": "rm -rf build", "cwd": "/repo"}),
            )
            .await,
        );
        let screen = render(&app, 74, 24, Theme::new());

        insta_like(&screen, &["Bash command", "rm -rf build", "in /repo"]);
        assert!(!screen.contains("\"command\""), "{screen}");
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

        insta_like(&screen, &["ctrl+o to expand", "y  Yes"]);
    }

    #[tokio::test]
    async fn a_change_buried_in_context_still_reaches_the_top_of_the_modal() {
        let old: String = (0..20).map(|n| format!("let x{n} = {n};\n")).collect();
        let new = old.replace("let x10 = 10;", "let x10 = 11;");
        let mut app = edit_approval(&old, &new).await;
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        let screen = render(&app, 74, 60, Theme::new());

        // The collapsed context is counted, not silently dropped.
        insta_like(
            &screen,
            &[
                "unchanged lines",
                "- let x10 = 10;",
                "+ let x10 = 11;",
                "y  Yes",
            ],
        );
    }

    #[tokio::test]
    async fn a_short_terminal_still_renders_an_answerable_approval() {
        let app = edit_approval("once();\n", "twice();\n").await;
        for (width, height) in [(MIN_WIDTH, MIN_HEIGHT), (44, 12), (52, 14)] {
            let screen = render(&app, width, height, Theme::new());
            insta_like(&screen, &["Edit file", "/repo/src/retry.rs"]);
            assert!(
                screen.contains("y  Yes") && screen.contains("n  No (esc)"),
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

    async fn approval_evidence_prompt(
        command: &str,
        background: bool,
    ) -> smith_host::approval::ApprovalPrompt {
        approval_evidence_prompt_with_deadline(
            command,
            background,
            Deadline::after(&SystemClock, 600_000),
        )
        .await
    }

    async fn approval_evidence_prompt_with_deadline(
        command: &str,
        background: bool,
        deadline: Deadline,
    ) -> smith_host::approval::ApprovalPrompt {
        let (policy, mut requests) = smith_host::approval::InteractiveApproval::new(1);
        let command = command.to_owned();
        tokio::spawn(async move {
            let request = ApprovalRequest::new(
                PreparedToolCall::new(
                    ToolCallId::new("approval-evidence"),
                    "shell",
                    serde_json::json!({
                        "command": command,
                        "cwd": "/repo",
                        "timeout_ms": 600_000,
                        "run_in_background": background,
                    }),
                    [Permission::ProcessSpawn, Permission::FsRead, Permission::FsWrite,
                        Permission::NetHttp, Permission::CredentialUse, Permission::DataEgress]
                        .into_iter().collect::<PermissionSet>(),
                    SecurityResource::other("host-shell", "sha256:exact-shell-action"),
                    ToolEffects::read_only().with_spawn().with_network(),
                    ToolCallDisplay::new("Run unsandboxed host shell in /repo").with_detail(
                        "cargo publish --dry-run\nHost access: same-user files and inherited credentials",
                    ),
                ),
                deadline,
                ApprovalOrigin::new(SessionId::new("session-1"), RequestId::new("request-1")),
            );
            let _ = policy.decide(&request).await;
        });
        requests.recv().await.expect("an evidence prompt")
    }

    fn approval_screen_words(screen: &str) -> String {
        screen
            .lines()
            .map(|row| row.trim().trim_matches('│').trim())
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[tokio::test]
    async fn approval_padding_keeps_wrapped_body_and_controls_inside_the_safe_width() {
        for (width, height) in [(100, 32), (80, 24), (44, 16)] {
            let mut app = App::new("gpt-5.3", "/repo");
            app.present_approval(
                approval_evidence_prompt_with_deadline(
                    "printf abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz一二三四",
                    false,
                    Deadline::never(),
                )
                .await,
            );
            for expanded in [false, true] {
                app.work_details = expanded;
                let screen = render(&app, width, height, Theme::new().without_color());
                let mut body_rows = 0;
                for row in screen.lines() {
                    assert!(row.width() <= usize::from(width), "{screen}");
                    if let Some((_, inside)) = row.split_once('│') {
                        let (inside, _) = inside.rsplit_once('│').expect("right border");
                        assert!(inside.starts_with("  "), "left padding: {row}");
                        assert!(inside.ends_with("  "), "right padding: {row}");
                        body_rows += 1;
                    }
                }
                assert!(body_rows > 0, "{screen}");
                assert!(screen.contains("│  printf"), "{screen}");
                assert!(screen.contains("│  Do you want to proceed?"), "{screen}");
                assert!(screen.contains("│    n  No (esc)"), "{screen}");
            }
        }
    }

    #[tokio::test]
    async fn approval_place_omits_an_absent_deadline_and_retains_a_real_deadline() {
        for deadline in [Deadline::never(), Deadline::after(&SystemClock, 600_000)] {
            let absolute = deadline
                .instant()
                .map(|time| crate::time_display::local_timestamp(time.as_millis()));
            let mut app = App::new("gpt-5.3", "/repo");
            app.present_approval(
                approval_evidence_prompt_with_deadline("git status --short", false, deadline).await,
            );
            for (width, height) in [(100, 32), (44, 16)] {
                let screen = render(&app, width, height, Theme::new().without_color());
                let words = approval_screen_words(&screen);
                assert!(words.contains("in /repo · up to 10 min"), "{screen}");
                assert!(!words.contains("deadline no deadline"), "{screen}");
                if let Some(absolute) = &absolute {
                    assert_eq!(words.matches("deadline").count(), 1, "{screen}");
                    assert!(words.contains(&format!("deadline {absolute}")), "{screen}");
                    assert!(words.contains("remaining"), "{screen}");
                } else {
                    assert!(!words.contains("deadline"), "{screen}");
                }
            }
        }
    }

    #[tokio::test]
    async fn approval_default_view_retains_decision_evidence_at_44_columns_without_color() {
        for (width, height) in [(100, 32), (80, 24), (44, 16)] {
            let mut app = App::new("gpt-5.3", "/repo");
            let prompt = approval_evidence_prompt("cargo publish --dry-run", true).await;
            let deadline = crate::time_display::local_timestamp(
                prompt.deadline().instant().expect("deadline").as_millis(),
            );
            let hash = prompt.prepared().fingerprint().as_str().to_owned();
            app.present_approval(prompt);
            let screen = render(&app, width, height, Theme::new().without_color());
            let words = approval_screen_words(&screen);
            for text in [
                "Bash command",
                "cargo publish --dry-run",
                "in /repo",
                "up to 10 min",
                "background",
                "deadline",
                deadline.as_str(),
                "remaining",
                "Warning:",
                "Runs outside the sandbox with your files, environment and credentials, child processes, network, and data egress.",
                "Do you want to proceed?",
                "y Yes",
                "a Yes, don't ask again for this exact shell action this session",
                "n No (esc)",
                "ctrl+o details",
            ] {
                assert!(
                    words.contains(text),
                    "{width}×{height} missing {text}:\n{screen}"
                );
            }
            let ordered = [
                "cargo publish --dry-run",
                "in /repo",
                "Warning:",
                "Do you want to proceed?",
                "y Yes",
                "a Yes",
                "n No",
            ];
            let positions = ordered.map(|text| words.find(text).expect("default evidence"));
            assert!(
                positions.windows(2).all(|pair| pair[0] < pair[1]),
                "{screen}"
            );
            assert_eq!(words.matches("Warning:").count(), 1, "{screen}");
            for hidden in [
                hash.as_str(),
                "sha256:exact-shell-action",
                "permissions:",
                "raw arguments:",
                "\"command\"",
                "\"timeout_ms\"",
            ] {
                assert!(
                    !screen.contains(hidden),
                    "{hidden} escaped the detail view:\n{screen}"
                );
            }
            assert!(
                screen.lines().all(|row| row.width() <= usize::from(width)),
                "{screen}"
            );
            if width == 44 {
                let top = screen
                    .lines()
                    .find(|row| row.contains("Bash command"))
                    .expect("border");
                assert!(top.starts_with('╭') && top.ends_with('╮'), "{screen}");
                assert_eq!(
                    top.width(),
                    44,
                    "the narrow approval must use the safe width"
                );
            }
        }
    }

    #[tokio::test]
    async fn approval_identity_permissions_and_raw_arguments_require_ctrl_o() {
        let mut app = App::new("gpt-5.3", "/repo");
        let prompt = approval_evidence_prompt("cargo publish --dry-run", false).await;
        let hash = prompt.prepared().fingerprint().as_str().to_owned();
        let permissions = prompt
            .prepared()
            .required_permissions()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        app.present_approval(prompt);
        let folded = render(&app, 80, 80, Theme::new());
        assert!(!folded.contains(&hash));
        assert!(!folded.contains("permissions:"));
        assert!(!folded.contains("\"command\""));

        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        let expanded = render(&app, 80, 80, Theme::new().without_color());
        let words = approval_screen_words(&expanded);
        insta_like(
            &expanded,
            &[
                "identity:",
                "permissions:",
                "raw arguments:",
                "\"command\"",
                "\"timeout_ms\"",
                "ctrl+o fold",
            ],
        );
        // The identity may wrap; compare without inserted row whitespace.
        assert!(words.replace(' ', "").contains(&hash), "{expanded}");
        for permission in permissions {
            assert!(
                words.contains(&permission),
                "missing {permission}:\n{expanded}"
            );
        }
        assert_eq!(app.pending_approval_count(), 1);
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(render(&app, 80, 80, Theme::new()), folded);
    }

    #[tokio::test]
    async fn long_edit_diffs_expand_and_scroll_without_a_fixed_row_cut() {
        let old: String = (0..60).map(|n| format!("let x{n} = {n};\n")).collect();
        let new = old.replace("let x", "let y");
        let mut app = edit_approval(&old, &new).await;
        let folded = render_synced(&mut app, 44, 16, Theme::new().without_color());
        assert!(folded.contains("ctrl+o to expand"), "{folded}");
        assert!(!folded.contains("+ let y23"), "{folded}");
        assert!(!folded.contains("old_string"), "{folded}");

        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        render_synced(&mut app, 44, 16, Theme::new().without_color());
        assert!(app.approval_scroll_limit > 18);
        let mut seen = String::new();
        for _ in 0..=app.approval_scroll_limit {
            seen.push_str(&render_synced(
                &mut app,
                44,
                16,
                Theme::new().without_color(),
            ));
            app.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert!(
            seen.contains("+ let y23 = 23;"),
            "the expanded diff is still cut"
        );
        assert!(seen.contains("identity:"));
        assert!(seen.contains("permissions:"));
        assert!(seen.contains("\"old_string\""));
        assert_eq!(app.pending_approval_count(), 1);
        app.on_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
        assert_eq!(app.approval_scroll, 0);
        let refolded = render(&app, 44, 16, Theme::new().without_color());
        insta_like(
            &refolded,
            &["ctrl+o to expand", "ctrl+o details", "/repo/src/retry.rs"],
        );
        assert!(!refolded.contains("old_string"), "{refolded}");
        assert!(!refolded.contains("+ let y23"), "{refolded}");
    }

    #[tokio::test]
    async fn approval_queue_hint_counts_the_shared_fifo() {
        let mut app = App::new("gpt-5.3", "/repo");
        app.present_approval(approval_evidence_prompt("first", false).await);
        app.present_questionnaire(
            QuestionnaireForm::new(
                "waiting-question",
                vec![QuestionnaireQuestion::new(
                    "question",
                    "Question",
                    "Continue?",
                    vec![QuestionnaireChoice::new("yes", "Yes")],
                )],
                Deadline::never(),
            )
            .expect("questionnaire"),
        );
        let screen = render(&app, 80, 32, Theme::new());
        insta_like(&screen, &["first", "ctrl+o details · 1 more waiting"]);
        app.present_approval(approval_evidence_prompt("third", false).await);
        let screen = render(&app, 80, 32, Theme::new());
        insta_like(&screen, &["first", "ctrl+o details · 2 more waiting"]);
    }

    #[tokio::test]
    async fn approval_no_color_keeps_structure_and_uses_no_background_fill() {
        let mut app = App::new("gpt-5.3", "/repo");
        app.present_approval(approval_evidence_prompt("cargo publish --dry-run", false).await);
        assert_eq!(
            render(&app, 44, 16, Theme::new()),
            render(&app, 44, 16, Theme::new().without_color())
        );
        let mut terminal = Terminal::new(TestBackend::new(44, 16)).expect("terminal");
        terminal
            .draw(|frame| draw(frame, &app, Theme::new().without_color()))
            .expect("frame");
        for cell in &terminal.backend().buffer().content {
            assert_eq!(cell.fg, Color::Reset);
            assert_eq!(cell.bg, Color::Reset);
        }
        let screen = screen_text(terminal.backend().buffer());
        assert!(screen.contains("Warning:"), "{screen}");
        assert!(!screen.contains('⚠'), "{screen}");
    }

    #[tokio::test]
    async fn folded_edit_keeps_its_target_material_change_and_deadline_visible_when_narrow() {
        let old: String = (0..60).map(|n| format!("let x{n} = {n};\n")).collect();
        let new = old.replace("let x", "let y");
        let app = edit_approval(&old, &new).await;
        let deadline = match &app.overlay {
            Some(Overlay::Approval { prompt, .. }) => crate::time_display::local_timestamp(
                prompt.deadline().instant().expect("deadline").as_millis(),
            ),
            _ => unreachable!(),
        };
        let screen = render(&app, 44, 16, Theme::new().without_color());
        let words = approval_screen_words(&screen);
        for required in [
            "/repo/src/retry.rs",
            "60 removed · 60 added",
            "- let x0 = 0;",
            "ctrl+o to expand",
            "deadline",
            deadline.as_str(),
            "remaining",
            "Do you want to proceed?",
            "y Yes",
            "n No (esc)",
        ] {
            assert!(words.contains(required), "missing {required}:\n{screen}");
        }
        assert!(!screen.contains("identity:"), "{screen}");
        assert!(!screen.contains("permissions:"), "{screen}");
        assert!(!screen.contains("old_string"), "{screen}");
    }

    #[tokio::test]
    async fn transcript_scrolling_redraws_the_work_behind_an_unanswered_approval() {
        let visible_work = |screen: &str| {
            screen
                .lines()
                .filter_map(|line| {
                    let (_, index) = line.split_once("earlier work ")?;
                    index.trim().parse::<usize>().ok()
                })
                .collect::<std::collections::BTreeSet<_>>()
        };
        let mut app = App::new("gpt-5.3", "/repo");
        for index in 0..50 {
            app.transcript.push_user(format!("earlier work {index}"));
        }
        app.present_approval(approval_evidence_prompt("cargo publish --dry-run", false).await);
        let before = render_synced(&mut app, 100, 32, Theme::new().without_color());
        let before_work = visible_work(&before);
        assert!(!before_work.is_empty(), "{before}");
        assert!(app.following);
        app.on_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
        let after = render_synced(&mut app, 100, 32, Theme::new().without_color());
        assert_eq!(app.scroll_back, 10);
        let after_work = visible_work(&after);
        assert!(!after_work.is_empty(), "{after}");
        assert!(
            after_work.last().expect("visible work after PageUp")
                < before_work.last().expect("visible work before PageUp"),
            "PageUp did not move back in history: {before_work:?} -> {after_work:?}"
        );
        insta_like(
            &after,
            &["Bash command", "cargo publish --dry-run", "n  No (esc)"],
        );
        assert_eq!(app.pending_approval_count(), 1);
        app.on_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        let newest = render_synced(&mut app, 100, 32, Theme::new().without_color());
        assert_eq!(app.scroll_back, 0);
        assert_eq!(visible_work(&newest), before_work, "{newest}");
        assert_eq!(app.pending_approval_count(), 1);
    }
