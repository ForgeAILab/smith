use super::*;
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime_core::content::{ContentPart, ReasoningProducer};
use agent_runtime_core::provider::{Capabilities, FinishReason, ProviderStreamEvent};
use smith_config::resolve::Overrides;

const REASONING_TEXT: &str = "first-provider-private-reasoning";
const REASONING_SIGNATURE: &str = "first-provider-signed-reasoning-token";
const TOOL_TEXT: &str = "read evidence survives the provider switch";

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            format!(
                r#"{LOCAL_COMMAND_CONFIG}

[providers.other]
kind = "fake"

[models."other/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#,
            ),
        )
        .expect("config");
        std::fs::write(project.path().join("evidence.txt"), TOOL_TEXT).expect("tool evidence");
        Self { home, project }
    }

    async fn start(
        &self,
        provider: Arc<FakeProvider>,
        resume: Option<SessionId>,
        overrides: Overrides,
    ) -> HostSession {
        let mut config = resolve(
            &ResolveRequest::new(self.project.path())
                .with_home_dir(self.home.path())
                .with_cli(overrides),
        )
        .expect("resolution")
        .config;
        config.persistence.enabled.value = true;
        let runtime = RuntimeRequest {
            provider: Some(provider),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(self.project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(agent_runtime_core::approval::AllowAll)),
            ..RuntimeRequest::new(config, HostSurface::Terminal)
        };
        let mut request = HostSessionRequest::new(runtime, self.project.path())
            .checkpoint_keys(Arc::new(TestCheckpointKeys));
        if let Some(session) = resume {
            request = request.resume(session);
        }
        let host = Box::pin(smith_runtime::host::start(request))
            .await
            .expect("host");
        host.set_goal_continuation_enabled(false);
        host
    }
}

async fn finish_turn(host: &HostSession, prompt: &str, answer: &str) {
    let turn = host.session().send(UserInput::text(prompt)).expect("turn");
    tokio::time::timeout(Duration::from_secs(10), Box::pin(turn.completed()))
        .await
        .expect("turn completed");
    assert_eq!(
        host.snapshot()
            .history
            .last()
            .expect("committed answer")
            .joined_text(),
        answer,
    );
}

#[tokio::test]
async fn resumed_provider_switch_omits_signed_reasoning_and_completes() {
    let fixture = Fixture::new();
    let mut events = vec![ProviderStreamEvent::ReasoningDelta {
        text: REASONING_TEXT.to_owned(),
        redacted: false,
        signature: Some(REASONING_SIGNATURE.to_owned()),
    }];
    events.extend(tool_call_fragments(
        0,
        "read-evidence",
        "read",
        r#"{"path":"evidence.txt"}"#,
    ));
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let first_provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(events),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "first provider finished".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let host = Box::pin(fixture.start(first_provider.clone(), None, Overrides::default())).await;
    Box::pin(finish_turn(
        &host,
        "Read the evidence",
        "first provider finished",
    ))
    .await;
    let reasoning = ContentPart::Reasoning {
        text: REASONING_TEXT.to_owned(),
        redacted: false,
        signature: Some(REASONING_SIGNATURE.to_owned()),
        producer: Some(ReasoningProducer {
            provider: "local".to_owned(),
            model: ModelId::new("example-model"),
        }),
    };
    let first_requests = first_provider.requests();
    assert_eq!(first_requests.len(), 2);
    assert!(
        first_requests[1]
            .messages
            .iter()
            .any(|message| message.content.contains(&reasoning)),
        "the first provider reuses its own signed reasoning after the tool call",
    );

    let session = host.session().id().clone();
    let mut selection = Selection::default();
    let mut resume = None;
    apply_palette_command(
        &mut selection,
        &mut resume,
        session.as_str().to_owned(),
        SelectionCommand::Model {
            provider: Some("other".to_owned()),
            model: "example-model".to_owned(),
        },
    );
    Box::pin(host.shutdown()).await.expect("shutdown");

    let second_provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: "second provider finished".to_owned(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])],
    ));
    let resumed = Box::pin(fixture.start(
        second_provider.clone(),
        resume.map(SessionId::new),
        selection.overrides(),
    ))
    .await;
    assert_eq!(resumed.session().id(), &session);
    assert!(
        resumed
            .snapshot()
            .history
            .iter()
            .any(|message| message.content.contains(&reasoning)),
        "signed reasoning remains in the persisted canonical history",
    );
    Box::pin(finish_turn(
        &resumed,
        "Continue after switching",
        "second provider finished",
    ))
    .await;

    let requests = second_provider.requests();
    assert_eq!(requests.len(), 1);
    let request = serde_json::to_string(&requests[0]).expect("recorded provider request");
    assert!(!request.contains(REASONING_TEXT), "{request}");
    assert!(!request.contains(REASONING_SIGNATURE), "{request}");
    assert!(request.contains("Read the evidence"), "{request}");
    assert!(request.contains("first provider finished"), "{request}");
    assert!(request.contains("Continue after switching"), "{request}");
    let mut parts = requests[0]
        .messages
        .iter()
        .flat_map(|message| &message.content);
    assert!(
        parts
            .clone()
            .all(|part| !matches!(part, ContentPart::Reasoning { .. }))
    );
    assert!(
        parts
            .clone()
            .any(|part| matches!(part, ContentPart::ToolCall(call)
        if call.id.as_str() == "read-evidence" && call.name == "read"))
    );
    assert!(parts.any(|part| {
        let ContentPart::ToolResult(result) = part else {
            return false;
        };
        result.call_id.as_str() == "read-evidence"
            && !result.is_error
            && result
                .content
                .iter()
                .any(|part| part.as_text().is_some_and(|text| text.contains(TOOL_TEXT)))
    }));
    Box::pin(resumed.shutdown())
        .await
        .expect("resumed shutdown");
}
