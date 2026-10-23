use super::*;

#[tokio::test]
async fn late_child_inspection_renders_task_tools_and_answer_without_resuming() {
    use agent_runtime::delegation::SpawnOutcome;
    use agent_runtime::provider::fake::tool_call_fragments;
    use agent_runtime_core::delegation::{
        ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
    };

    #[derive(Debug)]
    struct PausingProvider {
        inner: Arc<FakeProvider>,
        calls: std::sync::atomic::AtomicUsize,
        ready: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    #[async_trait::async_trait]
    impl agent_runtime_core::provider::Provider for PausingProvider {
        fn describe(&self) -> Vec<agent_runtime_core::provider::ModelDescriptor> {
            self.inner.describe()
        }

        fn capabilities(
            &self,
            model: &agent_runtime_core::provider::ModelId,
        ) -> Option<Capabilities> {
            self.inner.capabilities(model)
        }

        async fn stream(
            &self,
            request: agent_runtime_core::provider::ProviderRequest,
            ctx: agent_runtime_core::provider::ProviderCallContext,
        ) -> Result<
            agent_runtime_core::provider::ProviderStream,
            agent_runtime_core::provider::ProviderError,
        > {
            if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                self.ready.notify_one();
                self.release.notified().await;
            }
            self.inner.stream(request, ctx).await
        }
    }

    let render = |app: &smith_tui::app::App| {
        let backend = ratatui::backend::TestBackend::new(100, 40);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| smith_tui::render::draw(frame, app, smith_tui::theme::Theme::default()))
            .expect("render child view");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    let fixture = Fixture::new();
    std::fs::write(
        fixture.project.path().join("transcript-marker.txt"),
        "retained tool evidence",
    )
    .expect("marker file");
    let mut first = vec![ProviderStreamEvent::TextDelta {
        text: "I will read the marker before reporting.".into(),
    }];
    first.extend(tool_call_fragments(
        0,
        "child-marker-read",
        "read",
        &serde_json::json!({"path":"transcript-marker.txt"}).to_string(),
    ));
    first.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let reply = |text: &str| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta { text: text.into() },
            usage_event(50, 12),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let inner = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(first),
            reply("The marker contains retained tool evidence."),
            reply("The delegated inspection is complete."),
        ],
    ));
    let provider = Arc::new(PausingProvider {
        inner,
        calls: std::sync::atomic::AtomicUsize::new(0),
        ready: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let (host, resources) = fixture
        .start_configured(None, Overrides::default(), false, &[], |request| {
            request.provider = Some(provider.clone());
            request.clock = Some(Arc::new(agent_runtime_core::clock::SystemClock));
        })
        .await;
    let coordinator = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("coordinator");
    let child = match coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Read the marker and report its contents."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(2),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("spawn")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("unexpected spawn: {other:?}"),
    };
    tokio::time::timeout(Duration::from_secs(10), provider.ready.notified())
        .await
        .expect("child reached the paused provider request");
    let turns_before = coordinator.status(&child).expect("status").turns_used;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (snapshot, forwarder) = crate::tui_driver::subscribe_to_child(&host, &child, tx)
        .await
        .expect("late subscription");
    let mut app = fixture.app(&host, &resources, None, None).await.app;
    app.work_details = true;
    app.restore_child_transcript_with_observations(
        child.as_str(),
        &snapshot.history,
        snapshot.next_event_sequence,
        &[],
        &snapshot.observations,
    );
    for (call, display) in snapshot.tool_displays {
        app.set_child_tool_display(child.as_str(), call.as_str(), display);
    }
    for (call, result) in snapshot.tool_results {
        app.set_child_tool_result_preview(child.as_str(), call.as_str(), result);
    }
    for event in snapshot.pending_events {
        app.restore_child_pending_event(child.as_str(), &event);
    }
    app.inspected_child = Some(child.to_string());
    let paused = render(&app);
    assert!(
        paused.contains("Read the marker and report its contents."),
        "{paused}"
    );
    assert!(paused.contains("retained tool evidence"), "{paused}");
    assert!(
        !paused.contains("The marker contains retained tool evidence."),
        "{paused}"
    );
    provider.release.notify_one();
    tokio::time::timeout(
        Duration::from_secs(10),
        coordinator.wait_task_outcome(&child),
    )
    .await
    .expect("child completed")
    .expect("child outcome");
    loop {
        let (event_child, event) = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("child live tail")
            .expect("child stream");
        let finished = matches!(event.payload, RuntimeEvent::TurnCompleted { .. });
        app.apply_child(event_child.as_str(), &event);
        if finished {
            break;
        }
    }
    let status = coordinator.status(&child).expect("completed status");
    let (state, detail) = crate::submission::child_summary_projection(&status);
    app.replace_children(vec![(child.to_string(), state, Some(detail))]);
    let screen = render(&app);
    for text in [
        "Read the marker and report its contents.",
        "I will read the marker before reporting.",
        "transcript-marker.txt",
        "retained tool evidence",
        "The marker contains retained tool evidence.",
    ] {
        assert!(screen.contains(text), "missing {text:?}:\n{screen}");
    }
    assert_eq!(
        coordinator.status(&child).expect("status").turns_used,
        turns_before
    );
    println!("Paused child after late subscription:\n{paused}\nCompleted child:\n{screen}");
    if let Some(task) = forwarder {
        task.abort();
    }
    let parent_id = host.session().id().clone();
    host.shutdown().await.expect("shutdown");
    drop(host);
    let (restored, _) = fixture.start(Some(parent_id), Overrides::default()).await;
    let retained = restored
        .child_transcript(&child)
        .await
        .expect("durable child history");
    let retained_wire = serde_json::to_string(&retained.history).expect("retained messages");
    assert!(retained_wire.contains("Read the marker and report its contents."));
    assert!(retained_wire.contains("retained tool evidence"));
    assert!(retained_wire.contains("The marker contains retained tool evidence."));
    assert_eq!(retained.status.turns_used, turns_before);
    restored.shutdown().await.expect("restored shutdown");
}
