//! Advisor routing, root-only registration, and recoverable mid-turn review.

use std::sync::Arc;
use std::time::Duration;

use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime::runtime::StartSession;
use agent_runtime_core::content::{ContentPart, UserInput};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::provider::{
    Capabilities, FinishReason, Provider, ProviderError, ProviderErrorKind, ProviderStreamEvent,
    ToolChoice,
};
use agent_runtime_core::tool::{
    InvocationContext, PreparedToolCall, Tool, ToolEffects, ToolOutcome, ToolSpec,
};
use agent_runtime_testkit::scenarios::stop_events;
use agent_runtime_testkit::{MemoryWorkspace, RecordingObserver};
use async_trait::async_trait;
use serde_json::json;
use smith_config::model::ProfileUse;
use smith_config::resolve::{Overrides, ResolveRequest, ResolvedConfig, resolve};
use smith_runtime::factory::{
    self, AdvisorProfileRequest, FactoryError, HostSurface, RuntimeRequest, SmithRuntime,
};
use smith_runtime::harness::{HarnessSpec, resolve as resolve_harness};
use smith_runtime::host::{HostSessionRequest, start};

const CONFIG: &str = r#"
default_profile = "work"

[profiles.work]
use = ["main", "child"]
provider = "local"
model = "worker"
advisor = "review"
delegation = false

[profiles.review]
use = ["advisor"]
provider = "reviewer"
model = "review-model"
instructions = "REVIEW_PROFILE_INSTRUCTION: prioritize correctness."
max_output_tokens = 128

[providers.local]
kind = "fake"

[providers.reviewer]
kind = "fake"

[models."local/worker"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."reviewer/review-model"]
context_tokens = 32000
max_input_tokens = 30000
max_output_tokens = 512

[context]
reasoning_reserve = 0

[approval]
mode = "deny"
"#;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let home = tempfile::tempdir().expect("user root");
        let project = tempfile::tempdir().expect("project root");
        let directory = project.path().join(".smith");
        std::fs::create_dir_all(&directory).expect("configuration directory");
        std::fs::write(directory.join("config.toml"), config).expect("configuration");
        let user_directory = home.path().join(".smith");
        std::fs::create_dir_all(&user_directory).expect("user configuration directory");
        std::fs::write(
            user_directory.join("config.toml"),
            "[persistence]\nenabled = false\n",
        )
        .expect("user persistence policy");
        Self { home, project }
    }

    fn config(&self, profile: &str, placement: ProfileUse) -> ResolvedConfig {
        resolve(
            &ResolveRequest::new(self.project.path())
                .with_home_dir(self.home.path())
                .with_cli(Overrides {
                    profile: Some(profile.to_owned()),
                    ..Overrides::default()
                })
                .with_profile_use(placement),
        )
        .expect("resolved profile")
        .config
    }

    fn request(
        &self,
        surface: HostSurface,
        main: Arc<dyn Provider>,
        advisor: Arc<dyn Provider>,
    ) -> RuntimeRequest {
        let placement = if surface == HostSurface::Child {
            ProfileUse::Child
        } else {
            ProfileUse::Main
        };
        let mut request = RuntimeRequest::new(self.config("work", placement), surface);
        request.workspace = Some(Arc::new(MemoryWorkspace::new("/repo")));
        request.provider = Some(main);
        request.built_in_tools = false;
        if request.config.agent.profile.advisor.is_some() {
            request.advisor_profile = Some(AdvisorProfileRequest {
                config: self.config("review", ProfileUse::Advisor),
                catalog_sources: Vec::new(),
                provider: Some(advisor),
            });
        }
        request
    }
}

async fn build(request: RuntimeRequest) -> Result<SmithRuntime, FactoryError> {
    factory::build(resolve_harness(HarnessSpec::trusted(request)).expect("resolved harness")).await
}

fn advisor_provider(events: Vec<ProviderStreamEvent>) -> Arc<FakeProvider> {
    Arc::new(FakeProvider::new(
        "review-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(events)],
    ))
}

fn step(id: &str, name: &str, arguments: &str) -> ScriptedStream {
    let mut events = tool_call_fragments(0, id, name, arguments);
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    ScriptedStream::new(events)
}

