use super::*;

#[tokio::test]
async fn interrupting_one_turn_does_not_cancel_the_hosted_session() {
    let fixture = Fixture::new();
    let provider: Arc<dyn Provider> = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::blocking(vec![ProviderStreamEvent::TextDelta {
                text: "discarded partial answer".to_owned(),
            }]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "later turn completed".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(provider);
    let host = start(request).await.expect("a hosted session");
    let session = host.session();
    let mut events = session.subscribe();

    let first = session
        .send(UserInput::text("start blocking work"))
        .expect("the first turn is accepted");
    let first_id = first.id().clone();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if event.turn.as_ref() == Some(&first_id)
                && matches!(event.payload, RuntimeEvent::TextDelta { .. })
            {
                return;
            }
        }
        panic!("the event stream ended before the first delta");
    })
    .await
    .expect("the first attempt starts");

    session
        .interrupt_current_turn(CancelReason::UserRequested)
        .expect("the active turn is interruptible");
    tokio::time::timeout(std::time::Duration::from_secs(5), first.completed())
        .await
        .expect("the interrupted turn reaches a terminal boundary");

    let first_finish = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(event) = events.next().await {
            if event.turn.as_ref() == Some(&first_id)
                && let RuntimeEvent::TurnCompleted { finish, .. } = event.payload
            {
                return finish;
            }
        }
        panic!("the event stream ended before turn completion");
    })
    .await
    .expect("the first turn emits its terminal event");
    assert_eq!(
        first_finish,
        TurnFinish::Cancelled {
            reason: CancelReason::UserRequested
        }
    );

    let second = session
        .run(UserInput::text("run after the interruption"))
        .await
        .expect("the same session accepts a later turn");
    assert_ne!(second.id(), &first_id);
    let history = session.history();
    assert!(
        history
            .iter()
            .any(|message| message.joined_text() == "later turn completed"),
        "the later turn did not complete on the original session"
    );
    assert!(
        history
            .iter()
            .all(|message| !message.joined_text().contains("discarded partial answer")),
        "discarded speculative text entered canonical session history"
    );

    host.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn questionnaire_answer_resumes_the_same_turn_without_approval_authority() {
    let fixture = Fixture::new();
    let arguments = serde_json::json!({
        "questions": [{
            "id": "direction",
            "header": "Direction",
            "prompt": "Which implementation direction?",
            "choices": [
                {"id": "small", "label": "Small"},
                {"id": "large", "label": "Large"}
            ]
        }],
        "sensitivity": "public"
    })
    .to_string();
    let mut question = tool_call_fragments(0, "question-call", "ask_user", &arguments);
    question.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(question),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "Implemented the small direction.".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let (interaction, mut requests) = InteractiveInteraction::new();
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(provider.clone());
    request.runtime.approval = Some(Arc::new(DenyAll));
    request.runtime.interaction = Some(Arc::new(interaction));
    let host = start(request).await.expect("interactive host");

    let run = host
        .session()
        .run(UserInput::text("ask then continue this turn"));
    let answer = async {
        let InteractionNotice::Present(prompt) =
            requests.recv().await.expect("questionnaire presentation")
        else {
            panic!("expected a questionnaire presentation");
        };
        assert_eq!(prompt.request().origin().session(), host.session().id());
        prompt
            .answer(vec![QuestionAnswer::choice(
                QuestionId::new("direction"),
                ChoiceId::new("small"),
            )])
            .expect("typed answer accepted");
    };
    let (result, ()) = tokio::join!(run, answer);
    result.expect("the same turn resumes after the answer");

    assert_eq!(
        provider.requests().len(),
        2,
        "answering created a second user turn or repeated the first provider request"
    );
    assert!(
        host.session()
            .history()
            .iter()
            .any(|message| { message.joined_text() == "Implemented the small direction." })
    );
    host.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn sensitive_questionnaire_answer_is_live_but_redacted_from_default_persistence() {
    const SECRET: &str = "private-answer-never-in-default-persistence";
    let fixture = Fixture::new();
    let arguments = serde_json::json!({
        "questions": [{
            "id": "detail",
            "header": "Detail",
            "prompt": "Supply the private implementation detail",
            "allow_free_form": true
        }],
        "sensitivity": "sensitive"
    })
    .to_string();
    let mut question = tool_call_fragments(0, "sensitive-question-call", "ask_user", &arguments);
    question.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(question),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "Used the private detail without repeating it.".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let redactor = DefaultRedactor::new();
    let (interaction, mut requests) =
        InteractiveInteraction::with_sensitive_value_sink(Arc::new(redactor.clone()));
    let mut request = fixture.request(HostSurface::Terminal);
    request.runtime.provider = Some(provider.clone());
    request.runtime.approval = Some(Arc::new(DenyAll));
    request.runtime.interaction = Some(Arc::new(interaction));
    request.runtime.persistence_redactor = Some(redactor);
    let host = start(request).await.expect("interactive host");
    let session_id = host.session().id().clone();
    let paths = host.paths().expect("persistent paths").clone();

    let run = host
        .session()
        .run(UserInput::text("ask for the private detail"));
    let answer = async {
        let InteractionNotice::Present(prompt) =
            requests.recv().await.expect("questionnaire presentation")
        else {
            panic!("expected a questionnaire presentation");
        };
        prompt
            .answer(vec![QuestionAnswer::free_form(
                agent_runtime_core::ids::QuestionId::new("detail"),
                SECRET,
            )])
            .expect("typed answer accepted");
    };
    let (result, ()) = tokio::join!(run, answer);
    result.expect("the same turn resumes after the sensitive answer");

    assert!(
        serde_json::to_string(&provider.requests()[1].messages)
            .expect("serializable provider messages")
            .contains(SECRET),
        "the live continuation did not receive the exact answer"
    );
    let protected = std::fs::read(
        paths
            .checkpoint(&session_id)
            .expect("protected checkpoint path"),
    )
    .expect("protected checkpoint");
    assert!(
        !protected
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes()),
        "the protected checkpoint envelope exposed plaintext"
    );
    host.shutdown().await.expect("clean shutdown");

    let snapshot = std::fs::read_to_string(paths.snapshot(&session_id).expect("snapshot path"))
        .expect("redacted snapshot");
    let journal = std::fs::read_to_string(paths.journal(&session_id).expect("journal path"))
        .expect("redacted journal");
    assert!(!snapshot.contains(SECRET), "snapshot leaked the answer");
    assert!(!journal.contains(SECRET), "journal leaked the answer");
    assert!(
        snapshot.contains("[redacted]"),
        "snapshot did not retain an explicit redaction marker"
    );
}

