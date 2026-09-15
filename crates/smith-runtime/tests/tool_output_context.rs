//! Production host composition: a long single task keeps recoverable text out
//! of the inline working set. No summary or provider network calls are used.
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime_core::approval::AllowAll;
use agent_runtime_core::artifact::{ArtifactId, ArtifactRead, ArtifactStore};
use agent_runtime_core::content::{ContentPart, Message, UserInput};
use agent_runtime_core::ids::SessionId;
use agent_runtime_core::provider::{Capabilities, FinishReason, Provider, ProviderStreamEvent};
use agent_runtime_core::tool::ToolOutcome;
use smith_config::resolve::{ResolveRequest, resolve};
use smith_host::ProjectWorkspace;
use smith_runtime::checkpoint::{CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError};
use smith_runtime::factory::{HostSurface, RuntimeRequest};
use smith_runtime::host::{HostSessionRequest, start};
use std::collections::BTreeSet;
use std::sync::Arc;

const CONFIG: &str = r#"
default_profile = "dev"
[profiles.dev]
provider = "local"
model = "example-model"
[providers.local]
kind = "fake"
[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#;
#[derive(Debug)]
struct Keys;
impl CheckpointKeyProvider for Keys {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        Ok(CheckpointKey::new([0x37; 32]))
    }
}
struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            home: tempfile::tempdir().unwrap(),
            project: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(fixture.project.path().join(".smith")).unwrap();
        std::fs::write(fixture.project.path().join(".smith/config.toml"), CONFIG).unwrap();
        fixture
    }
    fn request(
        &self,
        provider: Arc<dyn Provider>,
        resume: Option<SessionId>,
    ) -> HostSessionRequest {
        let config =
            resolve(&ResolveRequest::new(self.project.path()).with_home_dir(self.home.path()))
                .unwrap()
                .config;
        let runtime = RuntimeRequest {
            provider: Some(provider),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(self.project.path()).unwrap(),
            )),
            approval: Some(Arc::new(AllowAll)),
            ..RuntimeRequest::new(config, HostSurface::Headless)
        };
        let request =
            HostSessionRequest::new(runtime, self.project.path()).checkpoint_keys(Arc::new(Keys));
        match resume {
            Some(id) => request.resume(id),
            None => request,
        }
    }
}
fn tool_stream(id: &str, name: &str, args: serde_json::Value) -> ScriptedStream {
    let mut events = tool_call_fragments(0, id, name, &args.to_string());
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    ScriptedStream::new(events)
}
fn done() -> ScriptedStream {
    ScriptedStream::new(vec![
        ProviderStreamEvent::TextDelta {
            text: "done".into(),
        },
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        },
    ])
}
fn artifacts(messages: &[Message]) -> BTreeSet<String> {
    messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|part| {
            let ContentPart::ToolResult(result) = part else {
                return None;
            };
            result
                .content
                .iter()
                .filter_map(ContentPart::as_text)
                .find_map(|text| {
                    text.strip_prefix("[artifact id=")
                        .and_then(|rest| rest.split_whitespace().next())
                        .map(str::to_owned)
                })
        })
        .collect()
}
async fn exact(store: &dyn ArtifactStore, session: &SessionId, id: &str) -> ToolOutcome {
    let mut bytes = Vec::new();
    let mut offset = 0;
    loop {
        let page = store
            .read(ArtifactRead {
                session: session.clone(),
                id: ArtifactId::new(id).unwrap(),
                offset,
                limit: 2048,
            })
            .await
            .unwrap();
        bytes.extend(page.bytes);
        match page.next_offset {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn many_medium_tool_results_in_one_task_survive_restart_without_replaying_effects() {
    const CALLS: usize = 18;
    let fixture = Fixture::new();
    let text = format!(
        "BEGIN\n{}\nerror: MIDDLE_DIAGNOSTIC_EXCERPT\n{}\nEND\n",
        "passing 0123456789abcdef\n".repeat(300),
        "passing 0123456789abcdef\n".repeat(300)
    );
    assert!((8193..32768).contains(&text.len()));
    std::fs::write(fixture.project.path().join("medium.log"), &text).unwrap();
    let streams = (0..CALLS)
        .map(|index| {
            tool_stream(
                &format!("call-{index}"),
                "shell",
                serde_json::json!({"command": "cat medium.log; printf x >> executions.txt"}),
            )
        })
        .chain([done()])
        .collect();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        streams,
    ));
    let host = start(fixture.request(provider.clone(), None))
        .await
        .unwrap();
    assert_eq!(
        host.runtime().policy().tool_output_context.inline_bytes,
        8192
    );
    assert_eq!(host.runtime().policy().output_limit, 65536);
    assert!(host.runtime().policy().artifact_offloading);
    host.session()
        .run(UserInput::text(
            "Run the test work. Keep the exact constraint: DO_NOT_CHANGE_PUBLIC_API.",
        ))
        .await
        .unwrap();
    let requests = provider.requests();
    assert_eq!(requests.len(), CALLS + 1);
    let final_wire = serde_json::to_string(&requests.last().unwrap().messages).unwrap();
    assert!(final_wire.contains("DO_NOT_CHANGE_PUBLIC_API"));
    assert!(final_wire.contains("MIDDLE_DIAGNOSTIC_EXCERPT"));
    assert!(final_wire.contains("discover it with registry.search"));
    assert!(
        final_wire.len() < CALLS * text.len() / 2,
        "inline wire bytes {} vs raw captured text {}",
        final_wire.len(),
        CALLS * text.len()
    );
    let ids = artifacts(&requests.last().unwrap().messages);
    assert_eq!(
        ids.len(),
        CALLS,
        "every completed tool exchange remains paired with its retrievable result"
    );
    let session_id = host.session().id().clone();
    let saved_history = host.session().history();
    let store = host.runtime().artifact_store().unwrap();
    for id in &ids {
        let outcome = exact(store.as_ref(), &session_id, id).await;
        assert_eq!(outcome.value["exit_code"], 0);
        assert_eq!(outcome.value["truncated"], false);
        assert!(
            !outcome.is_error,
            "a heuristic 'error' excerpt must not change the actual outcome"
        );
        let original = outcome
            .content
            .as_inline()
            .unwrap()
            .iter()
            .filter_map(ContentPart::as_text)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(original, text.trim_end());
    }
    let work_before_restart = std::fs::read(fixture.project.path().join("executions.txt")).unwrap();
    assert_eq!(work_before_restart, vec![b'x'; CALLS]);
    host.shutdown().await.unwrap();
    drop(host);
    // Reopen through the actual protected session loader and consume a bounded
    // artifact page through the actual advertised tool, not a test-only reader.
    let id = ids.first().unwrap();
    let resumed_provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            tool_stream(
                "discover-after-restart",
                "registry.search",
                serde_json::json!({"query": smith_runtime::tool_output::ARTIFACT_DISCOVERY_QUERY, "max_results": 1}),
            ),
            tool_stream(
                "read-after-restart",
                "artifact.read",
                serde_json::json!({"id": id, "limit": 65536}),
            ),
            done(),
        ],
    ));
    let resumed = start(fixture.request(resumed_provider.clone(), Some(session_id.clone())))
        .await
        .unwrap();
    assert!(
        resumed_provider.requests().is_empty(),
        "opening a saved conversation must not call a provider"
    );
    assert_eq!(resumed.session().history(), saved_history);
    for id in &ids {
        assert_eq!(
            exact(
                resumed.runtime().artifact_store().unwrap().as_ref(),
                &session_id,
                id
            )
            .await
            .value["truncated"],
            false
        );
    }
    resumed
        .session()
        .run(UserInput::text(
            "Read only the needed evidence from the saved artifact.",
        ))
        .await
        .unwrap();
    let resumed_requests = resumed_provider.requests();
    assert_eq!(resumed_requests.len(), 3);
    assert!(
        !resumed_requests[0]
            .tools
            .iter()
            .any(|tool| tool.name == "artifact.read"),
        "the fixture must exercise actual lazy discovery, not an eagerly exposed tool"
    );
    assert!(
        resumed_requests[1]
            .tools
            .iter()
            .any(|tool| tool.name == "artifact.read"),
        "discovery must advertise the reader at the next provider boundary: {:?}",
        resumed_requests[1].messages
    );
    let page = resumed_requests[2]
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|part| {
            let ContentPart::ToolResult(result) = part else {
                return None;
            };
            (result.name == "artifact.read").then_some(result)
        })
        .expect("an actual bounded artifact result");
    assert!(!page.is_error, "artifact tool failed: {page:?}");
    let value: serde_json::Value =
        serde_json::from_str(page.content[0].as_text().unwrap()).unwrap();
    assert_eq!(value["next_offset"], 2048);
    assert!(value["content"].as_str().unwrap().len() <= 2048);
    assert_eq!(
        std::fs::read(fixture.project.path().join("executions.txt")).unwrap(),
        work_before_restart
    );
    resumed.shutdown().await.unwrap();
}

#[tokio::test]
async fn no_artifact_store_does_not_claim_recoverable_offloading() {
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![done()],
    ));
    let mut request = fixture.request(provider, None);
    request.runtime.config.persistence.enabled.value = false;
    let host = start(request).await.unwrap();
    assert!(!host.runtime().policy().artifact_offloading);
    assert!(
        !host
            .runtime()
            .policy()
            .tools
            .iter()
            .any(|name| name == "artifact.read")
    );
    host.session().run(UserInput::text("reply")).await.unwrap();
    host.shutdown().await.unwrap();
}