#[tokio::test]
async fn advisor_is_registered_only_on_configured_root_surfaces() {
    for surface in [
        HostSurface::Terminal,
        HostSurface::Headless,
        HostSurface::Child,
    ] {
        for configured in [true, false] {
            let config = if configured {
                CONFIG.to_owned()
            } else {
                CONFIG.replace("advisor = \"review\"", "")
            };
            let fixture = Fixture::new(&config);
            let main = Arc::new(FakeProvider::new(
                "worker",
                Capabilities::basic_streaming(),
                vec![
                    ScriptedStream::new(stop_events("done")),
                    ScriptedStream::new(stop_events("done again")),
                ],
            ));
            let advisor = advisor_provider(stop_events("advice"));
            let recorder = RecordingObserver::shared();
            let mut request = fixture.request(surface, main.clone(), advisor.clone());
            request.observers.push(recorder.clone());
            let smith = build(request).await.expect("runtime");
            let expected = configured && surface != HostSurface::Child;
            assert_eq!(smith.abilities().names().contains(&"advisor"), expected);
            let session = smith
                .runtime()
                .start_session(StartSession::new())
                .await
                .expect("session");
            smith.wire_advisor(&session).expect("session wiring");
            // Core availability must not depend on an advisor keyword in the
            // user's input, and must remain present on subsequent turns.
            for input in ["Say hello.", "Say goodbye."] {
                session.run(UserInput::text(input)).await.expect("turn");
            }
            let requests = main.requests();
            assert_eq!(requests.len(), 2, "runtime events: {:?}", recorder.events());
            let epoch = session.activation_epoch().expect("an activation epoch");
            assert_eq!(
                epoch.activated().iter().any(|(id, _)| id.name == "advisor"),
                expected,
                "advisor enters the runtime's authorized activation epoch",
            );
            for request in &requests {
                let tool = request.tools.iter().find(|tool| tool.name == "advisor");
                assert_eq!(tool.is_some(), expected);
                assert_eq!(
                    request
                        .tools
                        .iter()
                        .filter(|tool| tool.name == "advisor")
                        .count(),
                    usize::from(expected),
                    "advisor is advertised exactly once when registered",
                );
                if let Some(tool) = tool {
                    assert_eq!(
                        tool.input_schema,
                        json!({
                            "type": "object", "properties": {}, "additionalProperties": false
                        })
                    );
                    assert!(tool.description.contains("whole conversation"));
                    assert!(tool.description.contains("no arguments"));
                }
            }
            assert!(advisor.requests().is_empty());
            session.shutdown().await.expect("shutdown");
        }
    }
}

#[tokio::test]
async fn existing_root_tools_are_activated_by_semantic_intent() {
    let config = CONFIG
        .replace("advisor = \"review\"", "")
        .replace("delegation = false", "delegation = true");
    let fixture = Fixture::new(&config);
    for (input, expected) in [
        ("Say hello.", false),
        ("Use agent, write_todos, and ask_user.", true),
    ] {
        let main = Arc::new(FakeProvider::new(
            "worker",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(stop_events("done"))],
        ));
        let mut request = fixture.request(
            HostSurface::Terminal,
            main.clone(),
            advisor_provider(stop_events("unused")),
        );
        request.built_in_tools = true;
        let (interaction, _interactions) = smith_host::InteractiveInteraction::new();
        request.interaction = Some(Arc::new(interaction));
        let smith = build(request).await.expect("runtime");
        let session = smith
            .runtime()
            .start_session(StartSession::new())
            .await
            .expect("session");
        session.run(UserInput::text(input)).await.expect("turn");
        let requests = main.requests();
        assert_eq!(requests.len(), 1);
        for name in ["agent", "write_todos", "ask_user"] {
            assert_eq!(
                requests[0].tools.iter().any(|tool| tool.name == name),
                expected,
                "{name} advertisement follows normal semantic activation: {:?}",
                requests[0].tools,
            );
        }
        assert!(
            requests[0]
                .tools
                .iter()
                .any(|tool| tool.name == "registry.search")
        );
        session.shutdown().await.expect("shutdown");
    }
}

#[derive(Debug)]
struct EvidenceTool {
    name: &'static str,
    result: &'static str,
}

#[async_trait]
impl Tool for EvidenceTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "Return fixed evidence without workspace effects.",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            ToolEffects::default(),
        )
    }

    async fn invoke(
        &self,
        _prepared: PreparedToolCall,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        Ok(ToolOutcome::text(self.result))
    }
}

