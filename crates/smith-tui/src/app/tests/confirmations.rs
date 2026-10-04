// Shared confirmation presentation, scrolling, decisions, and overlay policy.

struct ConfirmationCase {
    title: &'static str,
    tone: Tone,
    warning: Option<&'static str>,
    label: &'static str,
    accept: Option<Action>,
    cancel: Option<Action>,
}

fn confirmation_cases() -> Vec<ConfirmationCase> {
    let reverse = Some("No action is selected by default. Review the complete reverse patch.");
    vec![
        ConfirmationCase {
            title: "undo last Smith turn",
            tone: Tone::Warning,
            warning: reverse,
            label: "apply undo",
            accept: Some(Action::ApplyUndo),
            cancel: Some(Action::CancelUndo),
        },
        ConfirmationCase {
            title: "redo last exact Smith turn",
            tone: Tone::Warning,
            warning: Some("No action is selected by default. Review the complete forward patch."),
            label: "apply redo",
            accept: Some(Action::ApplyRedo),
            cancel: Some(Action::CancelRedo),
        },
        ConfirmationCase {
            title: "revert selected change",
            tone: Tone::Warning,
            warning: reverse,
            label: "apply revert",
            accept: Some(Action::ApplyRevert {
                scope: "file.txt".to_owned(),
                fingerprint: "exact".to_owned(),
            }),
            cancel: Some(Action::CancelRevert {
                scope: "file.txt".to_owned(),
                fingerprint: "exact".to_owned(),
            }),
        },
        ConfirmationCase {
            title: "trust this MCP server",
            tone: Tone::Warning,
            warning: Some("Trust MCP server docs? No action is selected by default."),
            label: "trust and connect",
            accept: Some(Action::TrustMcpServer {
                server: "docs".to_owned(),
            }),
            cancel: None,
        },
        ConfirmationCase {
            title: "trust this project skill",
            tone: Tone::Warning,
            warning: Some("Trust project skill deploy? No action is selected by default."),
            label: "trust and activate",
            accept: Some(Action::TrustSkill {
                skill: "deploy".to_owned(),
            }),
            cancel: None,
        },
        ConfirmationCase {
            title: "read-only review",
            tone: Tone::Accent,
            warning: None,
            label: "start provider-backed review",
            accept: Some(Action::StartReview {
                scope: "all".to_owned(),
            }),
            cancel: None,
        },
        ConfirmationCase {
            title: "read-only child agent",
            tone: Tone::Accent,
            warning: None,
            label: "start child and spend provider tokens",
            accept: Some(Action::StartAgent {
                preset: "review".to_owned(),
                task: "inspect the diff".to_owned(),
            }),
            cancel: None,
        },
        ConfirmationCase {
            title: "existing child follow-up",
            tone: Tone::Accent,
            warning: None,
            label: "start follow-up and spend provider tokens",
            accept: Some(Action::FollowUpAgent {
                child_id: "child-1".to_owned(),
                task: "inspect the diff".to_owned(),
            }),
            cancel: None,
        },
        ConfirmationCase {
            title: "resume interrupted child",
            tone: Tone::Accent,
            warning: None,
            label: "resume exact checkpoint",
            accept: Some(Action::ResumeAgent {
                child_id: "child-1".to_owned(),
            }),
            cancel: None,
        },
        ConfirmationCase {
            title: "switch provider account",
            tone: Tone::Warning,
            warning: None,
            label: "switch account and resend",
            accept: None,
            cancel: None,
        },
        ConfirmationCase {
            title: "exit",
            tone: Tone::Warning,
            warning: Some("quit with work in progress?"),
            label: "quit",
            accept: Some(Action::Quit),
            cancel: None,
        },
    ]
}

