use super::*;

#[tokio::test]
async fn a_child_artifact_is_explicitly_transferred_without_widening_source_ownership() {
    child_artifact_policy_case(10_000, 8192).await;
}

#[tokio::test]
async fn a_child_uses_the_resolved_small_inline_threshold_not_the_runtime_default() {
    child_artifact_policy_case(100, 1024).await;
}

async fn child_artifact_policy_case(lines: usize, inline_bytes: u32) {
    let fixture = Fixture::new();
    let large_fixture = "child-owned artifact line\n".repeat(lines);
    std::fs::write(
        fixture.project.path().join("child-artifact.txt"),
        large_fixture,
    )
    .expect("large read-only child fixture");
    let mut read = tool_call_fragments(
        0,
        "child-large-read",
        "read",
        &serde_json::json!({ "path": "child-artifact.txt", "limit": 10_000 }).to_string(),
    );
    read.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(read),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "child artifact ready".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "parent received the transferred artifact".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path())
        .expect("protected Smith paths");
    let store = Arc::new(SmithArtifactStore::new(paths));
    let mut runtime_request = request(&fixture, provider.clone());
    runtime_request.workspace = Some(Arc::new(
        ProjectWorkspace::new(fixture.project.path()).expect("a project workspace"),
    ));
    runtime_request.artifact_store = Some(store.clone());
    runtime_request
        .config
        .context
        .tool_output_inline_bytes
        .value = inline_bytes;
    let smith = factory::build_request(runtime_request)
        .await
        .expect("a runtime with protected artifact transfer");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a parent session");
    let delegation = smith.delegation().expect("a delegation surface");
    let _lifecycle = wire_delegation(&session, delegation)
        .await
        .expect("delegation wires");
    let coordinator = delegation.coordinator().expect("a coordinator");

    let spawned = coordinator
        .spawn(ChildSpec {
            task: UserInput::text(
                "Read the large fixture so its bounded result is retained as an artifact.",
            ),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the child spawns");
    let child = match spawned {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    let outcome = coordinator
        .wait_task_outcome(&child)
        .await
        .expect("a completed child outcome");
    let transferred = match &outcome {
        ChildTaskOutcome::Completed { result, .. } => {
            assert_eq!(result.text, "child artifact ready");
            assert_eq!(result.artifacts.len(), 1);
            result.artifacts[0].clone()
        }
        other => panic!("expected a completed child outcome, got {other:?}"),
    };
    assert_eq!(transferred.provenance.session, *session.id());
    let source = transferred
        .provenance
        .derived_from
        .clone()
        .expect("the parent reference retains child lineage");
    assert_ne!(source.session, *session.id());
    assert_eq!(source.digest, transferred.digest);

    assert_eq!(
        store
            .read(ArtifactRead {
                session: session.id().clone(),
                id: source.id,
                offset: 0,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await,
        Err(ArtifactError::AccessDenied),
        "the transferred reference must not grant access to its child-owned source"
    );
    let page = store
        .read(ArtifactRead {
            session: session.id().clone(),
            id: transferred.id.clone(),
            offset: 0,
            limit: MAX_ARTIFACT_READ_BYTES,
        })
        .await
        .expect("the explicit parent-owned copy is readable");
    assert!(
        String::from_utf8_lossy(&page.bytes).contains("child-owned artifact line"),
        "the transferred parent copy preserves exact child output"
    );

    wait_for_provider_requests(&provider, 3).await;
    let parent_request = &provider.requests()[2];
    let delivery = serde_json::to_string(&parent_request.messages).expect("parent messages");
    assert!(
        delivery.contains("delegation.child-completion"),
        "{delivery}"
    );
    assert!(delivery.contains(child.as_str()), "{delivery}");
    assert!(delivery.contains("child artifact ready"), "{delivery}");
    assert_eq!(
        coordinator
            .task_outcome(&child)
            .expect("known child")
            .expect("retained protected outcome"),
        outcome,
        "automatic admission does not consume the exact protected status result"
    );

    session.shutdown().await.expect("a clean shutdown");
}
