//! One composition path, proved offline.
//!
//! The unit tests in `factory` and `catalog` check the mapping rules in
//! isolation. These start where a real Smith host starts — a `.smith/config.toml`
//! on disk — and follow it all the way to a runtime that runs a turn, because
//! the properties worth proving here are about the *seam*: that a resolved
//! configuration and injected host adapters produce one runtime, that the
//! failures happen before anything expensive, and that a credential that
//! entered the process never comes back out.
//!
//! Every test writes into temporary directories and none of them reaches a
//! network, a keychain, or the real `~/.smith`. The one test that resolves a
//! secret points its endpoint at a closed loopback port: the request fails to
//! connect, which is exactly the path where a leaky error message would show
//! up.

#![allow(deprecated)] // Exercises the documented protocol-v1 embedding adapter.

use std::sync::{Arc, Condvar, Mutex};

use agent_runtime::ability::Skill;
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
use agent_runtime::registry::Permission;
use agent_runtime::runtime::StartSession;
use agent_runtime_core::catalog::{
    CatalogSource, ModelLimits, ModelRecord, ProfileField, StaticSource,
};
use agent_runtime_core::content::UserInput;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::provider::{
    Capabilities, FinishReason, ModelId, Provider, ProviderStreamEvent,
};
use agent_runtime_core::store::Secret;
use agent_runtime_core::tool::{InvocationContext, PreparedToolCall, Tool, ToolOutcome, ToolSpec};
use agent_runtime_testkit::{MemoryWorkspace, RecordingObserver};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use smith_config::catalog::CatalogSnapshot;
use smith_config::credential::{CredentialResolver, Environment, Keychain, KeychainError};
use smith_config::model::AgentPosture;
use smith_config::resolve::{Layer, ResolveRequest, ResolvedConfig, resolve};
use smith_config::trust::TrustStatus;
use smith_host::ProjectWorkspace;
use smith_runtime::factory::{self, FactoryError, HostSurface, RuntimeRequest};
use smith_runtime::journal::{DefaultRedactor, EventJournal, JournalConfig, Redactor};
use smith_runtime::mcp::McpSupervisor;
use smith_runtime::memory::{SmithMemoryRecord, SmithMemorySource};
use smith_runtime::model_catalog::{EMBEDDED_MODELS_DEV_SEED, runtime_catalog_source};
use smith_runtime::project_instructions::ProjectInstructionsSnapshot;
use smith_runtime::skills::SmithSkillSources;

mod compaction;
mod credentials;
mod mcp;
mod model_catalog;
mod preflight;
mod prompts;
mod skills_and_memory;
mod tool_routing;

/// A resolvable configuration using the deterministic provider.
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

/// The same configuration with no `[models]` table at all.
const NO_LIMITS_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[approval]
mode = "allow-all"
"#;

/// A value shaped like a real key, so a leak is unmistakable in a diff.
const TOKEN: &str = "sk-live-4kQm2ZpX8vRt7nLb1cWs9aYe";

/// A project and a user root, both temporary.
///
/// The user root is injected rather than discovered: a test that reads the
/// developer's `~/.smith/config.toml` passes or fails depending on whose
/// machine it runs on.
struct Fixture {
    home: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Fixture {
    fn new(config: &str) -> Self {
        let home = tempfile::tempdir().expect("a user root");
        let project = tempfile::tempdir().expect("a project root");
        let dir = project.path().join(".smith");
        std::fs::create_dir_all(&dir).expect("a project `.smith`");
        std::fs::write(dir.join("config.toml"), config).expect("a project config");
        Self { home, project }
    }

    fn config(&self) -> ResolvedConfig {
        resolve(&ResolveRequest::new(self.project.path()).with_home_dir(self.home.path()))
            .expect("a resolved configuration")
            .config
    }

    #[cfg(unix)]
    fn new_private_user(config: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().expect("a user root");
        let project = tempfile::tempdir().expect("a project root");
        std::fs::create_dir_all(project.path().join(".smith")).expect("a project `.smith`");
        let user_dir = home.path().join(".smith");
        std::fs::create_dir_all(&user_dir).expect("a user `.smith`");
        let path = user_dir.join("config.toml");
        std::fs::write(&path, config).expect("a user config");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("an owner-only user config");
        Self { home, project }
    }
}

/// A request with the one host adapter every composition requires.
fn request(fixture: &Fixture, surface: HostSurface) -> RuntimeRequest {
    RuntimeRequest {
        workspace: Some(Arc::new(MemoryWorkspace::new("/repo"))),
        ..RuntimeRequest::new(fixture.config(), surface)
    }
}

/// An environment that answers from a fixed value, so no test reads the real
/// process environment or opens the developer's keychain.
#[derive(Debug)]
struct FixedEnvironment(Option<String>);

impl Environment for FixedEnvironment {
    fn value(&self, _name: &str) -> Option<Secret> {
        self.0.as_ref().map(Secret::new)
    }
}

fn resolver(value: Option<&str>) -> CredentialResolver {
    CredentialResolver::new("/nonexistent-user-state")
        .with_environment(Arc::new(FixedEnvironment(value.map(str::to_owned))))
}

#[derive(Debug)]
struct PanicsIfCredentialResolved;

impl Keychain for PanicsIfCredentialResolved {
    fn secret(&self, _service: &str, _account: &str) -> Result<Secret, KeychainError> {
        panic!("an inline API key must not consult the platform credential service")
    }
}

impl Environment for PanicsIfCredentialResolved {
    fn value(&self, _name: &str) -> Option<Secret> {
        panic!("an inline API key must not consult the environment")
    }
}

#[tokio::test]
async fn a_resolved_fake_configuration_builds_a_runtime_and_runs_a_turn() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let smith = factory::build_request(request(&fixture, HostSurface::Terminal))
        .await
        .expect("a runtime");