async fn open_confirmation_case(
    app: &mut App,
    index: usize,
    body: &str,
) -> Option<tokio::task::JoinHandle<RotationDecision>> {
    match index {
        0 => app.confirm_undo(recovery_preview(body)),
        1 => app.confirm_redo(recovery_preview(body)),
        2 => app.confirm_revert(revert_preview("file.txt", "exact", body)),
        3 => app.confirm_mcp_trust("docs", body),
        4 => app.confirm_skill_trust("deploy", body),
        5 => app.confirm_review(smith_client::review_report::ReviewPreview {
            scope: "all".to_owned(),
            title: "diff · all uncommitted".to_owned(),
            patch: recovery_preview(body).patch,
        }),
        6 => {
            app.composer.replace("@review inspect the diff");
            app.on_key(key(KeyCode::Enter));
        }
        7 => {
            app.restore_child("child-1", ChildState::Idle, None);
            app.composer.replace("@child-1 inspect the diff");
            app.on_key(key(KeyCode::Enter));
        }
        8 => {
            app.restore_child("child-1", ChildState::Interrupted { resumable: true }, None);
            app.composer.replace("/agent resume child-1");
            app.on_key(key(KeyCode::Enter));
        }
        9 => {
            let (policy, mut requests) = InteractiveRotation::new(1);
            let request = offer(vec![member(1, "keychain:smith/work")]);
            let pending = tokio::spawn(async move { policy.decide(&request).await });
            app.present_rotation(requests.recv().await.expect("rotation offer"));
            return Some(pending);
        }
        10 => {
            app.status.activity = crate::status::Activity::Working;
            app.request_exit();
        }
        _ => unreachable!("enumerated confirmation kind"),
    }
    None
}

fn confirmation_screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("test terminal");
    terminal
        .draw(|frame| {
            crate::render::draw_synced(frame, app, crate::theme::Theme::new().without_color())
        })
        .expect("frame");
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn every_confirmation_preserves_its_labels_scrolls_and_has_guarded_decisions() {
    let long_body = (0..200)
        .map(|index| format!("body line {index:03}\n"))
        .collect::<String>();
    for (index, case) in confirmation_cases().into_iter().enumerate() {
        for decision in [KeyCode::Char('y'), KeyCode::Char('n'), KeyCode::Esc] {
            let mut app = agent_first_app();
            let clock = prompt_clock(&mut app);
            let rotation = open_confirmation_case(&mut app, index, &long_body).await;
            let Some(Overlay::Confirm(dialog)) = &mut app.overlay else {
                panic!("{} must open", case.title)
            };
            assert_eq!(dialog.title, case.title);
            assert_eq!(dialog.tone, case.tone);
            assert_eq!(
                dialog.warning.as_ref().map(|(text, _)| text.as_str()),
                case.warning
            );
            assert_eq!(dialog.accept_label, case.label);
            assert_eq!(
                dialog.accept_tone,
                match index {
                    0..=2 | 10 => Tone::Danger,
                    3 | 4 => Tone::Warning,
                    _ => Tone::Accent,
                }
            );
            assert_eq!(
                dialog.cancel_label,
                match index {
                    3 => "leave untrusted",
                    4 => "leave withheld",
                    9 => "stay",
                    10 => "keep working",
                    _ => "cancel",
                }
            );
            assert_eq!(dialog.cancel_key, if index == 10 { "n" } else { "n/esc" });
            assert_ne!(dialog.body.last().map(String::as_str), Some(""));
            // The child, exit, and rotation sources have short authored bodies;
            // exercise the same viewport with long content for these too.
            if index >= 6 {
                dialog.body.extend(long_body.lines().map(str::to_owned));
            }
            assert_eq!(
                dialog.body.last().map(String::as_str),
                Some("body line 199")
            );
            let first = confirmation_screen(&mut app, 100, 40);
            assert!(first.contains(case.title), "{first}");
            assert!(first.contains(case.label), "{first}");
            if let Some(warning) = case.warning {
                let flattened = first
                    .replace('│', " ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                assert!(flattened.contains(warning), "{first}");
            }
            assert!(first.contains("↑↓ review"), "{first}");
            assert!(!first.contains("body line 199"), "{first}");

            clock.advance(499);
            assert_eq!(app.on_key(key(decision)), None, "{} guard", case.title);
            assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
            clock.advance(499);
            assert_eq!(app.on_key(key(KeyCode::Enter)), None);
            assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
            app.on_key(key(KeyCode::Down));
            assert!(matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.scroll == 1));
            app.on_key(key(KeyCode::Up));
            assert!(matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.scroll == 0));
            app.on_key(key(KeyCode::PageDown));
            assert!(matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.scroll == 10));
            app.on_key(key(KeyCode::PageUp));
            assert!(matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.scroll == 0));
            for _ in 0..30 {
                app.on_key(key(KeyCode::PageDown));
            }
            let last = confirmation_screen(&mut app, 100, 40);
            assert!(last.contains("body line 199"), "{}: {last}", case.title);
            assert!(last.contains(case.label), "{last}");
            assert!(
                matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.scroll == dialog.scroll_limit)
            );
            // Resize while scrolled and retain a reachable final wrapped row.
            confirmation_screen(&mut app, 40, 10);
            for _ in 0..30 {
                app.on_key(key(KeyCode::PageDown));
            }
            let narrow = confirmation_screen(&mut app, 40, 10);
            assert!(narrow.contains("body line 199"), "{}: {narrow}", case.title);

            clock.advance(500);
            assert_eq!(
                app.on_key(key(KeyCode::Enter)),
                None,
                "{} Enter",
                case.title
            );
            assert!(matches!(app.overlay, Some(Overlay::Confirm(_))));
            let expected = if decision == KeyCode::Char('y') {
                case.accept.clone()
            } else {
                case.cancel.clone()
            };
            assert_eq!(
                app.on_key(key(decision)),
                expected,
                "{} decision",
                case.title
            );
            assert!(app.overlay.is_none());
            assert_eq!(
                app.should_quit,
                index == 10 && decision == KeyCode::Char('y')
            );
            if let Some(rotation) = rotation {
                assert_eq!(
                    rotation.await.expect("rotation decision"),
                    if decision == KeyCode::Char('y') {
                        RotationDecision::Switch { position: 1 }
                    } else {
                        RotationDecision::Decline
                    }
                );
            }
        }
    }
}