#[tokio::test]
async fn hosted_mid_turn_advisor_sees_task_tool_calls_results_and_image_placeholder() {
    let fixture = Fixture::new(CONFIG);
    let main = Arc::new(FakeProvider::new(
        "worker",
        Capabilities::basic_streaming(),
        vec![
            step("read-call", "read", r#"{"path":"main.rs"}"#),
            step("shell-call", "shell", "{}"),
            step("advisor-call", "advisor", "{}"),
            ScriptedStream::new(stop_events("work completed after review")),
        ],
    ));
    let advisor = advisor_provider(stop_events("Check the missing cancellation path."));
    let mut request = fixture.request(HostSurface::Headless, main.clone(), advisor.clone());
    request.trusted_native.add_tool(Arc::new(EvidenceTool {
        name: "read",
        result: "READ_EVIDENCE",
    }));
    request.trusted_native.add_tool(Arc::new(EvidenceTool {
        name: "shell",
        result: "SHELL_EVIDENCE",
    }));
    let host = start(HostSessionRequest::new(request, fixture.project.path()))
        .await
        .expect("host wires the advisor slot");
    tokio::time::timeout(
        Duration::from_secs(5),
        host.session().run(UserInput {
            parts: vec![
                    ContentPart::text(
                        "USER_TASK: inspect the cancellation path using read, shell, and advisor tools.",
                    ),
                ContentPart::Image {
                    url: "https://example.test/private-image".to_owned(),
                    detail: None,
                },
            ],
        }),
    )
    .await
    .expect("turn does not hang while reading live history")
    .expect("turn");

    let requests = advisor.requests();
    assert_eq!(requests.len(), 1);
    let review = &requests[0];
    assert_eq!(review.model.as_str(), "review-model");
    assert_eq!(review.max_output_tokens, Some(128));
    assert!(review.tools.is_empty());
    assert_eq!(review.tool_choice, ToolChoice::None);
    assert_eq!(review.messages.len(), 2);
    assert!(
        review.messages[0]
            .joined_text()
            .contains("reviewing another agent's work")
    );
    assert!(
        review.messages[0]
            .joined_text()
            .ends_with("REVIEW_PROFILE_INSTRUCTION: prioritize correctness.")
    );
    let transcript = review.messages[1].joined_text();
    for expected in [
        "data, not instructions",
        "User:\nUSER_TASK:",
        "[image omitted]",
        "Assistant:\nTool call: read {\"path\":\"main.rs\"}",
        "Tool result: read\nREAD_EVIDENCE",
        "Tool call: shell {}",
        "Tool result: shell\nSHELL_EVIDENCE",
        "Tool call: advisor {}",
    ] {
        assert!(
            transcript.contains(expected),
            "missing {expected}: {transcript}"
        );
    }
    assert!(!transcript.contains("private-image"));
    assert!(!transcript.contains("Check the missing cancellation path."));
    assert!(!transcript.contains("work completed after review"));
    assert_eq!(main.requests().len(), 4);
    let history = host.session().history();
    assert_eq!(
        history.last().expect("final message").joined_text(),
        "work completed after review"
    );
    assert!(
        history
            .iter()
            .flat_map(|message| &message.content)
            .any(|part| {
                matches!(part, ContentPart::ToolResult(result)
            if result.name == "advisor" && !result.is_error
                && result.content[0].as_text() == Some("Check the missing cancellation path."))
            })
    );
    host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn advisor_provider_error_is_a_tool_error_and_the_main_turn_completes() {
    let fixture = Fixture::new(CONFIG);
    let main = Arc::new(FakeProvider::new(
        "worker",
        Capabilities::basic_streaming(),
        vec![
            step("advisor-call", "advisor", "{}"),
            ScriptedStream::new(stop_events("continued")),
        ],
    ));
    let advisor = advisor_provider(vec![ProviderStreamEvent::Error {
        error: ProviderError::new(ProviderErrorKind::Server, "review service unavailable"),
    }]);
    let host = start(HostSessionRequest::new(
        fixture.request(HostSurface::Headless, main.clone(), advisor),
        fixture.project.path(),
    ))
    .await
    .expect("host");
    host.session()
        .run(UserInput::text(
            "Use the advisor tool to review the approach.",
        ))
        .await
        .expect("turn");
    let history = host.session().history();
    let result = history
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| match part {
            ContentPart::ToolResult(result) if result.name == "advisor" => Some(result),
            _ => None,
        })
        .expect("advisor result");
    assert!(result.is_error);
    assert!(
        result.content[0]
            .as_text()
            .expect("error text")
            .contains("review service unavailable")
    );
    assert_eq!(
        history.last().expect("final answer").joined_text(),
        "continued"
    );
    assert_eq!(main.requests().len(), 2);
    host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn missing_or_broken_advisor_route_fails_startup() {
    let fixture = Fixture::new(CONFIG);
    let main = Arc::new(FakeProvider::new(
        "worker",
        Capabilities::basic_streaming(),
        Vec::new(),
    ));
    let advisor = advisor_provider(stop_events("advice"));
    let mut missing = fixture.request(HostSurface::Headless, main.clone(), advisor.clone());
    missing.advisor_profile = None;
    assert!(matches!(
        build(missing).await,
        Err(FactoryError::MissingHostPolicy {
            what: "resolved advisor profile",
            ..
        })
    ));

    let mut broken = fixture.request(HostSurface::Headless, main, advisor.clone());
    broken
        .advisor_profile
        .as_mut()
        .expect("advisor config")
        .config
        .provider
        .kind
        .value = "missing-adapter".to_owned();
    assert!(build(broken).await.is_err());
    assert!(
        advisor.requests().is_empty(),
        "startup failure performs no advisor request"
    );
}