#[tokio::test]
async fn upgrading_a_hosted_session_interrupts_only_the_unfinished_turn() {
    let fixture = Fixture::new();
    let first = start(fixture.request(HostSurface::Headless)).await.unwrap();
    let paths = first.paths().unwrap().clone();
    let mut snapshot = first.session().snapshot();
    first.shutdown().await.unwrap();
    let session_id = snapshot.id.clone();
    snapshot.history = vec![
        agent_runtime_core::content::Message::user("previous question"),
        agent_runtime_core::content::Message::assistant(vec![
            agent_runtime_core::content::ContentPart::text("previous answer"),
        ]),
        UserInput::text("unfinished request").into_message(),
    ];
    // The immutable registry fingerprint changes when embedded skills or tool
    // definitions change. Keep all other persisted activation data intact.
    snapshot
        .extension_state
        .get_mut("runtime.core.live_abilities")
        .unwrap()
        .value["snapshot"] = serde_json::json!("registry-before-upgrade");
    let checkpoint = TurnCheckpoint::accepted(
        TurnId::new("upgrade-interrupted-turn"),
        UserInput::text("unfinished request"),
        snapshot.clone(),
        2,
        Deadline::never(),
        1,
        snapshot.identity.event_seq + 1,
        Timestamp::ZERO,
    )
    .unwrap();
    let checkpoints = SmithCheckpointStore::initialize_with(paths.clone(), test_checkpoint_keys())
        .await
        .unwrap();
    checkpoints.save(&checkpoint).await.unwrap();
    let provider = Arc::new(FakeProvider::text_reply("new answer"));
    let mut request = fixture
        .request(HostSurface::Headless)
        .resume(session_id.clone());
    request.runtime.provider = Some(provider.clone() as Arc<dyn Provider>);
    let resumed = start(request)
        .await
        .expect("upgrade keeps the conversation resumable");
    assert_eq!(
        resumed.session().interrupted_on_resume(),
        Some(&checkpoint.turn)
    );
    assert_eq!(resumed.session().history(), snapshot.history);
    assert!(
        provider.requests().is_empty(),
        "startup must not replay or spend"
    );
    assert!(
        checkpoints
            .load_latest(&session_id)
            .await
            .unwrap()
            .unwrap()
            .state
            .is_terminal()
    );
    resumed
        .session()
        .run(UserInput::text("continue deliberately"))
        .await
        .unwrap();
    assert_eq!(provider.requests().len(), 1);
    resumed.shutdown().await.unwrap();
    let again = start(fixture.request(HostSurface::Headless).resume(session_id))
        .await
        .unwrap();
    assert!(again.session().interrupted_on_resume().is_none());
    assert!(
        again
            .session()
            .history()
            .iter()
            .any(|message| message.joined_text() == "previous answer")
    );
    again.shutdown().await.unwrap();
}