#[tokio::test]
async fn approval_confirmation_questionnaire_prompts_share_one_fifo() {
    let mut app = app();
    let clock = prompt_clock(&mut app);
    let (approval, decision) =
        pending_prompt_with("shell", serde_json::json!({"command": "build"})).await;
    app.present_approval(approval);
    let guard = app.prompt_input_guard.quiet_until;
    let rotation = open_confirmation_case(&mut app, 9, "")
        .await
        .expect("pending rotation");
    app.present_questionnaire(questionnaire_form("last-question", Deadline::never()));
    assert_eq!(app.queued_prompt_count(), 2);
    assert_eq!(
        app.prompt_input_guard.quiet_until, guard,
        "a queued prompt cannot restart the visible prompt's guard"
    );
    assert!(matches!(app.overlay, Some(Overlay::Approval { .. })));
    assert!(!rotation.is_finished());

    clock.advance(500);
    app.on_key(key(KeyCode::Char('n')));
    assert!(
        matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.title == "switch provider account")
    );
    assert_eq!(app.queued_prompt_count(), 1);
    assert!(matches!(
        decision.await.expect("approval decision"),
        ApprovalDecision::Deny { .. }
    ));
    assert_eq!(app.on_key(key(KeyCode::Char('y'))), None);
    assert!(!rotation.is_finished());
    clock.advance(500);
    app.on_key(key(KeyCode::Char('n')));
    assert_eq!(
        rotation.await.expect("rotation decision"),
        RotationDecision::Decline
    );
    assert!(
        matches!(&app.overlay, Some(Overlay::Questionnaire { state }) if state.form().request_id == "last-question")
    );
    app.on_key(key(KeyCode::Esc));
    assert!(
        matches!(app.take_questionnaire_resolution(), Some((id, QuestionnaireResolution::Cancelled)) if id == "last-question")
    );
    assert!(app.overlay.is_none());
}

fn transient_overlay(index: usize) -> Overlay {
    match index {
        0 => Overlay::Palette {
            selected: 0,
            error: None,
            restore_on_escape: None,
        },
        1 => Overlay::ResourcePicker {
            picker: ResourcePicker::new("Choose model", Vec::new(), "setup"),
            target: ResourceTarget::Model,
            restore_on_escape: "draft".to_owned(),
        },
        2 => Overlay::HistorySearch {
            original: "draft".to_owned(),
            query: String::new(),
            selected: None,
            matched: None,
        },
        3 => Overlay::Shortcuts,
        _ => unreachable!("transient kind"),
    }
}

