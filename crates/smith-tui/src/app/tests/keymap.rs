mod keymap {
    use super::*;
    use crossterm::event::Event as TerminalInput;
    use smith_client::keymap::{
        BindingCase, Direction, Edge, KEY_BINDINGS, Key as NamedKey, KeyChord, KeyContext,
        KeyEffect, KeyInput, Modifier,
    };

    const TASK: &str = "keymap task";
    const DRAFT: &str = "first\none two tail\nlast";
    const HISTORY: &str = "history first\nhistory last";

    // Terminal conversion belongs here, never in the client-neutral table.
    fn terminal_chord(chord: KeyChord) -> KeyEvent {
        let code = match chord.key {
            NamedKey::Char(character) => KeyCode::Char(character),
            NamedKey::Enter => KeyCode::Enter,
            NamedKey::Tab => KeyCode::Tab,
            NamedKey::BackTab => KeyCode::BackTab,
            NamedKey::Escape => KeyCode::Esc,
            NamedKey::Up => KeyCode::Up,
            NamedKey::Down => KeyCode::Down,
            NamedKey::Left => KeyCode::Left,
            NamedKey::Right => KeyCode::Right,
            NamedKey::Delete => KeyCode::Delete,
            NamedKey::Home => KeyCode::Home,
            NamedKey::End => KeyCode::End,
            NamedKey::PageUp => KeyCode::PageUp,
            NamedKey::PageDown => KeyCode::PageDown,
        };
        let modifiers = match chord.modifier {
            Modifier::None => KeyModifiers::NONE,
            Modifier::Control => KeyModifiers::CONTROL,
            Modifier::Alt => KeyModifiers::ALT,
            Modifier::Shift => KeyModifiers::SHIFT,
        };
        KeyEvent::new(code, modifiers)
    }

    fn terminal_inputs(input: KeyInput) -> Vec<TerminalInput> {
        match input {
            KeyInput::Chord(chord) => vec![TerminalInput::Key(terminal_chord(chord))],
            KeyInput::Text(text) => text
                .chars()
                .map(|character| TerminalInput::Key(key(KeyCode::Char(character))))
                .collect(),
            KeyInput::MouseWheel(direction) => vec![TerminalInput::Mouse(mouse(
                match direction {
                    Direction::Previous => MouseEventKind::ScrollUp,
                    Direction::Next => MouseEventKind::ScrollDown,
                },
                0,
                0,
            ))],
            KeyInput::Sequence(inputs) => {
                inputs.iter().copied().flat_map(terminal_inputs).collect()
            }
        }
    }

    struct Fixture {
        app: App,
        approval: Option<tokio::task::JoinHandle<ApprovalDecision>>,
    }

    async fn fixture(context: KeyContext, working: bool) -> Fixture {
        let mut app = agent_first_app();
        if working {
            app.apply(&turn_event("keymap-turn", RuntimeEvent::TurnStarted));
        }
        app.sync_scroll_limit(40);
        app.scroll_up(10);
        app.composer.replace(TASK);
        let mut approval = None;
        match context {
            KeyContext::Idle | KeyContext::PausedOutput => {}
            KeyContext::Working | KeyContext::ForegroundShell => {
                if !working {
                    app.apply(&turn_event("keymap-turn", RuntimeEvent::TurnStarted));
                }
                if context == KeyContext::ForegroundShell {
                    // The app sees the running call; process adoption belongs to the host.
                    app.apply(&event(tool_requested("keymap-shell", "shell")));
                }
            }
            KeyContext::EmptyIdleDraft | KeyContext::EmptyDraft | KeyContext::EmptyPausedOutput => {
                app.composer.clear();
            }
            KeyContext::AnyComposer | KeyContext::DraftLine => {
                app.composer.replace(DRAFT);
                app.composer.move_to_position(1, 4);
            }
            KeyContext::SlashDraft => {
                app.composer.replace(format!("/{DRAFT}"));
                app.composer.move_to_position(1, 4);
                app.overlay = Some(Overlay::Palette {
                    selected: 2,
                    error: Some("previous refusal".to_owned()),
                    restore_on_escape: None,
                });
            }
            KeyContext::DraftFirstLine | KeyContext::DraftLastLine => {
                app.composer.replace(HISTORY);
                app.composer.record_current();
                app.composer.replace("scratch\nsecond");
                if context == KeyContext::DraftFirstLine {
                    app.composer.move_to_position(0, 3);
                } else {
                    assert!(app.composer.recall_previous());
                }
            }
            KeyContext::DelegatedAgents | KeyContext::InspectedAgent => {
                app.composer.clear();
                app.restore_child("child-1", ChildState::Idle, None);
                app.restore_child("child-2", ChildState::Idle, None);
                if context == KeyContext::InspectedAgent {
                    app.inspect_child("child-2");
                }
            }
            KeyContext::QueuedTurn => {
                if !working {
                    app.apply(&turn_event("keymap-turn", RuntimeEvent::TurnStarted));
                }
                for draft in ["older queued task", "newest queued task"] {
                    app.composer.replace(draft);
                    app.on_key(key(KeyCode::Tab));
                }
            }
            KeyContext::Approval => {
                prompt_clock(&mut app);
                let (prompt, pending) =
                    pending_prompt_with("shell", serde_json::json!({"command": "git status"}))
                        .await;
                app.present_approval(prompt);
                elapse_prompt_guard(&mut app);
                approval = Some(pending);
            }
            KeyContext::Confirm => {
                prompt_clock(&mut app);
                app.confirm_undo(recovery_preview("preview"));
                elapse_prompt_guard(&mut app);
            }
            KeyContext::CommandPalette => {
                app.on_key(ctrl('p'));
            }
            KeyContext::HistorySearch => {
                app.composer.replace("history match");
                app.composer.record_current();
                app.composer.replace(TASK);
                app.on_key(ctrl('r'));
                type_text(&mut app, "history");
            }
            KeyContext::TokenBoundary => {
                app.composer.clear();
            }
            KeyContext::ShellCommand => {
                app.composer.replace("printf keymap");
                app.composer.move_to_start();
            }
        }
        Fixture { app, approval }
    }

    struct Before {
        text: String,
        cursor: usize,
        position: (usize, usize),
        scroll: usize,
        detail: bool,
        activity: crate::status::Activity,
        overlay: Option<std::mem::Discriminant<Overlay>>,
        inspected: Option<String>,
    }

    impl Before {
        fn capture(app: &App) -> Self {
            Self {
                text: app.composer.text().to_owned(),
                cursor: app.composer.cursor(),
                position: app.composer.cursor_position(),
                scroll: app.scroll_back,
                detail: app.work_details,
                activity: app.status.activity,
                overlay: app.overlay.as_ref().map(std::mem::discriminant),
                inspected: app.inspected_child.clone(),
            }
        }
    }

    // Keep this exhaustive: adding an effect must also add its behavioral check.
    fn observed(case: &BindingCase, before: &Before, app: &App, action: Option<&Action>) -> bool {
        let slash = if case.context == KeyContext::SlashDraft {
            "/"
        } else {
            ""
        };
        match case.effect {
            KeyEffect::SendTask => {
                matches!(action, Some(Action::Submit { submission, target: SubmissionTarget::WholeTurn })
                if submission.committed_text() == before.text && app.composer.is_empty())
            }
            KeyEffect::Steer => {
                matches!(action, Some(Action::Submit { submission, target: SubmissionTarget::Steer { expected_turn } })
                if submission.committed_text() == before.text && expected_turn.as_ref() == app.active_turn()
                    && expected_turn.as_ref().is_some_and(|turn| turn.as_str() == "keymap-turn") && app.composer.is_empty())
            }
            KeyEffect::QueueTurn => {
                action.is_none()
                    && app.composer.is_empty()
                    && app.pending_input.queued_turns.len() == 1
                    && app
                        .pending_input
                        .queued_turns
                        .front()
                        .is_some_and(|turn| turn.committed_text() == before.text)
            }
            KeyEffect::NextProfile => {
                action
                    == Some(&Action::Reconfigure(SessionControl::Reconfigure(
                        SelectionCommand::Profile("plan".to_owned()),
                    )))
            }
            KeyEffect::PreviousProfile => {
                action
                    == Some(&Action::Reconfigure(SessionControl::Reconfigure(
                        SelectionCommand::Profile("review".to_owned()),
                    )))
            }
            KeyEffect::ToggleDetail => {
                action.is_none()
                    && app.work_details != before.detail
                    && app.composer.text() == before.text
                    && app.overlay.as_ref().map(std::mem::discriminant) == before.overlay
            }
            KeyEffect::InterruptOrClose => match case.context {
                KeyContext::Working => {
                    action == Some(&Action::Interrupt)
                        && app.status.activity == crate::status::Activity::Interrupting
                        && app.composer.text() == before.text
                }
                KeyContext::Idle => {
                    action.is_none() && app.composer.is_empty() && app.overlay.is_none()
                }
                KeyContext::CommandPalette => {
                    action.is_none() && app.overlay.is_none() && app.composer.text() == TASK
                }
                KeyContext::Approval => {
                    action.is_none() && app.overlay.is_none() && app.pending_approval_count() == 0
                }
                KeyContext::Confirm => action == Some(&Action::CancelUndo) && app.overlay.is_none(),
                KeyContext::InspectedAgent => {
                    action.is_none()
                        && app.inspected_child.is_none()
                        && app.status.activity == before.activity
                }
                _ => false,
            },
            KeyEffect::InsertNewline => {
                let mut expected = before.text.chars().collect::<Vec<_>>();
                expected.insert(before.cursor, '\n');
                action.is_none()
                    && app.composer.text() == expected.into_iter().collect::<String>()
                    && app.composer.cursor() == before.cursor + 1
            }
            KeyEffect::MoveDraftLine(direction) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && app.composer.cursor_position()
                        == match direction {
                            Direction::Previous => (before.position.0 - 1, before.position.1),
                            Direction::Next => (before.position.0 + 1, before.position.1),
                        }
            }
            KeyEffect::BrowseHistory(direction) => {
                action.is_none()
                    && app.composer.text()
                        == match direction {
                            Direction::Previous => HISTORY,
                            Direction::Next => "scratch\nsecond",
                        }
                    && app.inspected_child.is_none()
            }
            KeyEffect::BrowseAgents(direction) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && app.inspected_child.as_deref() == Some("child-1")
                    && before.inspected.as_deref()
                        == match direction {
                            Direction::Previous => Some("child-2"),
                            Direction::Next => None,
                        }
            }
            KeyEffect::DraftEdge(edge) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && app.scroll_back == before.scroll
                    && app.composer.cursor()
                        == match edge {
                            Edge::Start => 0,
                            Edge::End => app.composer.len(),
                        }
            }
            KeyEffect::OutputEdge(edge) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && match edge {
                        Edge::Start => app.scroll_back == app.scroll_limit && !app.following,
                        Edge::End => app.scroll_back == 0 && app.following,
                    }
            }
            KeyEffect::LineEdge(edge) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && app.composer.cursor_position()
                        == (
                            1,
                            match edge {
                                Edge::Start => 0,
                                Edge::End => 12,
                            },
                        )
            }
            KeyEffect::MoveWord(direction) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && app.composer.cursor_position()
                        == (
                            1,
                            match direction {
                                Direction::Previous => 0,
                                Direction::Next => 7,
                            },
                        )
            }
            KeyEffect::MoveCharacter(direction) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && app.composer.cursor()
                        == match direction {
                            Direction::Previous => before.cursor - 1,
                            Direction::Next => before.cursor + 1,
                        }
            }
            KeyEffect::DeleteCharacter => {
                let mut expected = before.text.chars().collect::<Vec<_>>();
                expected.remove(before.cursor);
                action.is_none()
                    && app.composer.text() == expected.into_iter().collect::<String>()
                    && app.composer.cursor() == before.cursor
            }
            KeyEffect::DeleteWordLeft => {
                action.is_none()
                    && app.composer.text() == format!("{slash}first\ntwo tail\nlast")
                    && app.composer.cursor_position() == (1, 0)
            }
            KeyEffect::DeleteToLineStart => {
                action.is_none()
                    && app.composer.text() == format!("{slash}first\ntwo tail\nlast")
                    && app.composer.cursor_position() == (1, 0)
            }
            KeyEffect::DeleteToLineEnd => {
                action.is_none()
                    && app.composer.text() == format!("{slash}first\none \nlast")
                    && app.composer.cursor_position() == before.position
            }
            KeyEffect::EditQueuedTurn => {
                action.is_none()
                    && app.composer.text() == "newest queued task"
                    && app.pending_input.queued_turns.len() == 1
                    && app
                        .pending_input
                        .queued_turns
                        .front()
                        .is_some_and(|turn| turn.display_text() == "older queued task")
            }
            KeyEffect::BackgroundShell => {
                action == Some(&Action::BackgroundShell) && app.composer.text() == before.text
            }
            KeyEffect::OpenCommandPalette => {
                action.is_none()
                    && app.composer.text() == "/"
                    && matches!(&app.overlay, Some(Overlay::Palette { restore_on_escape: Some(original), .. }) if original == &before.text)
            }
            KeyEffect::ShowShortcuts => {
                action.is_none()
                    && app.composer.is_empty()
                    && matches!(app.overlay, Some(Overlay::Shortcuts))
            }
            KeyEffect::ScrollTranscript(direction) => {
                action.is_none()
                    && app.composer.text() == before.text
                    && app.overlay.as_ref().map(std::mem::discriminant) == before.overlay
                    && match direction {
                        Direction::Previous => app.scroll_back > before.scroll && !app.following,
                        Direction::Next => app.scroll_back < before.scroll,
                    }
            }
            KeyEffect::FollowNewest => {
                action.is_none()
                    && app.scroll_back == 0
                    && app.following
                    && app.composer.text() == before.text
                    && app.overlay.as_ref().map(std::mem::discriminant) == before.overlay
            }
            KeyEffect::SearchHistory => {
                action.is_none()
                    && app.composer.text() == before.text
                    && matches!(&app.overlay, Some(Overlay::HistorySearch { original, .. }) if original == &before.text)
            }
            KeyEffect::RestoreHistoryMatch => {
                action.is_none() && app.overlay.is_none() && app.composer.text() == "history match"
            }
            KeyEffect::CloseHistorySearch => {
                action.is_none() && app.overlay.is_none() && app.composer.text() == TASK
            }
            KeyEffect::StashDraft => {
                action.is_none()
                    && app.composer.is_empty()
                    && app
                        .composer
                        .search_history(&before.text, None)
                        .is_some_and(|(_, entry)| entry == before.text)
                    && app.overlay.as_ref().map(std::mem::discriminant) == before.overlay
            }
            KeyEffect::Quit => {
                action == Some(&Action::Quit) && app.should_quit && app.overlay.is_none()
            }
            KeyEffect::CompleteReferences => {
                action.is_none()
                    && matches!(&app.overlay,
                Some(Overlay::ResourcePicker { target: ResourceTarget::Reference, picker, .. })
                    if picker.entries.iter().any(|entry| entry.id == "file:src/lib.rs")
                        && picker.entries.iter().any(|entry| entry.id == "agent:review"))
            }
            KeyEffect::SendLiteral(character) => matches!(action,
                Some(Action::Submit { submission, target: SubmissionTarget::WholeTurn })
                    if submission.committed_text() == character.to_string() && app.composer.is_empty()),
            KeyEffect::ShellMode => {
                action.is_none() && app.composer.is_bash_mode() && app.composer.text() == "!"
            }
            KeyEffect::RunShell => {
                matches!(action, Some(Action::RunShell { command }) if command == "printf keymap" && app.composer.is_empty())
            }
        }
    }

    #[tokio::test]
    async fn every_binding_matches_current_key_handling() {
        for binding in KEY_BINDINGS {
            assert!(!binding.cases.is_empty(), "{} has no cases", binding.label);
            for case in binding.cases {
                let working_states: &[bool] = match case.context {
                    KeyContext::AnyComposer
                    | KeyContext::EmptyDraft
                    | KeyContext::DraftLine
                    | KeyContext::SlashDraft
                    | KeyContext::DraftFirstLine
                    | KeyContext::DraftLastLine
                    | KeyContext::DelegatedAgents
                    | KeyContext::InspectedAgent
                    | KeyContext::PausedOutput
                    | KeyContext::EmptyPausedOutput
                    | KeyContext::Approval
                    | KeyContext::Confirm
                    | KeyContext::CommandPalette
                    | KeyContext::HistorySearch
                    | KeyContext::TokenBoundary => &[false, true],
                    _ => &[false],
                };
                for &working in working_states {
                    let Fixture { mut app, approval } = fixture(case.context, working).await;
                    let before = Before::capture(&app);
                    let events = terminal_inputs(case.input);
                    assert!(!events.is_empty(), "{} has no input", binding.label);
                    let mut action = None;
                    for event in events {
                        match event {
                            TerminalInput::Key(key) => {
                                let next = app.on_key(key);
                                assert!(
                                    action.is_none(),
                                    "{} acted before its sequence ended",
                                    binding.label
                                );
                                action = next;
                            }
                            TerminalInput::Mouse(mouse) => assert_eq!(
                                app.on_mouse(mouse),
                                MouseOutcome::Redraw,
                                "{}: {case:?}",
                                binding.label,
                            ),
                            other => panic!("{}: unexpected input {other:?}", binding.label),
                        }
                    }
                    assert!(
                        observed(case, &before, &app, action.as_ref()),
                        "{}: {case:?}, working={working}, action={action:?}, draft={:?}, scroll={}, overlay={:?}",
                        binding.label,
                        app.composer.text(),
                        app.scroll_back,
                        app.overlay
                    );
                    if case.context == KeyContext::SlashDraft {
                        assert!(app.composer.text().starts_with('/'));
                        assert!(
                            matches!(
                                app.overlay,
                                Some(Overlay::Palette {
                                    selected: 0,
                                    error: None,
                                    ..
                                })
                            ),
                            "{} did not refresh the palette: {:?}",
                            binding.label,
                            app.overlay
                        );
                    }
                    if let Some(approval) = approval {
                        if case.effect != KeyEffect::InterruptOrClose
                            && case.effect != KeyEffect::Quit
                        {
                            assert!(
                                !approval.is_finished(),
                                "{} answered the approval",
                                binding.label
                            );
                            app.on_key(ctrl('c'));
                            app.on_key(ctrl('c'));
                        }
                        let expected_denial = case.effect == KeyEffect::InterruptOrClose;
                        let decision = approval.await.expect("approval decision");
                        assert!(
                            if expected_denial {
                                matches!(decision, ApprovalDecision::Deny { .. })
                            } else {
                                decision == ApprovalDecision::Cancelled
                            },
                            "{}: approval {decision:?}",
                            binding.label
                        );
                    }
                }
            }
        }
    }

    fn footer_binding(
        app: &mut App,
        width: u16,
        name: &str,
        chord: KeyChord,
        context: KeyContext,
        effect: KeyEffect,
    ) {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 40))
            .expect("test terminal");
        terminal
            .draw(|frame| crate::render::draw(frame, app, crate::theme::Theme::new()))
            .expect("footer frame");
        let buffer = terminal.backend().buffer();
        let footer = (0..width)
            .map(|column| buffer[(column, 39)].symbol())
            .collect::<String>();
        assert!(footer.contains(name), "missing {name}: {footer}");
        assert!(
            KEY_BINDINGS
                .iter()
                .flat_map(|binding| binding.cases)
                .any(|case| case.context == context
                    && case.input == KeyInput::Chord(chord)
                    && case.effect == effect),
            "footer {name} has no binding in {context:?} for {effect:?}"
        );
    }

    #[tokio::test]
    async fn footer_hint_key_names_have_bindings_in_the_matching_context() {
        use KeyContext::{
            AnyComposer, EmptyDraft, EmptyPausedOutput, PausedOutput, QueuedTurn, Working,
        };
        use KeyEffect::{
            DraftEdge, EditQueuedTurn, FollowNewest, InterruptOrClose, OutputEdge, QueueTurn,
            ShowShortcuts, Steer,
        };
        let mut app = fixture(EmptyDraft, false).await.app;
        app.follow_newest();
        footer_binding(
            &mut app,
            160,
            "?",
            KeyChord::new(NamedKey::Char('?'), Modifier::None),
            EmptyDraft,
            ShowShortcuts,
        );

        app.scroll_up(10);
        footer_binding(
            &mut app,
            160,
            "End",
            KeyChord::new(NamedKey::End, Modifier::None),
            EmptyPausedOutput,
            OutputEdge(Edge::End),
        );
        footer_binding(
            &mut app,
            160,
            "Ctrl+L",
            KeyChord::new(NamedKey::Char('l'), Modifier::Control),
            PausedOutput,
            FollowNewest,
        );

        // End retains its draft meaning when this same hint appears with text.
        app = fixture(AnyComposer, false).await.app;
        footer_binding(
            &mut app,
            160,
            "End",
            KeyChord::new(NamedKey::End, Modifier::None),
            AnyComposer,
            DraftEdge(Edge::End),
        );

        app = fixture(QueuedTurn, true).await.app;
        app.composer.replace(TASK);
        for paused in [false, true] {
            app.follow_newest();
            if paused {
                app.scroll_up(10);
            }
            for width in [44, 80, 160] {
                footer_binding(
                    &mut app,
                    width,
                    "enter",
                    KeyChord::new(NamedKey::Enter, Modifier::None),
                    Working,
                    Steer,
                );
                footer_binding(
                    &mut app,
                    width,
                    "tab",
                    KeyChord::new(NamedKey::Tab, Modifier::None),
                    Working,
                    QueueTurn,
                );
                footer_binding(
                    &mut app,
                    width,
                    "esc",
                    KeyChord::new(NamedKey::Escape, Modifier::None),
                    Working,
                    InterruptOrClose,
                );
            }
            if paused {
                footer_binding(
                    &mut app,
                    44,
                    "ctrl+l",
                    KeyChord::new(NamedKey::Char('l'), Modifier::Control),
                    PausedOutput,
                    FollowNewest,
                );
            } else {
                footer_binding(
                    &mut app,
                    160,
                    "alt+↑",
                    KeyChord::new(NamedKey::Up, Modifier::Alt),
                    QueuedTurn,
                    EditQueuedTurn,
                );
            }
        }
    }
}