    let policy = smith.policy();
    assert_eq!(policy.provider_name, "local");
    assert_eq!(policy.provider_kind, "fake");
    assert_eq!(policy.model, ModelId::new("example-model"));
    assert_eq!(
        policy.model_profile.limits,
        ModelLimits::new(128_000, 124_000, 4_096)
    );
    assert_eq!(
        policy.system_prompt,
        smith_runtime::prompt::legacy_system_prompt(&smith_runtime::prompt::DynamicPromptContext {
            agent_profile: Some(smith_runtime::prompt::AgentProfilePrompt {
                name: "dev".into(),
                posture: AgentPosture::Build,
                instructions: None,
                revision: policy.agent_profile_revision.clone(),
            }),
            // A terminal root run on a build posture registers all three
            // gated capabilities, so all three sections are contributed.
            todo_planning: true,
            questionnaire: true,
            delegation: true,
            ..smith_runtime::prompt::DynamicPromptContext::default()
        })
    );
    assert_eq!(
        policy.tools,
        [
            "read",
            "list",
            "search",
            "edit",
            "shell",
            "task_output",
            "task_stop",
            "ask_user",
            "write_todos",
            "get_goal",
            "create_goal",
            "update_goal",
            "agent"
        ],
        "a root surface registers the standard questionnaire, todo, and delegation tools"
    );
    assert_eq!(
        smith.abilities().names(),
        [
            "read",
            "list",
            "search",
            "edit",
            "shell",
            "task_output",
            "task_stop",
            "ask_user",
            "write_todos",
            "get_goal",
            "create_goal",
            "update_goal",
            "agent"
        ],
        "the one factory seals every executable tool as one descriptor-first ability"
    );
    // The built-in defaults, mapped: two retries is three attempts, an
    // unlimited tool loop carries no ceiling at all, and the reserve falls
    // back to the model's own declared ceiling.
    assert_eq!(policy.max_attempts, 3);
    assert_eq!(policy.max_tool_steps, None);
    assert_eq!(policy.context_policy.output_reserve, 4_096);
    assert_eq!(policy.context_policy.reasoning_reserve, 0);

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    assert!(
        session
            .history()
            .iter()
            .any(|message| message.joined_text().contains(factory::DEVELOPMENT_REPLY)),
        "the turn produced no assistant answer"
    );
    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn the_same_factory_gives_a_terminal_and_a_headless_run_the_same_policy() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let answer = "the same canonical answer";

    let mut built = Vec::new();
    for surface in [HostSurface::Terminal, HostSurface::Headless] {
        let request = RuntimeRequest {
            provider: Some(Arc::new(FakeProvider::text_reply(answer)) as Arc<dyn Provider>),
            ..request(&fixture, surface)
        };
        built.push(factory::build_request(request).await.expect("a runtime"));
    }
    let (tui, headless) = (&built[0], &built[1]);

    // The presentation is the only declared difference.
    assert_eq!(tui.policy(), headless.policy());
    assert_eq!(
        tui.policy().model_profile.fingerprint(),
        headless.policy().model_profile.fingerprint()
    );
    assert_eq!(tui.surface(), HostSurface::Terminal);
    assert_eq!(headless.surface(), HostSurface::Headless);

    let mut transcripts = Vec::new();
    for smith in [tui, headless] {
        let session = smith
            .runtime()
            .start_session(StartSession::new())
            .await
            .expect("a session");
        session
            .run(UserInput::text("hello"))
            .await
            .expect("the turn runs");
        transcripts.push(
            session
                .history()
                .iter()
                .map(|message| message.joined_text())
                .collect::<Vec<_>>(),
        );
        session.shutdown().await.expect("a clean shutdown");
    }
    assert_eq!(transcripts[0], transcripts[1]);
    assert!(transcripts[0].iter().any(|text| text.contains(answer)));
}
