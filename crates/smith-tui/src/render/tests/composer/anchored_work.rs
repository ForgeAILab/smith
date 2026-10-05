use super::*;

#[test]
fn working_progress_is_one_fixed_row_outside_the_scrolled_transcript() {
    for (width, height) in [(44, 16), (80, 24), (100, 32), (40, 10)] {
        let theme = Theme::new().without_color().without_motion();
        let mut app = App::new("model", "project");
        app.apply(&event(RuntimeEvent::TurnStarted));
        let before = render_synced(&mut app, width, height, theme);
        let progress_row = before
            .lines()
            .position(|line| line.contains("Working… ("))
            .unwrap();
        let composer_row = before
            .lines()
            .position(|line| line.contains("Ask Smith"))
            .unwrap();
        assert_eq!(progress_row + 2, composer_row, "{before}");
        assert_eq!(
            before.lines().nth(progress_row + 1).unwrap(),
            "─".repeat(usize::from(width))
        );

        for index in 0..40 {
            app.transcript
                .push_notice(NoticeKind::Monitor, format!("notice {index}"));
        }
        app.composer.replace("keep this draft");
        let appended = render_synced(&mut app, width, height, theme);
        assert_eq!(
            appended
                .lines()
                .position(|line| line.contains("Working… (")),
            Some(progress_row)
        );
        assert_eq!(appended.matches("Working… (").count(), 1, "{appended}");
        assert!(
            appended
                .lines()
                .nth(composer_row)
                .unwrap()
                .contains("keep this draft")
        );
        assert!(
            visual_scroll_limit(
                &transcript_lines(&app, theme, width),
                transcript_rect(Rect::new(0, 0, width, height), &app)
            ) > 0
        );

        app.following = false;
        app.scroll_back = usize::from(u16::MAX);
        let scrolled = render_synced(&mut app, width, height, theme);
        assert!(scrolled.contains("notice 0"), "{scrolled}");
        assert_eq!(
            scrolled
                .lines()
                .position(|line| line.contains("Working… (")),
            Some(progress_row)
        );
        assert!(
            transcript_lines(&app, theme, width)
                .iter()
                .all(|line| !line.to_string().contains("Working"))
        );
    }
}

#[test]
fn working_progress_keeps_token_provenance_and_interrupt_visible_when_narrow() {
    let mut app = App::new("model", "project");
    app.apply(&event(RuntimeEvent::TurnStarted));
    let theme = Theme::new();
    assert_eq!(
        working_line(&app, theme, 100).to_string(),
        "✻ Working… (0s · esc to interrupt)"
    );
    app.apply(&event(RuntimeEvent::Usage {
        record: UsageRecord {
            source: UsageSource::ProviderAttempt,
            provenance: Provenance::default(),
            delta: UsageDelta::new().with(CounterKind::Output, 1_200),
        },
    }));
    assert_eq!(
        working_line(&app, theme, 100).to_string(),
        "✻ Working… (0s · ↓ 1.2k tokens · esc to interrupt)"
    );
    app.turn_usage.output = crate::status::TokenCount::estimated(1_200);
    for width in [40, 44, 80, 100] {
        let line = working_line(&app, theme.without_motion().without_color(), width).to_string();
        assert!(line.starts_with("● Working… (0s · ↓ ~1.2k"), "{line}");
        assert!(line.contains("esc") && line.ends_with(')'), "{line}");
        assert!(line.width() <= usize::from(width), "{line}");
    }
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let line = working_line(&app, theme, 100).to_string();
    assert!(
        line.contains("Interrupting… (0s · ↓ ~1.2k tokens · esc to interrupt)"),
        "{line}"
    );
}

#[test]
fn working_progress_sits_between_the_anchored_pane_and_composer() {
    let mut app = App::new("model", "project");
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Public,
        counts: std::collections::BTreeMap::new(),
        items: Some(vec![PlanItemProjection {
            id: "todo".to_owned(),
            text: "Inspect the retry policy".to_owned(),
            status: PlanItemStatus::InProgress,
            reason: None,
        }]),
    }));
    let theme = Theme::new().without_motion();
    for picker in [false, true] {
        if picker {
            app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        }
        for (width, height) in [(44, 16), (80, 24), (100, 32)] {
            let screen = render(&app, width, height, theme);
            let progress = screen
                .lines()
                .position(|line| line.contains("Working… ("))
                .unwrap();
            let pane = screen
                .lines()
                .position(|line| line.contains(if picker { "/help" } else { "Todo" }))
                .unwrap();
            let composer = screen
                .lines()
                .enumerate()
                .filter_map(|(row, line)| line.starts_with("> ").then_some(row))
                .last()
                .unwrap();
            assert!(pane < progress, "{screen}");
            assert_eq!(progress + 2, composer, "{screen}");
        }
    }
}