#[tokio::test]
async fn every_prompt_closes_transients_and_blocks_new_transients() {
    for transient in 0..4 {
        for prompt_kind in 0..3 {
            let mut app = app();
            assert!(app.open_overlay(transient_overlay(transient)));
            match prompt_kind {
                0 => app.present_approval(prompt("shell").await),
                1 => app.present_questionnaire(questionnaire_form("visible", Deadline::never())),
                2 => app.confirm_mcp_trust("docs", "command: docs-server"),
                _ => unreachable!("prompt kind"),
            }
            assert!(app.overlay.as_ref().is_some_and(Overlay::is_prompt));
            for blocked in 0..4 {
                assert!(!app.open_overlay(transient_overlay(blocked)));
                assert!(app.overlay.as_ref().is_some_and(Overlay::is_prompt));
            }
            app.confirm_undo(recovery_preview("queued patch"));
            app.overlay = None;
            assert!(
                !app.open_overlay(transient_overlay(transient)),
                "a queued prompt owns the next slot too"
            );
            app.present_next_prompt();
            assert!(
                matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.title == "undo last Smith turn")
            );
        }
    }
}

#[test]
fn confirmation_decisions_present_the_next_prompt() {
    for decision in [KeyCode::Char('y'), KeyCode::Char('n'), KeyCode::Esc] {
        let mut app = app();
        app.confirm_undo(recovery_preview("reverse patch"));
        app.present_questionnaire(questionnaire_form("next", Deadline::never()));
        elapse_prompt_guard(&mut app);
        assert_eq!(
            app.on_key(key(decision)),
            Some(if decision == KeyCode::Char('y') {
                Action::ApplyUndo
            } else {
                Action::CancelUndo
            })
        );
        assert!(
            matches!(&app.overlay, Some(Overlay::Questionnaire { state })
            if state.form().request_id == "next")
        );
        assert_eq!(app.queued_prompt_count(), 0);
    }
}

#[tokio::test]
async fn exit_confirmation_retains_the_approval_and_questionnaire_lines() {
    let mut app = app();
    app.present_approval(prompt("shell").await);
    app.present_questionnaire(questionnaire_form("question", Deadline::never()));
    app.request_exit();
    elapse_prompt_guard(&mut app);
    app.on_key(key(KeyCode::Char('n')));
    app.on_key(key(KeyCode::Esc));
    let screen = confirmation_screen(&mut app, 100, 40);
    assert!(screen.contains("quit with work in progress?"), "{screen}");
    assert!(screen.contains("· an approval is pending"), "{screen}");
    assert!(screen.contains("· a questionnaire is pending"), "{screen}");
    assert!(screen.contains("y quit   n keep working"), "{screen}");
}

#[test]
fn a_user_requested_picker_uses_the_existing_idle_feedback_without_replacing_a_prompt() {
    let mut app = app();
    app.confirm_mcp_trust("docs", "command: docs-server");
    app.composer.replace("/model");
    assert_eq!(
        app.dispatch_command(commands::parse("/model").expect("model command")),
        None
    );
    assert!(
        matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.title == "trust this MCP server")
    );
    assert_eq!(app.composer.text(), "/model");
    assert_feedback_hint(&app, "/model requires an idle turn; draft preserved");
}

#[test]
fn a_blocked_transient_request_reports_feedback_and_keeps_the_prompt_queue() {
    let mut app = app();
    app.confirm_undo(recovery_preview("preview"));
    app.present_questionnaire(questionnaire_form("queued", Deadline::never()));
    assert!(!app.open_overlay(transient_overlay(0)));
    assert_feedback_hint(
        &app,
        "answer the pending prompt before opening another panel",
    );
    assert!(app.transcript.is_empty());
    assert!(
        matches!(&app.overlay, Some(Overlay::Confirm(dialog)) if dialog.title == "undo last Smith turn")
    );
    assert_eq!(app.queued_prompt_count(), 1);
    app.on_key(key(KeyCode::Char('x')));
    assert!(app.feedback_notice().is_none());
    assert_eq!(app.queued_prompt_count(), 1);
}
