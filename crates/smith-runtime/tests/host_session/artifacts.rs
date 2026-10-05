use super::*;

#[tokio::test]
async fn oversized_shell_output_is_recoverable_from_the_session_artifact_store() {
    let fixture = Fixture::with_config(&format!(
        "{CONFIG}\n[limits]\ntool_output_limit_bytes = 1024\n"
    ));
    // Larger than the configured inline/offload threshold but deliberately
    // smaller than ArtifactOffloader's default, proving the resolved Smith
    // policy is wired into the live processor rather than merely documented.
    let command = "yes 'recoverable artifact line' | head -c 4096";
    let mut shell = tool_call_fragments(
        0,
        "large-shell-call",
        "shell",
        &serde_json::json!({ "command": command }).to_string(),
    );
    shell.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(shell),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "the output is available as an artifact".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("a hosted session");
    assert!(
        host.runtime()
            .policy()
            .tools
            .iter()
            .any(|tool| tool == "artifact.read"),
        "the standard protected store did not register its reader"
    );

    host.session()
        .run(UserInput::text(
            "Use shell to produce the large diagnostic output.",
        ))
        .await
        .expect("the shell turn completes");

    let requests = provider.requests();
    let second_request = &requests[1];
    let wire =
        serde_json::to_string(&second_request.messages).expect("serializable provider messages");
    let marker = wire.split("[artifact id=").nth(1).unwrap_or_else(|| {
        panic!(
            "a model-facing artifact reference; first tools {:?}; wire chars {}",
            requests[0]
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            wire.chars().count(),
        )
    });
    let id = marker
        .split_whitespace()
        .next()
        .expect("artifact id in marker");
    assert!(
        !wire.contains("[output truncated at 131072 bytes]"),
        "Smith's former shell truncation ran before artifact offloading"
    );

    let store = host
        .runtime()
        .artifact_store()
        .expect("the protected Smith artifact store");
    let id = ArtifactId::new(id).expect("a bounded artifact id");
    let mut offset = 0;
    let mut exact = Vec::new();
    loop {
        let chunk = store
            .read(ArtifactRead {
                session: host.session().id().clone(),
                id: id.clone(),
                offset,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await
            .expect("an owner-authorized artifact page");
        exact.extend_from_slice(&chunk.bytes);
        let Some(next) = chunk.next_offset else {
            break;
        };
        assert!(next > offset, "pagination must advance");
        offset = next;
    }
    let exact = String::from_utf8(exact).expect("serialized text tool outcome");
    assert!(exact.contains("recoverable artifact line"));
    assert!(exact.contains(r#""truncated":false"#));
    assert!(exact.len() > 1024);
    assert!(
        exact.len() < 64 * 1024,
        "the fixture must remain below the offloader's default threshold"
    );

    host.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_protected_live_event_resolves_its_safe_display_from_canonical_history() {
    const OLD: &str = "TOP_SECRET_OLD_BODY";
    const NEW: &str = "TOP_SECRET_NEW_BODY";

    let fixture = Fixture::new();
    std::fs::write(fixture.project.path().join("tracked.txt"), OLD).expect("target file");
    let mut edit = tool_call_fragments(
        0,
        "edit-display-1",
        "edit",
        &serde_json::json!({
            "path": "tracked.txt",
            "old_string": OLD,
            "new_string": NEW
        })
        .to_string(),
    );
    edit.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(edit),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(provider);
    let host = start(request).await.expect("host");
    let journal_path = host
        .paths()
        .expect("persistent paths")
        .journal(host.session().id())
        .expect("journal path");
    let mut events = host.session().subscribe();
    host.session()
        .send(UserInput::text("perform the reviewed edit"))
        .expect("the turn is accepted");

    let invocation = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut invocation = None;
        while let Some(event) = events.next().await {
            match &event.payload {
                RuntimeEvent::ToolCallRequested {
                    call, arguments, ..
                } => {
                    assert!(
                        arguments.is_none(),
                        "Smith must not opt raw arguments into canonical events"
                    );
                    invocation = host
                        .tool_call_display(call)
                        .map(|display| display.invocation());
                }
                RuntimeEvent::TurnCompleted { .. } => break,
                _ => {}
            }
        }
        invocation
    })
    .await
    .expect("turn completed")
    .expect("safe display was available when the protected event arrived");

    assert_eq!(invocation, "Update(tracked.txt)");
    host.shutdown().await.expect("clean shutdown");
    let journal = std::fs::read_to_string(journal_path).expect("event journal");
    assert!(!journal.contains(OLD), "{journal}");
    assert!(!journal.contains(NEW), "{journal}");
}

#[tokio::test]
async fn an_edit_turn_is_attributed_and_undoable_through_the_host() {
    let fixture = Fixture::new();
    std::fs::write(fixture.project.path().join("tracked.txt"), "before\n").expect("file");
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(edit_provider());
    let host = start(request).await.expect("host");

    host.session()
        .run(UserInput::text("edit the file"))
        .await
        .expect("the turn runs");
    let set = host.changes().latest().expect("change set");
    assert!(set.is_fully_attributable());
    assert!(
        host.changes()
            .undo_preview()
            .expect("preview")
            .contains("-after")
    );
    host.changes().undo_latest().expect("undo");
    assert_eq!(
        std::fs::read_to_string(fixture.project.path().join("tracked.txt")).expect("read"),
        "before\n"
    );
    host.shutdown().await.expect("shutdown");
}
