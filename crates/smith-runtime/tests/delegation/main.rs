//! Direct-child delegation through the one factory (harness tasks 7.1–7.3).
//!
//! Children are composed by [`SmithChildFactory`] through the same policy as
//! the parent, managed root-only through the shared runtime's coordinator,
//! and their protected results are admitted through Runtime's attributed
//! child-completion turn when the parent is idle.
#![allow(deprecated)] // Exercises the documented protocol-v1 embedding adapter.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use agent_runtime::ability::descriptor::RiskLevel;
use agent_runtime::ability::{Ability, ToolAbility};
use agent_runtime::delegation::DELEGATION_PERMISSION;
use agent_runtime::delegation::{
    CHILD_CATALOG_NAMESPACE, ChildDurability, ChildState, ChildTaskOutcome, DurableChildCatalog,
    SpawnOutcome,
};
use agent_runtime::provider::fake::{
    FakeProvider, ScriptedStream, tool_call_fragments, usage_event,
};
use agent_runtime::registry::Permission;
use agent_runtime::runtime::StartSession;
use agent_runtime_core::artifact::{
    ArtifactError, ArtifactRead, ArtifactStore, MAX_ARTIFACT_READ_BYTES,
};
use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::cancel::Cancellation;
use agent_runtime_core::clock::{Deadline, SystemClock};
use agent_runtime_core::content::{ContentPart, UserInput};
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::event::{ChildPhase, RuntimeEvent};
use agent_runtime_core::ids::{RequestId, SessionId, ToolCallId};
use agent_runtime_core::provider::{
    Capabilities, FinishReason, ModelDescriptor, Provider, ProviderCallContext, ProviderError,
    ProviderRequest, ProviderStream, ProviderStreamEvent,
};
use agent_runtime_core::store::SessionStore;
use agent_runtime_core::tool::{InvocationContext, PreparationContext, Tool, ToolOutcome};
use agent_runtime_testkit::{InMemoryCheckpointStore, InMemorySessionStore, MemoryWorkspace};
use async_trait::async_trait;
use futures_util::StreamExt;
use smith_config::model::ProfileUse;
use smith_config::resolve::{Overrides, ResolveRequest, ResolvedConfig, resolve};
use smith_host::ProjectWorkspace;
use smith_runtime::artifact::SmithArtifactStore;
use smith_runtime::delegation::{
    AGENT_TOOL_NAME, AgentTool, AgentToolProfile, DelegationWaitPolicy, profile_route_key,
    wire_delegation,
};
use smith_runtime::factory::{self, ChildProfileRequest, HostSurface, RuntimeRequest};
use smith_runtime::project_instructions::ProjectInstructionsSnapshot;

mod agent_tool;
mod artifacts;
mod authority;
mod child_interactions;
mod completion;
mod durable_children;
mod model_routes;
mod profile_selection;

const FAKE_CONFIG: &str = r#"
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

[approval]
mode = "allow-all"
"#;

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("a user root");
        let project = tempfile::tempdir().expect("a project root");
        let dir = project.path().join(".smith");
        std::fs::create_dir_all(&dir).expect("a project `.smith`");
        std::fs::write(dir.join("config.toml"), FAKE_CONFIG).expect("a project config");
        Self { home, project }
    }

    fn config(&self) -> ResolvedConfig {
        resolve(&ResolveRequest::new(self.project.path()).with_home_dir(self.home.path()))
            .expect("a resolved configuration")
            .config
    }
}

/// A provider with `n` scripted text replies, shared by parent and children.
fn scripted(n: usize, text: &str) -> Arc<FakeProvider> {
    let scripts = (0..n)
        .map(|_| {
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta { text: text.into() },
                usage_event(5, 2),
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])
        })
        .collect();
    Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ))
}

async fn wait_for_provider_requests(provider: &FakeProvider, expected: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while provider.requests().len() < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "provider recorded {} requests while waiting for {expected}",
            provider.requests().len()
        )
    });
}

#[derive(Debug)]
struct CrashThenReplyProvider {
    calls: AtomicUsize,
    entered: tokio::sync::Notify,
    requests: Mutex<Vec<ProviderRequest>>,
}

impl CrashThenReplyProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            requests: Mutex::new(Vec::new()),
        }
    }

    async fn wait_for_calls(&self, expected: usize) {
        while self.calls.load(Ordering::SeqCst) < expected {
            self.entered.notified().await;
        }
    }

    fn requests(&self) -> Vec<ProviderRequest> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .clone()
    }
}

#[async_trait]
impl Provider for CrashThenReplyProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        Vec::new()
    }

    fn capabilities(&self, _model: &agent_runtime_core::provider::ModelId) -> Option<Capabilities> {
        Some(Capabilities::basic_streaming())
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        _ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests
            .lock()
            .expect("provider requests poisoned")
            .push(request);
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_waiters();
        if call == 0 {
            Ok(Box::pin(futures_util::stream::pending()))
        } else {
            Ok(Box::pin(futures_util::stream::iter(vec![
                ProviderStreamEvent::TextDelta {
                    text: "resumed exact child".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])))
        }
    }
}

fn questionnaire_script(
    call: &str,
    question: &str,
    prompt: &str,
    sensitivity: &str,
) -> ScriptedStream {
    let arguments = serde_json::json!({
        "questions": [{
            "id": question,
            "header": "Choice",
            "prompt": prompt,
            "choices": [
                {"id": "one", "label": "One"},
                {"id": "two", "label": "Two"}
            ]
        }],
        "sensitivity": sensitivity,
    })
    .to_string();
    let mut events = tool_call_fragments(0, call, "ask_user", &arguments);
    events.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    ScriptedStream::new(events)
}

fn request(fixture: &Fixture, provider: Arc<dyn Provider>) -> RuntimeRequest {
    RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        provider: Some(provider),
        ..RuntimeRequest::new(fixture.config(), HostSurface::Terminal)
    }
}

async fn invoke_agent(
    tool: &AgentTool,
    arguments: serde_json::Value,
    ctx: &InvocationContext,
) -> Result<ToolOutcome, RuntimeError> {
    let preparation = PreparationContext {
        session: ctx.session.clone(),
        turn: ctx.turn.clone(),
        call_id: ctx.call_id.clone(),
        request: ctx.request.clone(),
        workspace: ctx.workspace.clone(),
        clock: ctx.clock.clone(),
        cancel: ctx.cancel.clone(),
        deadline: ctx.deadline,
    };
    let prepared = tool.prepare(arguments, &preparation).await?;
    tool.invoke(prepared, ctx).await
}
