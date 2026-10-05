use super::*;

#[test]
fn active_context_window_is_named_in_the_identity_footer() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.status.context_window = Some("872k".to_owned());

    let screen = render(&app, 74, 16, Theme::new().without_color());
    let footer = screen.lines().last().unwrap_or_default();
    assert!(footer.contains("gpt-5.3 · build · 872k"), "{footer}");
}

#[test]
fn command_completion_renders_above_the_composer_without_a_control_strip() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.on_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    for character in "bogus".chars() {
        app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let screen = render(&app, 74, 20, Theme::new().without_color());
    insta_like(
        &screen,
        &[
            "no matching commands",
            "> /bogus",
            "unknown command",
            "gpt-5.3",
        ],
    );
    assert!(!screen.contains("tab complete"), "{screen}");
    assert!(!screen.contains("↑↓ select"), "{screen}");
    assert!(!screen.contains("enter run"), "{screen}");
    assert!(!screen.contains("esc close"), "{screen}");
    assert!(
        !screen.contains("command completion"),
        "the completion list must not grow a modal title:\n{screen}"
    );
}

#[test]
fn command_completion_keeps_one_identity_footer_row_at_all_widths() {
    for (width, height) in [(44, 14), (74, 24), (120, 32)] {
        let mut app = App::new("gpt-5.3", "~/work/api");
        app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));

        let screen = render(
            &app,
            width,
            height,
            Theme::new().without_color().without_motion(),
        );
        assert!(screen.contains("/help"), "{width}x{height}: {screen}");
        assert!(
            screen
                .lines()
                .last()
                .is_some_and(|line| line.contains("gpt-5.3")),
            "{width}x{height}: {screen}"
        );
        assert!(
            !screen.contains("tab complete"),
            "{width}x{height}: {screen}"
        );
        assert!(!screen.contains("enter run"), "{width}x{height}: {screen}");
        assert!(!screen.contains("esc close"), "{width}x{height}: {screen}");
    }
}

#[test]
fn command_completion_and_footer_keep_the_semantic_color_roles() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.on_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));

    {
        let mut terminal = Terminal::new(TestBackend::new(74, 20)).expect("a test terminal");
        terminal
            .draw(|frame| draw(frame, &app, Theme::new()))
            .expect("a frame");
        let buffer = terminal.backend().buffer();
        let row = |y: u16| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };

        let completion_y = (0..buffer.area.height)
            .find(|y| row(*y).starts_with("❯ /help "))
            .expect("selected completion row");
        let composer_y = (0..buffer.area.height)
            .find(|y| row(*y).trim_end() == "> /")
            .expect("composer row");
        assert!(
            completion_y < composer_y,
            "completion should sit above the fixed composer"
        );
        let completion = row(completion_y);
        let command_x = u16::try_from(
            completion[..completion.find("/help").expect("command position")].width(),
        )
        .expect("command position fits");
        let description_x = u16::try_from(
            completion[..completion
                .find("List commands and keys")
                .expect("description")]
                .width(),
        )
        .expect("description position fits");
        assert_eq!(buffer[(command_x, completion_y)].fg, Color::Cyan);
        assert!(
            buffer[(command_x, completion_y)]
                .modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            buffer[(description_x, completion_y)]
                .modifier
                .contains(Modifier::DIM)
        );
        assert!(!completion.contains("command completion"));
        assert!(!completion.contains('╭'));
    }

    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let mut terminal = Terminal::new(TestBackend::new(74, 20)).expect("a test terminal");
    terminal
        .draw(|frame| draw(frame, &app, Theme::new()))
        .expect("a frame");
    let buffer = terminal.backend().buffer();
    let footer_y = buffer.area.height - 1;
    let footer = (0..buffer.area.width)
        .map(|x| buffer[(x, footer_y)].symbol())
        .collect::<String>();
    let model_x = u16::try_from(footer.find("gpt-5.3").expect("model in footer"))
        .expect("model position fits");
    let path_x = u16::try_from(footer.find("~/work/api").expect("path in footer"))
        .expect("path position fits");
    assert_eq!(buffer[(model_x, footer_y)].fg, Color::Cyan);
    assert_eq!(buffer[(path_x, footer_y)].fg, Color::Green);
}

