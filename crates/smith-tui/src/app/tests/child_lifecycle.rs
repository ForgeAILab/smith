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
    fn disabled_child_profile_name_is_sent_as_literal_text() {
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

        let submission = expect_whole_submission(app.on_key(key(KeyCode::Enter)));
        assert_eq!(submission.display_text(), "@main-only inspect the diff");
        assert!(submission.files().is_empty());
        assert!(app.composer.is_empty());
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
    fn completed_parent_turn_projects_parked_without_an_open_provider_turn() {
        let mut app = app();
        let child = ChildId::new("child-parked");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply(&event(RuntimeEvent::TurnCompleted {
            finish: TurnFinish::Completed,
            visible_output: true,
        }));

        assert_eq!(app.status.activity, Activity::ParkedAwaitingChild);
        assert_eq!(app.status.activity.label(), "waiting for child");
        assert!(app.active_turn.is_none());

        app.apply(&event(RuntimeEvent::ChildCompleted {
            child,
            result: "done".to_owned(),
        }));
        assert_eq!(app.status.activity, Activity::Idle);
    }

    #[test]
    fn child_progress_stays_out_of_the_root_transcript_and_in_the_child_log() {
        let mut app = app();
        let child = ChildId::new("child-1");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 3,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply(&event(RuntimeEvent::ChildProgress {
            child: child.clone(),
            phase: ChildPhase::TurnStarted,
        }));
        // What the child *did* comes from the child's own stream — the same
        // events, folded by the same code, into its own transcript.
        app.apply_child(child.as_str(), &event(tool_requested("call-1", "search")));
        app.apply_child(
            child.as_str(),
            &event(tool_completed("call-1", "search", false)),
        );
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::TextDelta {
                request: RequestId::new("request-1"),
                attempt: AttemptId::new("attempt-1"),
                text: "Two call sites.".to_owned(),
            }),
        );
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::ProviderAttemptOutputCommitted {
                request: RequestId::new("request-1"),
                attempt: AttemptId::new("attempt-1"),
            }),
        );
        app.apply(&event(RuntimeEvent::ChildProgress {
            child: child.clone(),
            phase: ChildPhase::TurnFinished,
        }));
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "Two call sites.".to_owned(),
        }));

        let notices = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::Notice { source, text } if source == "sub-agent" => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        // The reviewed spawn row is now the one place a spawn is announced
        // (there is no `agent` tool call here to carry one, since this test
        // drives lifecycle events directly), so only the terminal outcome
        // keeps its attributed transcript line.
        assert_eq!(
            notices.len(),
            1,
            "the started notice is gone; only the terminal outcome remains: {notices:?}"
        );
        assert!(notices[0].contains("Two call sites"));
        assert!(
            !notices.iter().any(|text| text.contains("is working")
                || text.contains("search")
                || text.contains("started")),
            "mid-flight progress must not narrate itself into the root transcript"
        );

        assert_eq!(
            child_log(&app, child.as_str()),
            [
                "started · read-only · up to 3 turns",
                "turn · working",
                "ok search",
                "completed: Two call sites.",
            ],
            "the inspector keeps the record the transcript no longer prints"
        );
    }

    #[test]
    fn a_long_running_child_keeps_a_bounded_log_tail() {
        let mut app = app();
        let child = ChildId::new("child-busy");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: u32::MAX,
            max_tokens: None,
            deadline_ms: None,
        }));
        for index in 0..MAX_CHILD_BLOCKS + 50 {
            let call = format!("call-{index}");
            let name = format!("read{index}");
            app.apply_child(child.as_str(), &event(tool_requested(&call, &name)));
            app.apply_child(child.as_str(), &event(tool_completed(&call, &name, false)));
        }

        let log = child_log(&app, child.as_str());
        assert_eq!(
            log.len(),
            MAX_CHILD_BLOCKS,
            "a call and its outcome are one row, not two"
        );
        assert_eq!(
            log.last().map(String::as_str),
            Some(format!("ok read{}", MAX_CHILD_BLOCKS + 49).as_str()),
            "the newest activity is what a bounded tail keeps"
        );
    }

    #[test]
    fn a_child_tool_row_carries_the_arguments_and_result_the_host_resolves() {
        let mut app = app();
        let child = ChildId::new("child-1");
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 1,
            max_tokens: None,
            deadline_ms: None,
        }));
        app.apply_child(child.as_str(), &event(tool_requested("call-1", "read")));

        // Before the host answers, the row says the shape of the call and
        // never guesses at its values.
        let Some(Block::Tool {
            display,
            protected_summary,
            status,
            ..
        }) = app.child_blocks(child.as_str()).last()
        else {
            panic!("the child's tool call is a tool row");
        };
        assert!(display.is_none());
        assert!(
            protected_summary.contains("path"),
            "the protected fallback names the argument keys: {protected_summary}"
        );
        assert_eq!(*status, ToolStatus::Running);

        app.set_child_tool_display(
            child.as_str(),
            "call-1",
            smith_tools::project_tool_call_display(
                "read",
                &serde_json::json!({"path": "src/retry.rs"}),
            )
            .expect("reviewed read projection"),
        );
        app.apply_child(
            child.as_str(),
            &event(tool_completed("call-1", "read", false)),
        );
        app.set_child_tool_result_preview(child.as_str(), "call-1", "fn retry() {}");

        let Some(Block::Tool {
            display,
            status,
            result_preview,
            ..
        }) = app.child_blocks(child.as_str()).last()
        else {
            panic!("the outcome resolves the same row");
        };
        assert_eq!(
            display.as_ref().map(smith_tools::ToolCallDisplay::target),
            Some("src/retry.rs")
        );
        assert_eq!(*status, ToolStatus::Ok);
        assert_eq!(result_preview.as_deref(), Some("fn retry() {}"));
    }

    #[test]
    fn a_failed_child_tool_call_reads_as_failed() {
        let mut app = app();
        let child = ChildId::new("child-1");
        app.apply_child(child.as_str(), &event(tool_requested("call-1", "shell")));
        app.apply_child(
            child.as_str(),
            &event(tool_completed("call-1", "shell", true)),
        );
        assert_eq!(child_log(&app, child.as_str()), ["failed shell"]);
    }

    /// A child's answer streams into its own transcript. The parent's copy of
    /// it, delivered later at the safe boundary, must not print it a second
    /// time under the first.
    #[test]
    fn a_live_child_answer_is_not_repeated_by_the_parent_completion() {
        let mut app = app();
        let child = ChildId::new("child-1");
        let request = RequestId::new("request-1");
        let attempt = AttemptId::new("attempt-1");
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::TextDelta {
                request: request.clone(),
                attempt: attempt.clone(),
                text: "Two call sites.".to_owned(),
            }),
        );
        app.apply_child(
            child.as_str(),
            &event(RuntimeEvent::ProviderAttemptOutputCommitted {
                request: request.clone(),
                attempt: attempt.clone(),
            }),
        );
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "Two call sites.".to_owned(),
        }));

        assert_eq!(
            child_log(&app, child.as_str()),
            ["completed: Two call sites."],
            "the child said it once"
        );
    }

    /// The same completion for a child the client never heard from — a durable
    /// record recovered after a restart — is the only copy of its answer, so
    /// it is still shown.
    #[test]
    fn a_recovered_child_answer_still_comes_from_the_parent_completion() {
        let mut app = app();
        let child = ChildId::new("child-1");
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "No findings.".to_owned(),
        }));
        assert_eq!(
            child_log(&app, child.as_str()),
            ["completed: No findings."],
            "a child with no live stream is not left with an empty log"
        );
    }

    #[test]
    fn a_stopped_child_settles_the_call_it_was_in_the_middle_of() {
        let mut app = app();
        let child = ChildId::new("child-1");
        app.apply_child(child.as_str(), &event(tool_requested("call-1", "shell")));
        app.apply(&event(RuntimeEvent::ChildStopped {
            child: child.clone(),
            reason: CancelReason::UserRequested,
        }));
        assert_eq!(
            child_log(&app, child.as_str())[0],
            "ran shell",
            "a call nobody will report an outcome for stops claiming it is running"
        );
    }

    #[test]
    fn arrow_keys_walk_the_agents_panel_and_escape_returns_to_the_root() {
        let mut app = app();
        app.restore_child("child-done", "completed", Some("No findings.".to_owned()));
        app.restore_child("child-live", "working", Some("ran Read".to_owned()));

        // Live work sorts first in the panel, so it selects first too.
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.inspected_child.as_deref(), Some("child-live"));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.inspected_child.as_deref(), Some("child-done"));
        // The last row holds rather than wrapping back to the top.
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.inspected_child.as_deref(), Some("child-done"));

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.inspected_child.as_deref(), Some("child-live"));
        app.on_key(key(KeyCode::Up));
        assert_eq!(
            app.inspected_child, None,
            "the row above the first child is the root timeline"
        );

        app.on_key(key(KeyCode::Down));
        assert_eq!(app.inspected_child.as_deref(), Some("child-live"));
        assert_eq!(app.on_key(key(KeyCode::Esc)), None);
        assert_eq!(app.inspected_child, None);
    }

    #[test]
    fn composer_history_keeps_the_arrows_until_it_runs_out() {
        let mut app = app();
        app.restore_child("child-live", "working", None);
        app.composer.replace("earlier message");
        app.composer.record_current();
        app.composer.clear();

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.composer.text(), "earlier message");
        assert_eq!(
            app.inspected_child, None,
            "recall owns the arrows while it has somewhere to go"
        );
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.composer.text(), "");
        assert_eq!(app.inspected_child, None);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.inspected_child.as_deref(), Some("child-live"));
    }

    #[test]
    fn an_ordinary_submission_while_inspecting_continues_that_child() {
        let mut app = agent_first_app();
        app.restore_child("child-1", "idle", Some("1/4 turns".to_owned()));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.inspected_child.as_deref(), Some("child-1"));

        type_text(&mut app, "check that edge case");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(matches!(
            app.overlay,
            Some(Overlay::AgentFollowUpConfirm { ref child_id, ref task, .. })
                if child_id == "child-1" && task == "check that edge case"
        ));
        assert_eq!(
            app.on_key(key(KeyCode::Char('y'))),
            Some(Action::FollowUpAgent {
                child_id: "child-1".to_owned(),
                task: "check that edge case".to_owned(),
            })
        );
    }

    #[test]
    fn a_working_child_refuses_a_follow_up_where_the_user_can_see_it() {
        let mut app = agent_first_app();
        app.restore_child("child-1", "working", Some("ran Read".to_owned()));
        app.on_key(key(KeyCode::Down));

        type_text(&mut app, "also check the parser");
        assert_eq!(app.on_key(key(KeyCode::Enter)), None);
        assert!(app.overlay.is_none(), "no follow-up may be confirmed");
        assert!(app.transcript.blocks().iter().any(|block| matches!(
            block,
            Block::Error { message } if message.contains("takes a follow-up once it settles")
        )));
        assert!(
            child_log(&app, "child-1")
                .iter()
                .any(|line| line.contains("follow-up refused")),
            "the refusal is also visible in the view the user is reading"
        );
    }

    #[test]
    fn a_local_command_while_inspecting_still_addresses_the_root() {
        let mut app = agent_first_app();
        app.restore_child("child-1", "idle", None);
        app.on_key(key(KeyCode::Down));

        type_text(&mut app, "/status");
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            Some(Action::Command(CommandAction::Status))
        ));
        assert_eq!(
            app.inspected_child.as_deref(),
            Some("child-1"),
            "a root command does not close the read-only view"
        );
    }

    #[test]
    fn a_stale_inspector_card_never_lands_on_another_child() {
        let mut app = app();
        app.restore_child("child-a", "working", None);
        app.restore_child("child-b", "working", None);
        app.inspect_child("child-a");
        app.set_inspected_detail("child-a", Some("session child-session-a".to_owned()));
        assert_eq!(app.inspected_detail(), Some("session child-session-a"));

        app.inspect_child("child-b");
        assert_eq!(
            app.inspected_detail(),
            None,
            "one child's accounting must not appear under another's name"
        );
        app.set_inspected_detail("child-a", Some("session child-session-a".to_owned()));
        assert_eq!(app.inspected_detail(), None);
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
        assert_eq!(
            app.status.activity,
            Activity::Idle,
            "a terminal needs-input outcome is inspectable metadata, not live child work"
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

    fn spawned_child(app: &mut App, id: &str) -> ChildId {
        let child = ChildId::new(id);
        app.apply(&event(RuntimeEvent::ChildSpawned {
            child: child.clone(),
            workspace: WorkspacePolicy::ReadOnlyView,
            max_turns: 3,
            max_tokens: None,
            deadline_ms: None,
        }));
        child
    }

    #[test]
    fn a_cleanly_finished_row_retires_itself_but_the_child_stays_known() {
        let mut app = app();
        let child = spawned_child(&mut app, "child-tidy");
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "done".to_owned(),
        }));

        let due = Instant::now() + COMPLETED_CHILD_LINGER;
        assert!(
            !app.expire_child_rows_at(due - Duration::from_millis(1)),
            "the outcome stays up long enough to read"
        );
        assert_eq!(app.inspectable_children(), [child.as_str()]);

        assert!(app.expire_child_rows_at(due), "the linger window closed");
        assert!(app.visible_children().is_empty());
        assert!(
            app.inspectable_children().is_empty(),
            "a row the eye cannot see must not be reachable by keyboard either"
        );
        assert!(
            app.children.contains_key(child.as_str()),
            "retiring a row must not forget the child: it still takes a follow-up"
        );
        assert_eq!(child_log(&app, child.as_str()).len(), 2);
    }

    #[test]
    fn only_an_outcome_nobody_has_to_act_on_retires_on_its_own() {
        let mut app = app();
        let failed = spawned_child(&mut app, "child-failed");
        app.apply(&event(RuntimeEvent::ChildFailed {
            child: failed.clone(),
            error: RuntimeError::internal("provider refused"),
        }));
        let stopped = spawned_child(&mut app, "child-stopped");
        app.apply(&event(RuntimeEvent::ChildStopped {
            child: stopped.clone(),
            reason: CancelReason::UserRequested,
        }));

        assert!(
            !app.expire_child_rows_at(Instant::now() + COMPLETED_CHILD_LINGER * 10),
            "a failure or a stop is unfinished business and stays on the panel"
        );
        assert_eq!(
            app.inspectable_children(),
            [failed.as_str(), stopped.as_str()]
        );
    }

    #[test]
    fn a_row_under_inspection_never_retires_out_from_under_the_reader() {
        let mut app = app();
        let child = spawned_child(&mut app, "child-read");
        app.inspect_child(child.to_string());
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "done".to_owned(),
        }));

        let long_read = Instant::now() + COMPLETED_CHILD_LINGER * 10;
        assert!(!app.expire_child_rows_at(long_read));
        assert_eq!(app.inspectable_children(), [child.as_str()]);

        // Looking away starts the countdown, it does not skip it: a row must
        // not vanish in the same frame the reader pressed Esc.
        assert!(app.leave_child_inspection());
        assert!(!app.expire_child_rows_at(Instant::now()));
        assert!(app.expire_child_rows_at(Instant::now() + COMPLETED_CHILD_LINGER));
        assert!(app.inspectable_children().is_empty());
    }

    #[test]
    fn any_further_activity_brings_a_retired_row_back() {
        let mut app = app();
        let child = spawned_child(&mut app, "child-again");
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "done".to_owned(),
        }));
        assert!(app.expire_child_rows_at(Instant::now() + COMPLETED_CHILD_LINGER));
        assert!(app.inspectable_children().is_empty());

        app.apply(&event(RuntimeEvent::ChildProgress {
            child: child.clone(),
            phase: ChildPhase::TurnStarted,
        }));
        assert_eq!(
            app.inspectable_children(),
            [child.as_str()],
            "a follow-up turn is exactly the activity the panel exists to show"
        );
    }

    #[test]
    fn the_inspector_keeps_the_whole_answer_the_panel_row_clips() {
        let mut app = app();
        let child = spawned_child(&mut app, "child-verbose");
        let result = format!("## report\n\n{}", "detail ".repeat(100));
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: result.clone(),
        }));

        assert_eq!(
            child_log(&app, child.as_str()).last().map(String::as_str),
            Some(format!("completed: {result}").as_str()),
            "the inspector is where delegated work is read, so it keeps it whole"
        );
        let summary = app.children[child.as_str()]
            .detail
            .clone()
            .expect("a completed child names its outcome");
        assert!(
            summary.chars().count() <= 201 && summary.ends_with('…'),
            "the one-line panel row still clips: {summary}"
        );
    }

    #[test]
    fn an_enormous_child_answer_is_still_bounded() {
        let mut app = app();
        let child = spawned_child(&mut app, "child-flood");
        app.apply(&event(RuntimeEvent::ChildCompleted {
            child: child.clone(),
            result: "x".repeat(MAX_CHILD_ANSWER_CHARS * 2),
        }));

        let Some(Block::Assistant { text, .. }) = app.child_blocks(child.as_str()).last() else {
            panic!("the answer is assistant prose");
        };
        assert_eq!(text.chars().count(), MAX_CHILD_ANSWER_CHARS + 1);
        assert!(text.ends_with('…'), "the clip says that it clipped");
    }