#[test]
fn pending_input_is_bounded_labelled_and_shares_the_anchored_budget_with_todos() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.composer.replace("correct the active turn");
    let submission = match app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
        Some(crate::app::Action::Submit { submission, .. }) => submission,
        other => panic!("expected a steer submission, got {other:?}"),
    };
    app.accept_steer(
        SteerReceipt {
            id: SteerId::new("steer-1"),
            turn: TurnId::new("turn-1"),
            ordinal: 1,
        },
        submission,
    );
    for index in 1..=5 {
        app.composer.replace(format!("queued turn {index}"));
        app.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    }
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Public,
        counts: std::collections::BTreeMap::new(),
        items: Some(vec![PlanItemProjection {
            id: "todo-1".to_owned(),
            text: "Run the focused tests".to_owned(),
            status: PlanItemStatus::InProgress,
            reason: None,
        }]),
    }));

    let screen = render(&app, 100, 20, Theme::new().without_color());
    insta_like(
        &screen,
        &[
            "Pending for this turn",
            "process-local",
            "correct the active turn",
            "Queued turns",
            "+2 more queued turns",
            "Todo",
            "Run the focused tests",
        ],
    );
    assert!(
        screen.lines().all(|line| line.width() <= 100),
        "pending input overflowed:\n{screen}"
    );

    app.composer.clear();
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    let picker = render(&app, 100, 20, Theme::new().without_color());
    assert!(!picker.contains("Pending for this turn"), "{picker}");
    assert!(!picker.contains("Todo"), "{picker}");
}

#[test]
fn open_items_render_first_and_completed_items_collapse_to_one_struck_row() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Public,
        counts: std::collections::BTreeMap::new(),
        items: Some(vec![
            PlanItemProjection {
                id: "inspect".to_owned(),
                text: "Inspect the retry module".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
            PlanItemProjection {
                id: "change".to_owned(),
                text: "Implement the fix".to_owned(),
                status: PlanItemStatus::InProgress,
                reason: None,
            },
            PlanItemProjection {
                id: "verify".to_owned(),
                text: "Run the focused tests".to_owned(),
                status: PlanItemStatus::Pending,
                reason: None,
            },
            PlanItemProjection {
                id: "docs".to_owned(),
                text: "Update the docs".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
            PlanItemProjection {
                id: "changelog".to_owned(),
                text: "Note the change in the changelog".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
        ]),
    }));

    let screen = render(&app, 100, 20, Theme::new().without_color());
    let lines: Vec<&str> = screen.lines().collect();
    let heading = lines
        .iter()
        .position(|line| line.contains("Todo"))
        .expect("todo heading");
    let implement = lines
        .iter()
        .position(|line| line.contains("Implement the fix"))
        .expect("open item in authored order");
    let verify = lines
        .iter()
        .position(|line| line.contains("Run the focused tests"))
        .expect("open item in authored order");
    let collapsed = lines
        .iter()
        .position(|line| line.contains("Note the change in the changelog"))
        .expect("the collapsed row names the most recently completed item");
    assert!(
        heading < implement && implement < verify && verify < collapsed,
        "open items render first in authored order, the collapsed row last:\n{screen}"
    );
    assert!(
        lines[collapsed].contains("(+2 done)"),
        "two completed items sit behind the one the row names:\n{}",
        lines[collapsed]
    );
    assert!(
        !screen.contains("Inspect the retry module") && !screen.contains("Update the docs"),
        "a completed item other than the most recent one gets no row of its own:\n{screen}"
    );

    // The collapsed row's text is struck through and dim, not the
    // Success-toned `[x]` an uncollapsed completed item would have used.
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("a test terminal");
    terminal
        .draw(|frame| draw(frame, &app, Theme::new()))
        .expect("a frame");
    let buffer = terminal.backend().buffer().clone();
    let row = (0..buffer.area.height)
        .find(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, *y)].symbol())
                .collect::<String>()
                .contains("Note the change")
        })
        .expect("the collapsed row");
    let rendered = (0..buffer.area.width)
        .map(|x| buffer[(x, row)].symbol())
        .collect::<String>();
    let text_x = u16::try_from(rendered.find("Note the change").expect("collapsed text"))
        .expect("text position fits");
    let cell = &buffer[(text_x, row)];
    assert!(
        cell.modifier.contains(Modifier::CROSSED_OUT),
        "the collapsed row's text is struck through"
    );
    assert!(
        cell.modifier.contains(Modifier::DIM),
        "the collapsed row's text is dim"
    );
}