#[test]
fn a_narrow_busy_footer_keeps_the_keys_and_drops_identity() {
    let app = conversation();
    let screen = render(&app, 44, 14, Theme::new());
    let footer = screen.lines().last().unwrap_or_default();
    assert!(
        footer.contains("enter steer · tab queue · esc interrupt"),
        "{footer}"
    );
    assert!(!footer.contains("gpt-5.3"), "{footer}");
    assert!(
        footer.width() <= 44,
        "the footer must not overflow: {footer}"
    );
}

#[test]
fn non_default_reasoning_hint_is_width_gated_in_the_existing_footer() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.status
        .set_reasoning_hint(Some("think on · effort high".to_owned()));
    let narrow = render(&app, 74, 14, Theme::new().without_color().without_motion());
    assert!(!narrow.contains("effort high"), "{narrow}");

    let wide = render(&app, 120, 14, Theme::new().without_color().without_motion());
    assert!(wide.contains("think on · effort high"), "{wide}");
}

#[test]
fn ctrl_c_exit_hint_replaces_then_restores_the_footer_at_all_widths() {
    for (width, height) in [(44, 14), (74, 24), (120, 32)] {
        let mut app = App::new("glm-5.2", "/Volumes/Data/codes/ai/agent-runtime:main");
        app.status.switch_model(Some("zai".to_owned()), "glm-5.2");
        app.status.set_agent("build");
        let theme = Theme::new().without_color().without_motion();

        let baseline = render(&app, width, height, theme);
        let baseline_footer = baseline.lines().last().unwrap_or_default().to_owned();
        assert!(baseline_footer.contains("glm-5.2"), "{baseline_footer}");

        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            None
        );
        let armed = render(&app, width, height, theme);
        let armed_footer = armed.lines().last().unwrap_or_default();
        assert_eq!(
            armed_footer.trim(),
            "press Ctrl+C again to exit",
            "{width}x{height}: {armed}"
        );
        assert!(!armed_footer.contains("glm-5.2"), "{armed_footer}");
        assert!(!armed_footer.contains("unknown ctx"), "{armed_footer}");

        assert!(app.expire_ctrl_c_exit_hint_at(
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ));
        let restored = render(&app, width, height, theme);
        assert_eq!(
            restored.lines().last().unwrap_or_default(),
            baseline_footer,
            "{width}x{height}: {restored}"
        );
    }
}

#[test]
fn running_background_tasks_appear_in_the_footer_and_clear_when_the_poll_reports_none() {
    let mut app = App::new("gpt-5.3", "~/work/api");
    app.set_running_tasks(vec![crate::app::RunningTaskSummary {
        task_id: "task:3".to_owned(),
        command_hint: "npm test".to_owned(),
    }]);

    let with_task = render(&app, 100, 20, Theme::new().without_color());
    assert!(with_task.contains("task:3"), "{with_task}");

    app.set_running_tasks(Vec::new());
    let cleared = render(&app, 100, 20, Theme::new().without_color());
    assert!(!cleared.contains("task:3"), "{cleared}");
}

#[test]
fn feedback_keeps_right_identity_at_100_and_44_columns() {
    for width in [100, 44] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let mut app = App::new("example-model", "<PROJECT>");
            app.status.approval_mode = Some("ask".to_owned());
            app.push_notice(NoticeKind::AccountUnchanged, "already using that account");

            let screen = render(&app, width, 20, theme);
            let footer = screen.lines().last().unwrap();
            let left = "  already using that account";
            let identity = if width == 100 {
                "example-model · build · ask · <PROJECT> · unknown ctx"
            } else {
                "example-model"
            };
            assert_eq!(
                footer,
                format!(
                    "{left}{}{identity}",
                    " ".repeat(usize::from(width) - left.width() - identity.width())
                ),
                "{screen}"
            );
            assert!(!footer.contains("? for shortcuts"), "{screen}");
            assert!(app.transcript.is_empty());
        }
    }
}

