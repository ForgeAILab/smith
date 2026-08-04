// child lifecycle behavior tests.

    #[test]
    fn accepted_child_confirmation_input_enters_history_before_confirmation() {
        let mut app = agent_first_app();
        app.composer.replace("@review inspect the diff");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(&app.overlay, Some(Overlay::AgentConfirm { .. })));

        app.on_key(key(KeyCode::Char('n')));
        assert!(app.overlay.is_none());
        app.composer.clear();
        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "@review inspect the diff");
    }

    #[test]
    fn legacy_profile_choices_reconfigure_the_legacy_agent_override() {
        let mut app = agent_first_app();
        app.resources.profiles.push(ResourceEntry::new(
            format!("{LEGACY_AGENT_PROFILE_PREFIX}review"),
            "review",
            "legacy root-mode adapter",
        ));
        app.composer.replace("/profile review");

        assert_eq!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Reconfigure(PaletteCommand::Agent(
                "review".to_owned()
            )))
        );
    }

    #[test]
    fn explicit_child_requires_non_default_spend_confirmation() {
        let mut app = agent_first_app();
        app.composer.replace("@review inspect the diff");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::AgentConfirm { ref preset, ref task, ref content })
                if preset == "review"
                    && task == "inspect the diff"
                    && content.contains("zai/glm-5.2")
                    && content.contains("read-only")
                    && content.contains("provider spend: yes")
        ));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(app.overlay, Some(Overlay::AgentConfirm { .. })));
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::StartAgent {
                preset: "review".to_owned(),
                task: "inspect the diff".to_owned(),
            })
        );
        assert!(app.composer.is_empty());
    }

    #[test]
    fn disabled_child_profile_fails_locally_and_preserves_the_draft() {
        let mut app = agent_first_app();
        app.resources.child_agents.push(
            ResourceEntry::new(
                "agent:main-only",
                "main-only",
                "main profile · unavailable for child use",
            )
            .disabled("profile is enabled only for main-agent use"),
        );
        app.composer.replace("@main-only inspect the diff");

        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(app.composer.text(), "@main-only inspect the diff");
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Error { message } if message.contains("unresolved reference")
        )));
        assert!(app.overlay.is_none());
    }

    #[test]
    fn existing_child_reference_confirms_a_follow_up_instead_of_spawning() {
        let mut app = agent_first_app();
        app.restore_child(
            "child-1",
            "idle",
            Some("durable · session child-session-1 · 1/4 turns".to_owned()),
        );
        app.composer.replace("@child-1 check the parser edge case");

        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::AgentFollowUpConfirm {
                ref child_id,
                ref task,
                ref content,
            }) if child_id == "child-1"
                && task == "check the parser edge case"
                && content.contains("new follow-up turn")
                && content.contains("reuse prior child history")
        ));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::FollowUpAgent {
                child_id: "child-1".to_owned(),
                task: "check the parser edge case".to_owned(),
            })
        );
    }

    #[test]
    fn interrupted_child_resume_is_explicit_and_has_no_enter_default() {
        let mut app = agent_first_app();
        app.restore_child(
            "child-2",
            "interrupted",
            Some("durable · session child-session-2 · resumable".to_owned()),
        );
        app.composer.replace("/agent resume child-2");

        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::AgentResumeConfirm { ref child_id, ref content })
                if child_id == "child-2"
                    && content.contains("exact interrupted checkpoint")
                    && content.contains("turn slot consumed: no")
        ));
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::ResumeAgent {
                child_id: "child-2".to_owned(),
            })
        );
    }

    #[test]
    fn local_status_and_agent_commands_never_become_provider_sends() {
        for command in ["/status", "/context", "/agent", "/diff"] {
            let mut app = app();
            type_text(&mut app, command);
            assert!(matches!(
                app.on_key(key(KeyCode::Enter)),
                Some(Action::Command(_))
            ));
            assert!(
                !app.transcript
                    .blocks()
                    .iter()
                    .any(|block| matches!(block, Block::User { .. }))
            );
        }
    }

    #[test]
    fn child_lifecycle_is_inline_and_available_for_agent_detail() {
        let mut app = app();
        let child = ChildId::new("child-1");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "No findings.".to_owned(),
        }));
        assert_eq!(app.children[child.as_str()].state, "completed");
        assert!(app.transcript.blocks().iter().any(|block| {
            matches!(block, Block::Notice { text, .. } if text.contains("No findings"))
        }));
    }

    #[test]
    fn durable_child_recovery_and_resume_replay_match_live_projection() {
        let child = ChildId::new("child-9");
        let session = SessionId::new("child-session-9");
        let payloads = vec![
            RuntimeEvent::ChildProgress {
                child: child.clone(),
                phase: ChildPhase::Recovered {
                    child_session: session.clone(),
                    state: ChildRecoveryState::Interrupted,
                    resumable: true,
                },
            },
            RuntimeEvent::ChildProgress {
                child: child.clone(),
                phase: ChildPhase::ResumeStarted {
                    child_session: session.clone(),
                },
            },
            RuntimeEvent::ChildProgress {
                child: child.clone(),
                phase: ChildPhase::Interrupted {
                    child_session: session,
                    resumable: false,
                },
            },
        ];
        let events = payloads
            .into_iter()
            .enumerate()
            .map(|(sequence, payload)| {
                EventEnvelope::new(
                    u64::try_from(sequence).expect("bounded sequence"),
                    EventId::new(format!("child-event-{sequence}")),
                    SessionId::new("parent-session"),
                    None,
                    Timestamp::ZERO,
                    payload,
                )
            })
            .collect::<Vec<_>>();
        let encoded = serde_json::to_vec(&events).expect("events serialize");
        let replayed: Vec<EventEnvelope> =
            serde_json::from_slice(&encoded).expect("events deserialize");

        let mut live = app();
        for event in &events {
            live.apply(event);
        }
        let mut replay = app();
        for event in &replayed {
            replay.apply(event);
        }

        assert_eq!(live.children, replay.children);
        assert_eq!(live.transcript.blocks(), replay.transcript.blocks());
        assert_eq!(live.children[child.as_str()].state, "interrupted");
        assert!(
            live.children[child.as_str()]
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("no compatible checkpoint"))
        );
    }

    #[test]
    fn child_needs_input_is_metadata_only_and_does_not_open_a_questionnaire() {
        let mut app = app();
        let child = ChildId::new("child-ask");
        app.apply(&event(RuntimeEvent::ChildNeedsInput {
            child: child.clone(),
            child_session: SessionId::new("child-session"),
            turn: TurnId::new("child-turn"),
            call: ToolCallId::new("ask-call"),
            request: InteractionRequestId::new("child-request"),
            question_ids: vec![
                QuestionId::new("question-one"),
                QuestionId::new("question-two"),
            ],
            sensitivity: InteractionSensitivity::Sensitive,
        }));

        let summary = &app.children[child.as_str()];
        assert_eq!(summary.state, "needs input");
        assert!(
            summary
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("2 questions"))
        );
        assert!(
            app.overlay.is_none(),
            "a child request must not seize the root questionnaire overlay"
        );
        assert!(app.transcript.blocks().iter().any(|block| {
            matches!(
                block,
                Block::Notice { source, text }
                    if source == "sub-agent"
                        && text.contains("child-request")
                        && text.contains("2 questions")
            )
        }));
    }

    #[test]
    fn child_clocks_tick_live_and_freeze_when_the_child_settles() {
        let mut app = app();
        let child = ChildId::new("child-clock");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        assert_eq!(app.live_child_count(), 1);
        assert!(app.child_elapsed(child.as_str()).is_some());

        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "done".to_owned(),
        }));
        assert_eq!(app.live_child_count(), 0);
        let frozen = app.child_elapsed(child.as_str()).expect("a settled clock");
        assert_eq!(
            app.child_elapsed(child.as_str()),
            Some(frozen),
            "a settled clock reads the same twice"
        );

        // A recovered durable record ran in another process; no honest
        // wall-clock exists for it.
        app.apply(&event(RuntimeEvent::ChildProgress {
            child: child.clone(),
            phase: ChildPhase::Recovered {
                child_session: SessionId::new("child-session-clock"),
                state: ChildRecoveryState::Idle,
                resumable: false,
            },
        }));
        assert_eq!(app.child_elapsed(child.as_str()), None);
    }