#[test]
fn a_single_completed_item_reports_no_done_count() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Public,
        counts: std::collections::BTreeMap::new(),
        items: Some(vec![
            PlanItemProjection {
                id: "inspect".to_owned(),
                text: "Inspect the retry module".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
            PlanItemProjection {
                id: "verify".to_owned(),
                text: "Run the focused tests".to_owned(),
                status: PlanItemStatus::Pending,
                reason: None,
            },
        ]),
    }));

    let screen = render(&app, 100, 20, Theme::new().without_color());
    assert!(
        screen.contains("Inspect the retry module"),
        "the single completed item still names itself:\n{screen}"
    );
    assert!(
        !screen.contains("done)"),
        "no item is hidden behind it, so no `(+N done)` suffix renders:\n{screen}"
    );
}

#[test]
fn a_cancelled_item_keeps_its_own_row_and_is_excluded_from_the_collapse() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Public,
        counts: std::collections::BTreeMap::new(),
        items: Some(vec![
            PlanItemProjection {
                id: "skip".to_owned(),
                text: "Skip the deprecated path".to_owned(),
                status: PlanItemStatus::Cancelled,
                reason: None,
            },
            PlanItemProjection {
                id: "inspect".to_owned(),
                text: "Inspect the retry module".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
            PlanItemProjection {
                id: "verify".to_owned(),
                text: "Run the focused tests".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
        ]),
    }));

    let screen = render(&app, 100, 20, Theme::new().without_color());
    assert!(
        screen.contains("[-] Skip the deprecated path"),
        "a cancelled item keeps its own row among the open items:\n{screen}"
    );
    assert!(
        screen.contains("(+1 done)"),
        "two items are completed, so the cancelled item must not inflate \
             the hidden count to two:\n{screen}"
    );
}

#[test]
fn a_fully_completed_plan_stays_visible_while_working_and_retires_once_the_turn_stops() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Public,
        counts: std::collections::BTreeMap::new(),
        items: Some(vec![
            PlanItemProjection {
                id: "inspect".to_owned(),
                text: "Inspect the retry module".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
            PlanItemProjection {
                id: "verify".to_owned(),
                text: "Run the focused tests".to_owned(),
                status: PlanItemStatus::Completed,
                reason: None,
            },
        ]),
    }));

    let while_working = render(&app, 100, 20, Theme::new().without_color());
    assert!(
        while_working.contains("Todo") && while_working.contains("Run the focused tests"),
        "a fully-completed plan still renders while the turn is working:\n{while_working}"
    );

    app.apply(&event(RuntimeEvent::TurnCompleted {
        finish: TurnFinish::Completed,
        visible_output: true,
    }));
    let after_stop = render(&app, 100, 20, Theme::new().without_color());
    assert!(
        !after_stop.contains("Todo") && !after_stop.contains("Run the focused tests"),
        "once the turn is no longer running, a fully-completed plan retires \
             instead of pinning the finished list until the next turn:\n{after_stop}"
    );
}

#[test]
fn a_sensitive_plan_shows_no_item_text_and_no_collapse_row() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.apply(&event(RuntimeEvent::TurnStarted));
    app.apply(&event(RuntimeEvent::PlanUpdated {
        revision: 1,
        sensitivity: PlanSensitivity::Sensitive,
        counts: std::collections::BTreeMap::from([("completed".to_owned(), 3)]),
        items: Some(vec![PlanItemProjection {
            id: "hidden".to_owned(),
            text: "Sensitive step".to_owned(),
            status: PlanItemStatus::Completed,
            reason: None,
        }]),
    }));

    let screen = render(&app, 100, 20, Theme::new().without_color());
    assert!(
        !screen.contains("Todo"),
        "a sensitive plan renders no anchored pane, collapsed or otherwise:\n{screen}"
    );
    assert!(!screen.contains("Sensitive step"), "{screen}");
    assert!(!screen.contains("done)"), "{screen}");
}

#[test]
fn connecting_and_failed_servers_show_in_status_and_go_quiet_once_settled() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.transcript.push_user("explain the retry policy");
    let quiet_transcript = render(&app, 100, 24, Theme::new().without_color());

    app.status.mcp = crate::status::McpStatus {
        connecting: 2,
        failed: 0,
    };
    let connecting = render(&app, 100, 24, Theme::new().without_color());
    assert!(
        connecting.contains("mcp 2 connecting"),
        "a server that is still starting is visible while it lasts: {connecting}"
    );

    app.status.mcp = crate::status::McpStatus {
        connecting: 1,
        failed: 1,
    };
    let mixed = render(&app, 100, 24, Theme::new().without_color());
    assert!(
        mixed.contains("mcp 1 connecting") && mixed.contains("1 failed"),
        "{mixed}"
    );

    // Both settle: status returns to quiet, and nothing about their
    // transitions was written into the conversation.
    app.status.mcp = crate::status::McpStatus::default();
    let settled = render(&app, 100, 24, Theme::new().without_color());
    assert!(
        !settled.contains("mcp "),
        "a settled server reports nothing: {settled}"
    );
    assert_eq!(
        settled, quiet_transcript,
        "the transcript is untouched by a server connecting or failing"
    );
}
