// background-task pushed-state, key-mapping, and exit-gate behavior tests.

    #[test]
    fn status_listing_shows_a_running_task_and_drops_it_when_the_poll_reports_none() {
        let mut app = app();
        assert_eq!(app.render_running_tasks_footer(), None);

        app.set_running_tasks(vec![RunningTaskSummary {
            task_id: "task:1".to_owned(),
            command_hint: "npm test".to_owned(),
        }]);
        assert_eq!(
            app.render_running_tasks_footer().as_deref(),
            Some("bg task:1")
        );

        // The registry already filters to non-terminal tasks, so a task's
        // absence from the next push is itself the terminal signal — no
        // separate removal call is needed.
        app.set_running_tasks(Vec::new());
        assert_eq!(app.render_running_tasks_footer(), None);
        assert!(app.running_tasks.is_empty());
    }

    #[test]
    fn ctrl_b_backgrounds_the_shell_while_escape_keeps_interrupting() {
        let mut idle = app();
        assert_eq!(idle.on_key(ctrl('b')), Some(Action::BackgroundShell));

        let mut busy = app();
        busy.apply(&event(RuntimeEvent::TurnStarted));
        assert_eq!(busy.on_key(ctrl('b')), Some(Action::BackgroundShell));
        // The interrupt action stays cancel-and-kill; Ctrl+B must never be
        // read as a synonym for it.
        assert_eq!(busy.on_key(key(KeyCode::Esc)), Some(Action::Interrupt));
        assert_ne!(Action::BackgroundShell, Action::Interrupt);
    }

    #[test]
    fn quit_immediately_captures_a_running_background_task() {
        let mut app = app();
        app.set_running_tasks(vec![RunningTaskSummary {
            task_id: "task:7".to_owned(),
            command_hint: "cargo build".to_owned(),
        }]);

        type_text(&mut app, "/quit");
        assert_eq!(app.on_key(key(KeyCode::Enter)), Some(Action::Quit));
        assert!(app.overlay.is_none());
        assert!(app.should_quit);
        assert_eq!(app.stopped_work.as_deref(), Some("background task task:7"));
    }

    #[test]
    fn quit_summary_names_live_children_and_omits_settled_children() {
        let mut app = app();
        app.restore_child("child-running", ChildState::Running, None);
        app.restore_child("child-completed", ChildState::Completed, None);
        assert_eq!(app.request_exit(), Some(Action::Quit));
        assert_eq!(app.stopped_work.as_deref(), Some("child child-running"));
    }

    #[test]
    fn idle_quit_has_no_stopped_work_line() {
        let mut app = app();
        assert_eq!(app.request_exit(), Some(Action::Quit));
        assert!(app.should_quit);
        assert!(app.stopped_work.is_none());
    }
