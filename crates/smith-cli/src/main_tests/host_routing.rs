// host routing behavior tests.

    #[test]
    fn runtime_timeline_uses_stable_ids_terminal_plan_and_redacted_gate_evidence() {
        use agent_runtime_core::clock::Timestamp;
        use smith_runtime::client::PlanSensitivity;
        use agent_runtime_core::ids::{EventId, ToolCallId, TurnId};

        let session = SessionId::new("session-1");
        let turn = TurnId::new("turn-7");
        let envelope = |seq, payload| {
            EventEnvelope::new(
                seq,
                EventId::new(format!("event-{seq}")),
                session.clone(),
                Some(turn.clone()),
                Timestamp::ZERO,
                payload,
            )
        };
        let events = vec![
            envelope(
                1,
                RuntimeEvent::PlanUpdated {
                    revision: 3,
                    sensitivity: PlanSensitivity::Public,
                    counts: BTreeMap::from([
                        ("pending".to_owned(), 0),
                        ("in_progress".to_owned(), 0),
                        ("completed".to_owned(), 2),
                        ("cancelled".to_owned(), 1),
                    ]),
                    items: Some(Vec::new()),
                },
            ),
            envelope(
                2,
                RuntimeEvent::ToolCallRequested {
                    call: ToolCallId::new("call-4"),
                    name: "shell".to_owned(),
                    argument_keys: vec!["command".to_owned()],
                    argument_fingerprint: serde_json::from_value(serde_json::json!(
                        "0123456789abcdef0123456789abcdef"
                    ))
                    .expect("fingerprint"),
                    arguments: Some(serde_json::json!({"command": "secret-command"})),
                },
            ),
            envelope(
                3,
                RuntimeEvent::ToolCallCompleted {
                    call: ToolCallId::new("call-4"),
                    name: "shell".to_owned(),
                    is_error: false,
                },
            ),
            envelope(
                4,
                RuntimeEvent::TurnCompleted {
                    finish: TurnFinish::Completed,
                    visible_output: true,
                },
            ),
            envelope(
                5,
                RuntimeEvent::ChildSpawned {
                    child: ChildId::new("child-2"),
                    workspace: WorkspacePolicy::ReadOnlyView,
                    max_turns: 1,
                    max_tokens: None,
                    deadline_ms: None,
                },
            ),
            envelope(
                6,
                RuntimeEvent::ChildCompleted {
                    child: ChildId::new("child-2"),
                    result: "secret-child-result".to_owned(),
                },
            ),
        ];

        let rendered = render_runtime_timeline(&events);
        assert_eq!(
            rendered.lines,
            [
                "root turn-7 · completed · plan 0 active/0 pending/2 done/1 cancelled · gates 1 passed/0 failed",
                "child child-2 · started · ReadOnlyView · 1 turn limit",
                "child child-2 · task completed",
            ]
        );
        assert_eq!(rendered.children, BTreeSet::from([ChildId::new("child-2")]));
        let joined = rendered.lines.join("\n");
        assert!(!joined.contains("secret-command"));
        assert!(!joined.contains("secret-child-result"));
    }

    #[tokio::test]
    async fn pending_user_admission_gate_beats_automatic_goal_continuation() {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            LOCAL_COMMAND_CONFIG,
        )
        .expect("config");
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolution")
            .config;
        let provider = Arc::new(agent_runtime::provider::fake::FakeProvider::new(
            "example-model",
            agent_runtime_core::provider::Capabilities::basic_streaming(),
            vec![agent_runtime::provider::fake::ScriptedStream::new(vec![
                agent_runtime_core::provider::ProviderStreamEvent::TextDelta {
                    text: "user work completed".to_owned(),
                },
                agent_runtime_core::provider::ProviderStreamEvent::Finish {
                    reason: agent_runtime_core::provider::FinishReason::Stop,
                },
            ])],
        ));
        let runtime = RuntimeRequest {
            provider: Some(provider.clone()),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(agent_runtime_core::approval::DenyAll)),
            ..RuntimeRequest::new(config, HostSurface::Terminal)
        };
        let host = smith_runtime::host::start(
            HostSessionRequest::new(runtime, project.path())
                .checkpoint_keys(Arc::new(TestCheckpointKeys)),
        )
        .await
        .expect("host");

        host.set_goal_continuation_enabled(false);
        host.control_goal(GoalCommand::Create {
            objective: "finish the goal".to_owned(),
            token_budget: None,
        })
        .await
        .expect("goal creation");
        tokio::task::yield_now().await;
        assert!(
            provider.requests().is_empty(),
            "a disabled goal gate admitted automatic work"
        );

        let user = host
            .session()
            .send(UserInput::text("real user wins the boundary"))
            .expect("real-user admission");
        host.set_goal_continuation_enabled(true);
        user.completed().await;
        host.set_goal_continuation_enabled(false);

        let requests = provider.requests();
        let first = serde_json::to_string(&requests[0].messages).expect("provider request");
        assert!(first.contains("real user wins the boundary"), "{first}");
        host.shutdown().await.expect("shutdown");
    }

    #[test]
    fn session_update_timestamps_are_human_readable() {
        let now = time::OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("valid fixture time")
            .to_offset(time::UtcOffset::UTC);
        assert_eq!(
            format_session_updated_at(Timestamp(1_699_999_877_000), time::UtcOffset::UTC, now,),
            "2 minutes ago"
        );
        assert_eq!(
            format_session_updated_at(Timestamp(1_699_989_195_000), time::UtcOffset::UTC, now,),
            "3 hours ago"
        );
        assert_eq!(
            format_session_updated_at(Timestamp(1_699_900_000_000), time::UtcOffset::UTC, now),
            "2023-11-13 18:26:40 +00:00"
        );
    }

    #[test]
    fn palette_reconfiguration_preserves_or_replaces_the_intended_session() {
        let mut selection = Selection {
            profile: Some("old".into()),
            provider: Some("explicit-provider".into()),
            model: Some("explicit-model".into()),
            ..Selection::default()
        };
        let mut resume = Some("older-session".into());

        apply_palette_command(
            &mut selection,
            &mut resume,
            "current-session".into(),
            PaletteCommand::Profile("work".into()),
        );
        assert_eq!(selection.profile.as_deref(), Some("work"));
        assert_eq!(selection.provider, None);
        assert_eq!(selection.model, None);
        assert_eq!(resume.as_deref(), Some("current-session"));

        apply_palette_command(
            &mut selection,
            &mut resume,
            "current-session".into(),
            PaletteCommand::Agent("review".into()),
        );
        assert_eq!(selection.profile.as_deref(), Some("work"));
        assert_eq!(selection.agent.as_deref(), Some("review"));

        apply_palette_command(
            &mut selection,
            &mut resume,
            "current-session".into(),
            PaletteCommand::NewSession,
        );
        assert_eq!(resume, None);

        apply_palette_command(
            &mut selection,
            &mut resume,
            "ignored".into(),
            PaletteCommand::Resume("selected-session".into()),
        );
        assert_eq!(resume.as_deref(), Some("selected-session"));
    }

    #[test]
    fn reasoning_startup_errors_are_distinguished_for_compatible_switch_cleanup() {
        let reasoning = anyhow::Error::new(FactoryError::Reasoning {
            provider: "example".to_owned(),
            model: ModelId::new("fixed-model"),
            message: "reasoning is not adjustable".to_owned(),
        })
        .context("starting the Smith session");
        assert!(is_reasoning_startup_error(&reasoning));

        let unrelated = anyhow::anyhow!("provider is unavailable");
        assert!(!is_reasoning_startup_error(&unrelated));
    }

    #[test]
    fn an_invocation_effort_is_never_what_the_startup_recovery_path_clears() {
        // A saved override on resume, and an in-session choice: both were made
        // against another binding, so both are recoverable.
        assert!(reasoning_selection_is_recoverable(
            &Selection::default(),
            true
        ));
        assert!(reasoning_selection_is_recoverable(
            &Selection {
                reasoning_effort: Some("high".into()),
                ..Selection::default()
            },
            false
        ));

        // An invocation effort on its own is not recoverable: there is nothing
        // to clear but the flag, and the flag is the instruction. The run must
        // fail with the reasoning diagnostic instead of starting at some other
        // effort.
        assert!(!reasoning_selection_is_recoverable(
            &Selection {
                effort: Some("high".into()),
                ..Selection::default()
            },
            false
        ));

        // A fresh start with nothing selected has nothing to recover.
        assert!(!reasoning_selection_is_recoverable(
            &Selection::default(),
            false
        ));

        // Whatever the arm clears, the flag survives it — which is why a retry
        // that still cannot honor the flag fails rather than starting.
        let mut cleared = Selection {
            effort: Some("high".into()),
            reasoning_effort: Some("low".into()),
            reasoning_enabled: Some(true),
            ..Selection::default()
        };
        assert!(reasoning_selection_is_recoverable(&cleared, true));
        cleared.reasoning_enabled = None;
        cleared.reasoning_effort = None;
        cleared.reasoning_enabled_reset = true;
        cleared.reasoning_effort_reset = true;
        assert_eq!(cleared.effort.as_deref(), Some("high"));
        assert!(
            !reasoning_selection_is_recoverable(&cleared, true),
            "a second retry would loop instead of reporting the failure"
        );
    }

    #[test]
    fn a_child_profile_never_inherits_the_invocation_effort() {
        // Mirrors what `start_host` composes for each child-enabled profile:
        // the flag was chosen against the main binding, and a child profile
        // may sit on one with no ladder at all.
        let selection = Selection {
            profile: Some("work".into()),
            provider: Some("acme".into()),
            model: Some("example-model".into()),
            effort: Some("high".into()),
            reasoning_effort: Some("low".into()),
            reasoning_enabled: Some(true),
            ..Selection::default()
        };
        let mut child = selection.clone();
        child.profile = Some("explore".into());
        child.provider = None;
        child.model = None;
        child.reasoning_enabled = None;
        child.reasoning_effort = None;
        child.effort = None;

        assert_eq!(child.overrides().reasoning_effort, None);
        assert_eq!(child.session_overrides().reasoning_effort, None);
        assert_eq!(
            selection.overrides().reasoning_effort.as_deref(),
            Some("high"),
            "the parent selection must be left intact"
        );
    }
