use super::*;
use agent_runtime::provider::fake::tool_call_fragments;
use agent_runtime::runtime::StartSession;
use agent_runtime_core::content::{ContentPart, UserInput};
use serde_json::json;

fn scripted(calls: Vec<(&str, serde_json::Value)>) -> Arc<FakeProvider> {
    let mut streams = Vec::new();
    for (index, (name, args)) in calls.into_iter().enumerate() {
        let mut events = tool_call_fragments(0, &format!("call-{index}"), name, &args.to_string());
        events.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        streams.push(ScriptedStream::new(events));
    }
    streams.push(ScriptedStream::new(vec![
        ProviderStreamEvent::TextDelta {
            text: "done".into(),
        },
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ]));
    Arc::new(FakeProvider::new(
        "fake",
        Capabilities::basic_streaming(),
        streams,
    ))
}

fn host_request(
    root: &std::path::Path,
    provider: Arc<FakeProvider>,
    mode: ApprovalMode,
) -> RuntimeRequest {
    let mut config = resolved_config();
    config.model_limits.context_tokens = Some(sourced(32_768));
    config.model_limits.max_input_tokens = Some(sourced(24_000));
    config.model_limits.max_output_tokens = Some(sourced(4_096));
    config.model_input_modalities = Some(sourced(vec![
        smith_config::catalog::CatalogModality::Text,
        smith_config::catalog::CatalogModality::Image,
    ]));
    config.approval.mode = sourced(mode);
    config.persistence.enabled = sourced(false);
    config.agent.profile.delegation = sourced(false);
    let mut request = RuntimeRequest::new(config, HostSurface::Headless);
    request.workspace = Some(Arc::new(ProjectWorkspace::new(root).unwrap()));
    request.provider = Some(provider);
    request
}

fn results(provider: &FakeProvider) -> Vec<agent_runtime_core::content::ToolResultBlock> {
    provider
        .requests()
        .last()
        .unwrap()
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| {
            if let ContentPart::ToolResult(result) = part {
                Some(result.clone())
            } else {
                None
            }
        })
        .collect()
}

#[tokio::test]
async fn host_filesystem_allow_all_reads_lists_searches_and_edits_external_files() {
    let root = tempfile::tempdir().unwrap();
    let host = tempfile::tempdir().unwrap();
    let file = host.path().join("notes.txt");
    std::fs::write(&file, "original host content\n").unwrap();
    std::fs::write(root.path().join("local.txt"), "project content\n").unwrap();
    let created = host.path().join("new.txt");
    let fake = scripted(vec![
        ("read", json!({"path":file})),
        (
            "edit",
            json!({"path":file, "operation":"replace", "old_string":"original", "new_string":"updated"}),
        ),
        ("search", json!({"path":host.path(), "pattern":"updated"})),
        ("list", json!({"path":host.path()})),
        ("list", json!({})),
        ("read", json!({"path":"local.txt"})),
        (
            "edit",
            json!({"path":created, "operation":"create", "new_string":"new host content"}),
        ),
        ("read", json!({"path":created})),
        ("edit", json!({"path":created, "operation":"delete"})),
    ]);
    let runtime = build(
        crate::harness::resolve(crate::harness::HarnessSpec::trusted(host_request(
            root.path(),
            fake.clone(),
            ApprovalMode::AllowAll,
        )))
        .unwrap(),
    )
    .await
    .unwrap();
    let session = runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap();
    session
        .run(UserInput::text("work with host files"))
        .await
        .unwrap();
    let results = results(&fake);
    assert_eq!(results.len(), 9);
    for result in &results {
        assert!(!result.is_error, "{result:?}");
    }
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "updated host content\n"
    );
    assert!(!created.exists());
    assert!(
        results[4]
            .content
            .iter()
            .filter_map(ContentPart::as_text)
            .any(|text| text.contains("local.txt") && !text.contains("Volumes/"))
    );
}

#[tokio::test]
async fn host_filesystem_external_image_reaches_the_next_provider_request() {
    let root = tempfile::tempdir().unwrap();
    let host = tempfile::tempdir().unwrap();
    let path = host.path().join("reference.png");
    image::RgbImage::new(7, 5).save(&path).unwrap();
    let expected = smith_tools::prepare_image(
        &std::fs::read(&path).unwrap(),
        &smith_tools::ImageLimits::for_provider(smith_tools::ImageProvider::Other),
    )
    .unwrap();
    let fake = scripted(vec![("read", json!({"path":path}))]);
    let runtime = build(
        crate::harness::resolve(crate::harness::HarnessSpec::trusted(host_request(
            root.path(),
            fake.clone(),
            ApprovalMode::AllowAll,
        )))
        .unwrap(),
    )
    .await
    .unwrap();
    runtime
        .runtime()
        .start_session(StartSession::new())
        .await
        .unwrap()
        .run(UserInput::text("inspect the image"))
        .await
        .unwrap();
    let results = results(&fake);
    assert!(!results[0].is_error);
    assert!(
        results[0].content.iter().any(
            |part| matches!(part, ContentPart::Image { url, .. } if url == &expected.data_url)
        )
    );
}

