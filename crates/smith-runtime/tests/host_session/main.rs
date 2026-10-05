//! The lifecycle shared by the TUI and `smith -p`.
//!
//! These tests deliberately start above the runtime factory: configuration is
//! discovered from disk, the project workspace is real, and snapshots plus
//! canonical events are written below an injected user root. No test reaches
//! the network or the developer's home directory.

use std::sync::Arc;

use agent_runtime::provider::fake::{
    FakeProvider, ScriptedStream, tool_call_fragments, usage_event,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::approval::{AllowAll, DenyAll};
use agent_runtime_core::artifact::{
    ArtifactId, ArtifactRead, ArtifactRef, ArtifactStore, MAX_ARTIFACT_READ_BYTES,
};
use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::checkpoint::{CheckpointStore, TurnCheckpoint, TurnState};
use agent_runtime_core::clock::{Deadline, Timestamp};
use agent_runtime_core::content::UserInput;
use agent_runtime_core::delegation::{
    ChildLimits, ChildModelSelection, ChildSpec, ToolViewScope, WorkspacePolicy,
};
use agent_runtime_core::event::{EventEnvelope, RuntimeEvent, TurnFinish, canonical_payloads};
use agent_runtime_core::ids::{
    ChildId, ChoiceId, EventId, InteractionRequestId, QuestionId, SessionId, ToolCallId, TurnId,
};
use agent_runtime_core::interaction::{InteractionSensitivity, QuestionAnswer};
use agent_runtime_core::provider::{
    Capabilities, FinishReason, Provider, ProviderError, ProviderErrorKind, ProviderStreamEvent,
};
use agent_runtime_core::store::{
    SessionIdentityState, SessionSnapshot, SessionStateSensitivity, SessionStore,
    VersionedSessionState,
};
use agent_runtime_core::usage::{CounterKind, UsageLedger, UsageSource};
use agent_runtime_testkit::RecordingObserver;
use futures_util::StreamExt;
use smith_config::resolve::{Overrides, ResolveRequest, ResolvedConfig, resolve};
use smith_host::{InteractionNotice, InteractiveInteraction, ProjectWorkspace};
use smith_runtime::artifact::SmithArtifactStore;
use smith_runtime::checkpoint::{
    CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError, SmithCheckpointStore,
};
use smith_runtime::factory::{HostSurface, MidTurnDurability, RuntimeRequest};
use smith_runtime::host::{HostSessionError, HostSessionRequest, list, start};
use smith_runtime::journal::{DefaultRedactor, JournalLine, JournalRecord, read_journal};
use smith_runtime::resume_capsule::{
    ChildLifecycleState, ChildResumeProjection, ChildTerminalOutcome, ExactResumeState,
    RESUME_CAPSULE_STATE_NAMESPACE, ResumeCacheWarmth, ResumeCapsuleSlot,
};
use smith_runtime::session::FileSessionStore;
use smith_runtime::{ChildDurability, ChildState, SpawnOutcome};

mod artifacts;
mod background_tasks;
mod checkpoints;
mod children;
mod interaction;
mod journal;
mod policy;
mod project_instructions;
mod reasoning;
mod resume;
mod semantic_summaries;
mod todos;

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

fn edit_provider() -> Arc<dyn Provider> {
    let mut edit = tool_call_fragments(
        0,
        "edit-1",
        "edit",
        r#"{"path":"tracked.txt","old_string":"before\n","new_string":"after\n"}"#,
    );
    edit.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(edit),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "edited".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ))
}

struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

#[derive(Debug)]
struct TestCheckpointKeys;

impl CheckpointKeyProvider for TestCheckpointKeys {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        Ok(CheckpointKey::new([0x51; 32]))
    }
}

#[derive(Debug)]
struct UnavailableCheckpointKeys;

impl CheckpointKeyProvider for UnavailableCheckpointKeys {
    fn load_or_create(&self) -> Result<CheckpointKey, CheckpointProtectionError> {
        Err(CheckpointProtectionError::unavailable())
    }
}

fn test_checkpoint_keys() -> Arc<dyn CheckpointKeyProvider> {
    Arc::new(TestCheckpointKeys)
}

impl Fixture {
    fn new() -> Self {
        Self::with_config(CONFIG)
    }

    fn with_config(config: &str) -> Self {
        let home = tempfile::tempdir().expect("a user root");
        let project = tempfile::tempdir().expect("a project root");
        let config_dir = project.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("a project config directory");
        std::fs::write(config_dir.join("config.toml"), config).expect("a project config");
        Self { home, project }
    }

    fn config(&self) -> ResolvedConfig {
        resolve(&ResolveRequest::new(self.project.path()).with_home_dir(self.home.path()))
            .expect("resolved configuration")
            .config
    }

    fn request(&self, surface: HostSurface) -> HostSessionRequest {
        self.request_with_config(self.config(), surface)
    }

    fn request_with_config(
        &self,
        config: ResolvedConfig,
        surface: HostSurface,
    ) -> HostSessionRequest {
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                ProjectWorkspace::new(self.project.path()).expect("a project workspace"),
            )),
            approval: Some(Arc::new(AllowAll)),
            ..RuntimeRequest::new(config, surface)
        };
        HostSessionRequest::new(runtime, self.project.path())
            .checkpoint_keys(test_checkpoint_keys())
    }
}

#[tokio::test]
async fn terminal_and_headless_hosts_emit_the_same_canonical_turn() {
    let fixture = Fixture::new();
    let mut runs = Vec::new();

    for surface in [HostSurface::Terminal, HostSurface::Headless] {
        let observer = RecordingObserver::shared();
        let mut request = fixture.request(surface);
        request.runtime.observers.push(observer.clone());
        let host = start(request).await.expect("a hosted session");
        let policy = host.runtime().policy().clone();
        host.session()
            .run(UserInput::text("same input"))
            .await
            .expect("the turn runs");
        host.shutdown().await.expect("a clean shutdown");
        runs.push((policy, observer.events()));
    }

    assert_eq!(runs[0].0, runs[1].0, "surface changed runtime policy");
    assert_eq!(
        canonical_payloads(&runs[0].1),
        canonical_payloads(&runs[1].1),
        "surface changed canonical behavior"
    );
}