#[test]
fn feedback_yields_identity_before_clipping_whole_words() {
    for width in [100, 44] {
        for theme in [Theme::new(), Theme::new().without_color()] {
            let mut app = App::new("example-model", "<PROJECT>");
            let message = if width == 100 {
                "the account is already active; continue typing while keeping the current selection unchanged"
            } else {
                "account selection unchanged; keep typing"
            };
            app.push_notice(NoticeKind::AccountUnchanged, message);
            let screen = render(&app, width, 20, theme);
            assert_eq!(
                screen.lines().last().unwrap(),
                format!("  {message}"),
                "{screen}"
            );

            // Keep wide glyphs in the prefix and omit an oversized word whole.
            // Identity must not reclaim the space freed by clipping.
            app.push_notice(
                NoticeKind::AccountUnchanged,
                format!("keep 当前账号 selection {}", "x".repeat(100)),
            );
            let screen = render(&app, width, 20, theme);
            assert_eq!(
                screen.lines().last().unwrap(),
                "  keep 当前账号 selection…",
                "{screen}"
            );

            if width == 44 {
                app.push_notice(
                    NoticeKind::AccountUnchanged,
                    "account selection unchanged; keep typing another message",
                );
                let screen = render(&app, width, 20, theme);
                assert_eq!(
                    screen.lines().last().unwrap(),
                    "  account selection unchanged; keep typing…",
                    "{screen}"
                );
            }
        }
    }
}

#[test]
fn refused_command_feedback_replaces_left_hints_until_the_next_keypress() {
    for theme in [Theme::new(), Theme::new().without_color()] {
        let mut app = App::new("model", "project");
        app.apply(&event(RuntimeEvent::TurnStarted));
        app.composer.replace("/model");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let message = "/model requires an idle turn; draft preserved";
        assert!(app.transcript.is_empty());
        assert_eq!(app.composer.text(), "/model");
        let screen = render(&app, 100, 20, theme);
        assert!(screen.lines().last().unwrap().contains(message), "{screen}");
        assert!(
            screen
                .lines()
                .last()
                .unwrap()
                .ends_with("model · build · project · unknown ctx"),
            "{screen}"
        );
        assert_eq!(screen.matches(message).count(), 1, "{screen}");

        // Release events and unrelated runtime reports are not keypresses.
        let mut release = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        release.kind = crossterm::event::KeyEventKind::Release;
        app.on_key(release);
        app.apply(&event(RuntimeEvent::ProviderAttemptFinished {
            attempt: AttemptId::new("retry"),
            index: Some(0),
            max_attempts: Some(3),
            finish: agent_runtime_core::provider::FinishReason::Error,
            retryable: true,
            error: None,
            retry_delay_ms: Some(200),
        }));
        assert_eq!(app.feedback_notice().unwrap().text, message);
        let screen = render(&app, 100, 20, theme);
        assert!(screen.lines().last().unwrap().contains(message), "{screen}");
        assert!(
            screen.contains("provider · retrying 2/3 in 200ms: provider attempt failed"),
            "{screen}"
        );
        assert!(
            matches!(app.transcript.blocks(), [crate::transcript::Block::Notice {
            kind: NoticeKind::Provider, text,
        }] if text == "retrying 2/3 in 200ms: provider attempt failed")
        );

        // Even a navigation key clears feedback and restores normal hints.
        app.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert!(app.feedback_notice().is_none());
        let screen = render(&app, 100, 20, theme);
        assert!(!screen.contains(message), "{screen}");
        assert!(
            screen.contains("provider · retrying 2/3 in 200ms: provider attempt failed"),
            "{screen}"
        );
        assert_eq!(app.transcript.len(), 1);
    }
}