#[tokio::test]
async fn host_filesystem_external_read_prompts_and_requires_the_users_decision() {
    use smith_host::approval::{InteractiveApproval, PromptScope};
    for (allow, via_symlink) in [(false, false), (true, false), (true, true)] {
        let root = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        let path = host.path().join("private.txt");
        std::fs::write(&path, "EXTERNAL_CONTENT_CANARY").unwrap();
        let requested_path = if via_symlink {
            let link = root.path().join("reference.txt");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            link
        } else {
            path.clone()
        };
        let fake = scripted(vec![("read", json!({"path":requested_path}))]);
        let (approval, mut requests) = InteractiveApproval::new(4);
        let mut request = host_request(root.path(), fake.clone(), ApprovalMode::Ask);
        request.approval = Some(Arc::new(approval));
        let runtime =
            build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
                .await
                .unwrap();
        let session = runtime
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap();
        let run =
            tokio::spawn(async move { session.run(UserInput::text("read the host file")).await });
        let prompt = tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "no approval prompt; run finished={}, results={:?}",
                    run.is_finished(),
                    results(&fake)
                )
            })
            .unwrap();
        assert_eq!(prompt.tool(), "read");
        assert_eq!(fake.requests().len(), 1, "no result before the decision");
        assert_eq!(
            prompt.prepared().arguments()["path"],
            path.canonicalize().unwrap().to_string_lossy().as_ref()
        );
        assert!(
            matches!(prompt.prepared().resource(), SecurityResource::Filesystem { mount, .. } if mount == "/")
        );
        if allow {
            prompt.allow(PromptScope::Once);
        } else {
            prompt.deny("user denied host access");
        }
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let results = results(&fake);
        assert_eq!(results[0].is_error, !allow);
        let text = results[0]
            .content
            .iter()
            .filter_map(ContentPart::as_text)
            .collect::<String>();
        assert_eq!(text.contains("EXTERNAL_CONTENT_CANARY"), allow);
    }
}

#[test]
fn host_filesystem_allow_all_keeps_plan_profiles_read_only() {
    let root = tempfile::tempdir().unwrap();
    let fake = scripted(vec![]);
    let mut request = host_request(root.path(), fake, ApprovalMode::AllowAll);
    request.config.agent = agent(AgentPosture::Plan);
    let names = capabilities::tools(&request)
        .iter()
        .map(|tool| tool.spec().name)
        .collect::<Vec<_>>();
    assert!(names.contains(&"read".into()));
    assert!(!names.contains(&"edit".into()));
    assert!(!names.contains(&"shell".into()));
}

#[tokio::test]
async fn questionnaire_custom_answer_and_discussion_reach_the_active_runtime() {
    use agent_runtime_core::ids::QuestionId;
    use agent_runtime_core::interaction::QuestionAnswer;
    use smith_host::{InteractionNotice, InteractiveInteraction};
    for discuss in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let fake = scripted(vec![(
            "ask_user",
            json!({"questions":[{
                "id":"direction", "header":"Direction", "prompt":"Which direction?",
                "choices":[{"id":"a", "label":"A"}], "allow_free_form":false
            }]}),
        )]);
        let (broker, mut notices) = InteractiveInteraction::new();
        let mut request = host_request(root.path(), fake.clone(), ApprovalMode::AllowAll);
        request.surface = HostSurface::Terminal;
        request.interaction = Some(Arc::new(broker));
        let runtime =
            build(crate::harness::resolve(crate::harness::HarnessSpec::trusted(request)).unwrap())
                .await
                .unwrap();
        let session = runtime
            .runtime()
            .start_session(StartSession::new())
            .await
            .unwrap();
        let turn = session
            .send(UserInput::text("ask a question then help me choose"))
            .unwrap();
        let InteractionNotice::Present(prompt) =
            tokio::time::timeout(Duration::from_secs(5), notices.recv())
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "question not presented; results={:?}; history={:?}",
                        results(&fake),
                        session.history()
                    )
                })
                .unwrap()
        else {
            panic!("a question should open");
        };
        assert!(prompt.request().questionnaire_payload().questions()[0].allows_free_form());
        if discuss {
            session
                .steer_current_turn(
                    Some(turn.id()),
                    UserInput::text("Before choosing an answer, explain the tradeoffs"),
                )
                .expect("a waiting question accepts discussion steering");
            prompt.discuss("explain the tradeoffs").unwrap();
        } else {
            prompt
                .answer(vec![QuestionAnswer::free_form(
                    QuestionId::new("direction"),
                    "my own direction",
                )])
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(5), turn.completed())
            .await
            .unwrap();
        let requests = fake.requests();
        let last = requests.last().unwrap();
        let text = last
            .messages
            .iter()
            .map(Message::joined_text)
            .collect::<String>();
        if discuss {
            assert!(text.contains("explain the tradeoffs"), "{text}");
            let response = &results(&fake)[0];
            assert!(
                response
                    .content
                    .iter()
                    .filter_map(ContentPart::as_text)
                    .any(|text| text.contains("declined")),
                "{response:?}"
            );
        } else {
            let response = &results(&fake)[0];
            assert!(
                response
                    .content
                    .iter()
                    .filter_map(ContentPart::as_text)
                    .any(|text| text.contains("my own direction")),
                "{response:?}"
            );
        }
    }
}
