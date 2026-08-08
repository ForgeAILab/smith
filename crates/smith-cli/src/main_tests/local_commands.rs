// local commands behavior tests.

    use smith_tui::status::{PriceReference, PriceTable, SessionUsage};

    #[test]
    fn status_cost_reports_unknown_without_assuming_a_price() {
        // usage-accounting: "Price is unavailable" — `/status` must show the
        // counters and report cost as unknown rather than assuming a price,
        // unlike the exit report's "no cost line at all".
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_000);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            ..SessionUsage::default()
        };
        let rendered = render_status_cost(&usage, None, ("openai", "gpt-5.3"));
        assert_eq!(rendered, "unknown · no price reference for openai/gpt-5.3");
    }

    #[test]
    fn status_cost_names_the_priced_binding_and_labels_it_exact() {
        // usage-accounting: "A priced model with reported counters" — one
        // USD figure labelled exact, naming the provider and model.
        let mut totals = std::collections::BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_000_000);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            ..SessionUsage::default()
        };
        let price = PriceReference {
            provider: "openai".to_owned(),
            model: "gpt-5.3".to_owned(),
            table: PriceTable {
                input: Some(2_000_000),
                output: None,
                cache_read: None,
                cache_write: None,
            },
        };
        let rendered = render_status_cost(&usage, Some(&price), ("openai", "gpt-5.3"));
        assert_eq!(rendered, "$2.000 exact · openai/gpt-5.3");
    }

    #[test]
    fn status_cost_says_nothing_spent_yet_before_any_usage() {
        // A fresh session has no usage to price at all — a different fact
        // from "priced but unknown", and must not be conflated with it.
        let rendered = render_status_cost(&SessionUsage::default(), None, ("openai", "gpt-5.3"));
        assert_eq!(rendered, "nothing spent yet");
    }

    #[test]
    fn context_categories_name_tool_results_separately_from_user_input() {
        let tool = context_display_category("tool_result", 4_200);
        let user = context_display_category("user_input", 58);
        assert_eq!(tool.label, "tool results");
        assert_eq!(tool.glyph, glyph::CONTEXT_TOOL);
        assert_eq!(tool.tokens, 4_200);
        assert_eq!(user.label, "user input");
        assert_eq!(user.glyph, glyph::CONTEXT_INPUT);
        assert_eq!(user.tokens, 58);
        assert!(tool.rank < user.rank);

        let unknown = context_display_category("future_context_kind", 1);
        assert_eq!(unknown.label, "future context kind");
        assert_eq!(unknown.glyph, glyph::CONTEXT_OTHER);
    }

    #[test]
    fn context_categories_keep_system_and_tools_visible_and_aggregate_instructions() {
        let totals = std::collections::BTreeMap::from([
            ("system_instruction".to_owned(), 100),
            ("developer_instruction".to_owned(), 40),
            ("ability_instruction".to_owned(), 60),
            ("history".to_owned(), 300),
        ]);

        let categories = context_display_categories(&totals);
        assert_eq!(categories[0].label, "system instructions");
        assert_eq!(categories[0].tokens, 200);
        assert_eq!(categories[1].label, "tool schemas");
        assert_eq!(categories[1].tokens, 0);
        assert_eq!(categories[2].label, "history");
        assert!(
            categories
                .iter()
                .all(|category| category.label != "developer instructions")
        );
    }

    #[test]
    fn harness_status_names_registry_view_activation_and_context_provenance() {
        let mut status = Status::new("example-model", "/project");
        status.record_registry("registry-fingerprint", 6);
        status.record_scoped_view("view-fingerprint", 4);
        status.record_retrieval("resolver-1", vec!["tool:read".into(), "tool:search".into()]);
        status.record_activation(1, vec!["tool:read".into()]);
        let totals = std::collections::BTreeMap::new();
        status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-fingerprint",
            cache_fingerprint: "cache-fingerprint",
            input_tokens: 100,
            input_budget_tokens: 1_000,
            reserved_tokens: 100,
            segment_count: 1,
            totals: &totals,
            confidence: EstimationConfidence::Exact,
        });
        status.record_compaction(250);

        let rendered = render_harness_status(&status);
        assert!(
            rendered.contains("registry snapshot: registry-fingerprint · 6 entries"),
            "{rendered}"
        );
        assert!(
            rendered.contains("scoped capability view: view-fingerprint · 4 visible"),
            "{rendered}"
        );
        assert!(
            rendered.contains("activation epoch: 1 · tool:read"),
            "{rendered}"
        );
        assert!(
            rendered.contains("context provenance: context-fingerprint · cache cache-fingerprint"),
            "{rendered}"
        );
        assert!(
            rendered.contains("context compaction: 1 run(s) · 250 tokens reclaimed"),
            "{rendered}"
        );
    }

    #[tokio::test]
    async fn informational_commands_append_inline_without_provider_history() {
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
        let runtime = RuntimeRequest {
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
        let history_before = host.session().history().len();
        let mut app = App::new("example-model", project.path().display().to_string());

        let before_plan = render_context_status(&app.status, host.runtime().policy());
        assert!(before_plan.contains("not planned yet"), "{before_plan}");
        assert!(
            before_plan.contains("128k total · 4k reserved"),
            "{before_plan}"
        );
        assert!(
            before_plan.contains("compaction: enabled on overflow · 74.3k recovery target"),
            "{before_plan}"
        );
        let before_context = render_context_view(&app.status, host.runtime().policy());
        assert!(
            before_context.contains("usage unavailable until the first turn"),
            "{before_context}"
        );
        assert!(
            before_context.contains("· free input: 123.9k"),
            "{before_context}"
        );
        assert!(
            before_context.contains("□ output/reasoning reserve: 4k"),
            "{before_context}"
        );
        assert!(
            before_context.contains("■ system instructions: ? (not counted yet)"),
            "{before_context}"
        );
        assert!(
            before_context.contains("◆ tool schemas: ? (not counted yet)"),
            "{before_context}"
        );
        assert_eq!(
            before_context
                .lines()
                .filter(|line| {
                    !line.is_empty()
                        && line
                            .chars()
                            .all(|character| matches!(character, '·' | '□' | ' '))
                })
                .count(),
            5,
            "{before_context}"
        );

        let totals = std::collections::BTreeMap::from([
            (
                agent_runtime_core::manifest::SegmentKind::new("history"),
                1_500,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("tool_schema"),
                500,
            ),
        ]);
        app.status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-test",
            cache_fingerprint: "cache-test",
            input_tokens: 2_000,
            input_budget_tokens: 123_904,
            reserved_tokens: 4_096,
            segment_count: 2,
            totals: &totals,
            confidence: EstimationConfidence::Estimated,
        });
        let planned = render_context_status(&app.status, host.runtime().policy());
        assert!(planned.contains("~98% input left"), "{planned}");
        assert!(planned.contains("~2k used / 123.9k budget"), "{planned}");
        assert!(planned.contains("provider input (session): ?"), "{planned}");
        assert!(planned.contains("tool schema: ~500"), "{planned}");
        assert!(
            planned.contains("compaction: enabled on overflow · 74.3k recovery target"),
            "{planned}"
        );
        let context = render_context_view(&app.status, host.runtime().policy());
        assert!(
            context.contains("example-model · ~2k / 123.9k input tokens · ~98% left"),
            "{context}"
        );
        assert!(context.contains("◆ tool schemas: ~500"), "{context}");
        assert!(
            context.contains("■ system instructions: ~0 (0.0%)"),
            "{context}"
        );
        assert!(context.contains("● history: ~1.5k"), "{context}");
        assert!(
            context.contains("counting: estimated · 2 segments"),
            "{context}"
        );

        handle_local_command(&mut app, &host, project.path(), None, CommandAction::Status).await;
        handle_local_command(&mut app, &host, project.path(), None, CommandAction::Context).await;
        handle_local_command(&mut app, &host, project.path(), None, CommandAction::Agent(None)).await;
        handle_local_command(&mut app, &host, project.path(), None, CommandAction::Diff(None)).await;

        git(project.path(), &["init"]);
        git(
            project.path(),
            &["config", "user.email", "smith@example.invalid"],
        );
        git(project.path(), &["config", "user.name", "Smith Test"]);
        std::fs::write(project.path().join("tracked.txt"), "before\n").expect("tracked");
        git(project.path(), &["add", "tracked.txt"]);
        git(project.path(), &["commit", "-m", "initial"]);
        std::fs::write(project.path().join("tracked.txt"), "after\n").expect("changed");
        handle_local_command(
            &mut app,
            &host,
            project.path(),
            None,
            CommandAction::Diff(Some("unstaged".to_owned())),
        )
        .await;

        for character in "/help".chars() {
            app.on_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        assert_eq!(
            app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );

        let results = app
            .transcript
            .blocks()
            .iter()
            .filter_map(|block| match block {
                Block::LocalResult { title, state, .. } => Some((title.as_str(), *state)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            results,
            [
                ("status", LocalResultState::Info),
                ("context", LocalResultState::Info),
                ("agents", LocalResultState::Empty),
                ("diff", LocalResultState::Error),
                ("diff · unstaged", LocalResultState::Info),
                ("help", LocalResultState::Info),
            ]
        );
        assert!(app.overlay.is_none(), "local output must not open a viewer");
        assert_eq!(
            host.session().history().len(),
            history_before,
            "local output became provider conversation history"
        );
        let status_content = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "status" => {
                    Some(content.as_str())
                }
                _ => None,
            })
            .expect("status output");
        assert!(
            status_content.contains("~98% input left"),
            "{status_content}"
        );
        assert!(
            status_content.contains("profile: dev · posture build · use main · rev"),
            "{status_content}"
        );
        assert!(status_content.contains("source"), "{status_content}");
        // No usage has been recorded yet, so `/status` reports "nothing
        // spent yet" rather than either a zero dollar figure or "unknown" —
        // there is nothing to price, which is a different fact from a
        // priced session's cost being unpriceable.
        assert!(
            status_content.contains("cost: nothing spent yet"),
            "{status_content}"
        );
        let context_content = app
            .transcript
            .blocks()
            .iter()
            .find_map(|block| match block {
                Block::LocalResult { title, content, .. } if title == "context" => {
                    Some(content.as_str())
                }
                _ => None,
            })
            .expect("context output");
        assert!(
            context_content.contains("Estimated usage by category"),
            "{context_content}"
        );

        let totals = std::collections::BTreeMap::from([
            (
                agent_runtime_core::manifest::SegmentKind::new("summary"),
                600,
            ),
            (
                agent_runtime_core::manifest::SegmentKind::new("user_input"),
                600,
            ),
        ]);
        app.status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-summary",
            cache_fingerprint: "cache-summary",
            input_tokens: 1_200,
            input_budget_tokens: 123_904,
            reserved_tokens: 4_096,
            segment_count: 2,
            totals: &totals,
            confidence: EstimationConfidence::Estimated,
        });
        let compacted = render_context_status(&app.status, host.runtime().policy());
        assert!(
            compacted.contains("compaction: applied · ~600 summary · 74.3k recovery target"),
            "{compacted}"
        );
        host.shutdown().await.expect("shutdown");
    }
