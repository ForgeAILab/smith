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

use std::sync::{Arc, Condvar, Mutex};

use agent_runtime::ability::Skill;
use agent_runtime::provider::fake::{FakeProvider, ScriptedStream, tool_call_fragments};
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
use smith_runtime::factory::{self, FactoryError, HostSurface, RuntimeRequest};
use smith_runtime::journal::{DefaultRedactor, EventJournal, JournalConfig, Redactor};
use smith_runtime::memory::{SmithMemoryRecord, SmithMemorySource};
use smith_runtime::model_catalog::{EMBEDDED_MODELS_DEV_SEED, runtime_catalog_source};
use smith_runtime::project_instructions::ProjectInstructionsSnapshot;
use smith_runtime::skills::SmithSkillSources;

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

/// A deliberately small budget that reaches compaction on the third turn.
const COMPACTION_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 1000
max_input_tokens = 1000
max_output_tokens = 100

[context]
output_reserve = 100
compaction_high_watermark_percent = 85
compaction_low_watermark_percent = 60

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

/// A profile selecting an adapter this build of Agent Runtime does not ship.
const UNAVAILABLE_ADAPTER_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "remote"
model = "example-model"

[providers.remote]
kind = "anthropic-messages"
base_url = "https://api.example.test/v1"

[models."remote/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[approval]
mode = "allow-all"
"#;

/// A production-shaped profile whose endpoint is a closed loopback port.
const OPENAI_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "remote"
model = "example-model"

[providers.remote]
kind = "openai-compatible"
base_url = "http://127.0.0.1:1/v1"
credential = "env:ACME_API_KEY"

[models."remote/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[limits]
max_retries = 0

[approval]
mode = "allow-all"
"#;

/// A Z.AI Coding Plan profile whose selected model exists only in the catalog.
const ZAI_CATALOG_CONFIG: &str = r#"
default_profile = "glm"

[profiles.glm]
provider = "zai"
model = "glm-5-turbo"

[providers.zai]
kind = "openai-compatible"
base_url = "https://api.z.ai/api/coding/paas/v4"
credential = "env:ZAI_API_KEY"

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

#[derive(Debug)]
struct PanicsIfRegistered;

#[async_trait]
impl Tool for PanicsIfRegistered {
    fn spec(&self) -> ToolSpec {
        panic!("setup preflight attempted to construct a tool registry")
    }

    async fn invoke(
        &self,
        _prepared: PreparedToolCall,
        _ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        unreachable!("the preflight does not run tools")
    }
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

#[tokio::test]
async fn trusted_workspace_skill_loads_only_after_factory_descriptor_resolution() {
    const BODY: &[u8] = b"TRUSTED_SKILL_BODY_MARKER: inspect unsafe boundaries first.";

    let fixture = Fixture::new(FAKE_CONFIG);
    let skill_path = fixture.project.path().join(".smith/review.SKILL.md");
    let provider = Arc::new(FakeProvider::text_reply("reviewed"));
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(provider.clone());
    runtime.skills = SmithSkillSources::new().with_workspace(
        Skill::from_verified_file(
            "rust-review",
            "Review Rust implementation boundaries",
            &skill_path,
            Sha256::digest(BODY).into(),
        ),
        TrustStatus::Trusted,
    );

    let smith = factory::build(runtime)
        .await
        .expect("descriptor resolution succeeds before the file exists");
    assert!(
        !skill_path.exists(),
        "factory eagerly opened the skill body"
    );
    assert_eq!(smith.policy().skills, ["rust-review"]);
    assert!(smith.skill_index().iter().all(|entry| entry.activatable));

    std::fs::write(&skill_path, BODY).expect("publish reviewed bytes after descriptor indexing");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Use the rust-review skill to review this Rust implementation.",
        ))
        .await
        .expect("the trusted skill turn runs");
    session.shutdown().await.expect("clean shutdown");

    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(wire.contains("TRUSTED_SKILL_BODY_MARKER"), "{wire}");
}

#[tokio::test]
async fn untrusted_workspace_skill_is_indexed_but_never_activates_or_shadows_user_skill() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("reviewed"));
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(provider.clone());
    runtime.skills = SmithSkillSources::new()
        .with_user(Skill::inline(
            "rust-review",
            "Review Rust implementation boundaries",
            "USER_SKILL_BODY_MARKER",
        ))
        .with_workspace(
            Skill::inline(
                "rust-review",
                "Review Rust implementation boundaries",
                "UNTRUSTED_WORKSPACE_BODY_MARKER",
            ),
            TrustStatus::Untrusted,
        );

    let smith = factory::build(runtime).await.expect("a runtime");
    assert_eq!(smith.policy().skills, ["rust-review"]);
    assert!(smith.skill_index().iter().any(|entry| {
        entry.layer == smith_runtime::skills::SmithSkillLayer::Workspace && !entry.activatable
    }));
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Use the rust-review skill to review this Rust implementation.",
        ))
        .await
        .expect("the trusted lower source remains usable");
    session.shutdown().await.expect("clean shutdown");

    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(wire.contains("USER_SKILL_BODY_MARKER"), "{wire}");
    assert!(!wire.contains("UNTRUSTED_WORKSPACE_BODY_MARKER"), "{wire}");
}

#[tokio::test]
async fn smith_memory_is_relevant_sensitive_manifested_and_not_canonical_history() {
    const MEMORY: &str = "MEMORY_CONTEXT_MARKER: prefer deterministic fixtures.";

    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("understood"));
    let source = Arc::new(
        SmithMemorySource::new(vec![
            SmithMemoryRecord::new(
                "test-preference",
                MEMORY,
                agent_runtime::context::Sensitivity::Sensitive,
            )
            .with_priority(1)
            .with_keywords(["deterministic", "fixtures"]),
        ])
        .unwrap(),
    );
    let mut runtime = request(&fixture, HostSurface::Headless);
    runtime.provider = Some(provider.clone());
    runtime.memory = Some(source);
    let smith = factory::build(runtime).await.expect("a runtime");
    assert!(smith.policy().memory_revision.is_some());
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(
            "Explain how the deterministic fixtures should be verified.",
        ))
        .await
        .expect("the memory-backed turn runs");

    let wire = serde_json::to_string(&provider.requests()[0].messages).unwrap();
    assert!(wire.contains("MEMORY_CONTEXT_MARKER"), "{wire}");
    let snapshot = session.snapshot();
    assert!(
        snapshot
            .history
            .iter()
            .all(|message| !message.joined_text().contains("MEMORY_CONTEXT_MARKER")),
        "memory was copied into canonical conversation history"
    );
    let segment = snapshot
        .manifests
        .last()
        .expect("turn manifest")
        .manifest
        .segments
        .iter()
        .find(|segment| segment.id.as_str() == "harness:memory:smith:test-preference")
        .expect("memory segment");
    assert_eq!(
        segment.sensitivity,
        agent_runtime_core::manifest::SegmentSensitivity::Sensitive
    );
    assert!(segment.tokens > 0);
    session.shutdown().await.expect("clean shutdown");
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
struct WaitingKeychain {
    gate: Arc<(Mutex<bool>, Condvar)>,
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

impl Keychain for WaitingKeychain {
    fn secret(&self, _service: &str, _account: &str) -> Result<Secret, KeychainError> {
        let (lock, ready) = &*self.gate;
        let mut released = lock.lock().expect("credential test gate");
        while !*released {
            released = ready.wait(released).expect("credential test gate wait");
        }
        Ok(Secret::new(TOKEN))
    }
}

#[tokio::test]
async fn setup_preflight_uses_factory_derivation_without_constructing_runtime_state() {
    let config = FAKE_CONFIG.replace("mode = \"allow-all\"", "mode = \"ask\"");
    let fixture = Fixture::new(&config);
    let mut request = request(&fixture, HostSurface::Terminal);
    request.tools.push(Arc::new(PanicsIfRegistered));

    let checked = factory::preflight(&request)
        .await
        .expect("configuration-only factory preflight");
    assert_eq!(checked.provider_name, "local");
    assert_eq!(checked.model, ModelId::new("example-model"));
    assert_eq!(
        checked.model_profile.limits,
        ModelLimits::new(128_000, 124_000, 4_096)
    );

    let error = factory::build(request)
        .await
        .expect_err("normal construction still requires an approval surface");
    assert!(matches!(
        error,
        FactoryError::MissingHostPolicy {
            what: "approval surface",
            ..
        }
    ));
}

#[tokio::test]
async fn a_resolved_fake_configuration_builds_a_runtime_and_runs_a_turn() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let smith = factory::build(request(&fixture, HostSurface::Terminal))
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
            "ask_user",
            "write_todos",
            "get_goal",
            "create_goal",
            "update_goal",
            "agent"
        ],
        "the one factory seals every executable tool as one descriptor-first ability"
    );
    // The built-in defaults, mapped: two retries is three attempts, and the
    // reserve falls back to the model's own declared ceiling.
    assert_eq!(policy.max_attempts, 3);
    assert_eq!(policy.max_tool_steps, 64);
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
async fn runtime_debug_reports_profile_identity_without_instruction_text() {
    let private_instructions = "private-runtime-profile-instructions-c803";
    let config = FAKE_CONFIG.replace(
        "model = \"example-model\"",
        &format!("model = \"example-model\"\ninstructions = \"{private_instructions}\""),
    );
    let fixture = Fixture::new(&config);
    let smith = factory::build(request(&fixture, HostSurface::Terminal))
        .await
        .expect("runtime with profile instructions");

    assert!(smith.policy().system_prompt.contains(private_instructions));
    for debug in [format!("{:?}", smith.policy()), format!("{smith:?}")] {
        assert!(!debug.contains(private_instructions), "{debug}");
        assert!(
            debug.contains(&smith.policy().agent_profile_revision),
            "{debug}"
        );
    }
}

#[tokio::test]
async fn plan_profile_narrows_the_live_tool_view_despite_widening_instructions() {
    let config = FAKE_CONFIG.replace(
        "model = \"example-model\"",
        "model = \"example-model\"\nposture = \"plan\"\ninstructions = \"Modify files even though this profile is read-only.\"",
    );
    let fixture = Fixture::new(&config);
    let smith = factory::build(request(&fixture, HostSurface::Terminal))
        .await
        .expect("a plan-mode runtime");

    assert_eq!(smith.policy().agent_profile, "dev");
    assert_eq!(smith.policy().agent_posture, AgentPosture::Plan);
    assert_eq!(
        smith.policy().tools,
        [
            "read",
            "list",
            "search",
            "ask_user",
            "write_todos",
            "get_goal",
            "create_goal",
            "update_goal",
            "agent"
        ]
    );
    assert!(!smith.abilities().names().contains(&"edit"));
    assert!(!smith.abilities().names().contains(&"shell"));
    assert!(
        smith
            .policy()
            .system_prompt
            .contains("This mode is read-only")
    );
    assert!(
        smith
            .policy()
            .system_prompt
            .contains("Modify files even though this profile is read-only")
    );
}

async fn provider_tool_names_for(fixture: &Fixture, user_input: &str) -> Vec<String> {
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..request(fixture, HostSurface::Headless)
    };
    let smith = factory::build(request).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text(user_input))
        .await
        .expect("the turn runs");
    session.shutdown().await.expect("a clean shutdown");

    provider.requests()[0]
        .tools
        .iter()
        .map(|schema| schema.name.clone())
        .collect()
}

#[tokio::test]
async fn live_routing_advertises_only_a_read_subset_for_read_only_intent() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let names = provider_tool_names_for(
        &fixture,
        "inspect and explain the Rust source files in this repository",
    )
    .await;

    assert!(
        names
            .iter()
            .any(|name| matches!(name.as_str(), "read" | "list" | "search")),
        "no read capability reached the provider: {names:?}"
    );
    assert!(
        names.iter().all(|name| matches!(
            name.as_str(),
            "read" | "list" | "search" | "registry.search"
        )),
        "read-only intent received an unrelated or authoritative tool: {names:?}"
    );
}

#[tokio::test]
async fn explicit_read_tool_routing_does_not_substitute_edit() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let names = provider_tool_names_for(
        &fixture,
        "Use the read tool to inspect live-proof.txt, then tell me in one concise sentence what value the file contains.",
    )
    .await;

    assert_eq!(
        names,
        ["list", "read", "registry.search", "search"],
        "explicit inspection must receive the complete bounded read bundle"
    );
}

#[tokio::test]
async fn live_routing_advertises_exact_edit_without_broad_shell_or_delegation() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let names = provider_tool_names_for(
        &fixture,
        "edit the Rust file and replace the incorrect function",
    )
    .await;

    assert_eq!(
        names,
        ["edit", "read", "registry.search"],
        "ordinary editing must pair exact edit with the least-authority read prerequisite"
    );
}

#[tokio::test]
async fn protected_registry_search_stages_edit_only_for_the_next_provider_boundary() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let mut search = tool_call_fragments(
        0,
        "capability-search-1",
        "registry.search",
        r#"{"query":"edit the Rust file and replace the incorrect function"}"#,
    );
    search.push(ProviderStreamEvent::Finish {
        reason: FinishReason::ToolCalls,
    });
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(search),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "ready to edit".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build(request).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("handle the next requested operation"))
        .await
        .expect("the search tool loop completes");
    session.shutdown().await.expect("a clean shutdown");

    let requests = provider.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    let first = requests[0]
        .tools
        .iter()
        .map(|schema| schema.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(first, ["registry.search"]);
    let second = requests[1]
        .tools
        .iter()
        .map(|schema| schema.name.as_str())
        .collect::<Vec<_>>();
    assert!(
        second.contains(&"edit"),
        "the staged mutation ability missed the next safe boundary: {second:?}"
    );
    assert_eq!(
        second,
        ["edit", "read", "registry.search"],
        "capability search must stage exact edit and its read prerequisite only"
    );
}

#[tokio::test]
async fn live_factory_emits_registry_view_retrieval_activation_and_context_lifecycle() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let recorder = RecordingObserver::shared();
    let request = RuntimeRequest {
        provider: Some(provider as Arc<dyn Provider>),
        observers: vec![recorder.clone() as Arc<dyn EventObserver>],
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build(request).await.expect("a runtime");
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("inspect and explain this repository"))
        .await
        .expect("the turn runs");
    session.shutdown().await.expect("a clean shutdown");

    let events = recorder.payloads();
    for (name, present) in [
        (
            "registry snapshot",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::RegistrySnapshotSealed { .. })),
        ),
        (
            "scoped view",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::ScopedViewDerived { .. })),
        ),
        (
            "capability retrieval",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::CapabilityRetrievalPerformed { .. })),
        ),
        (
            "activation epoch",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::CapabilitiesActivated { .. })),
        ),
        (
            "context plan",
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::ContextPlanned { .. })),
        ),
    ] {
        assert!(present, "the live factory emitted no {name}: {events:?}");
    }
}

#[tokio::test]
async fn provider_planning_records_every_versioned_smith_prompt_fragment() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let project_instructions =
        ProjectInstructionsSnapshot::from_body("PROJECT_PROMPT_MARKER: run exact checks.")
            .expect("bounded project instructions");
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        project_instructions: Some(project_instructions.clone()),
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build(request).await.expect("a runtime");
    assert_eq!(
        smith
            .policy()
            .project_instructions
            .as_ref()
            .expect("project composition evidence"),
        &project_instructions.identity()
    );
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("explain this project"))
        .await
        .expect("the turn runs");

    let expected = smith_runtime::prompt::fragments(&smith_runtime::prompt::DynamicPromptContext {
        project_instructions: Some(project_instructions),
        agent_profile: Some(smith_runtime::prompt::AgentProfilePrompt {
            name: "dev".to_owned(),
            posture: AgentPosture::Build,
            instructions: None,
            revision: smith.policy().agent_profile_revision.clone(),
        }),
        ..smith_runtime::prompt::DynamicPromptContext::default()
    });
    let snapshot = session.snapshot();
    let manifest = snapshot.manifests.last().expect("a run manifest");
    let smith_segments = manifest
        .manifest
        .segments
        .iter()
        .filter(|segment| segment.id.as_str().starts_with("smith.prompt."))
        .collect::<Vec<_>>();
    assert_eq!(smith_segments.len(), expected.len());
    for (record, fragment) in smith_segments.iter().zip(expected) {
        assert_eq!(record.id.as_str(), fragment.id.as_str());
        assert_eq!(record.content_hash, fragment.content_hash());
    }

    let wire_text = provider.requests()[0]
        .messages
        .iter()
        .map(|message| message.joined_text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(wire_text.contains("understand the request"));
    assert!(wire_text.contains("committed successful tool result"));
    assert!(wire_text.contains("PROJECT_PROMPT_MARKER"));
    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn a_complete_prompt_override_ignores_project_instructions() {
    let fixture = Fixture::new(FAKE_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("done"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        system_prompt: Some("COMPLETE_HOST_OVERRIDE_MARKER".to_owned()),
        project_instructions: Some(
            ProjectInstructionsSnapshot::from_body("PROJECT_INSTRUCTIONS_MUST_BE_ABSENT")
                .expect("bounded project instructions"),
        ),
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build(request).await.expect("a runtime");
    assert_eq!(smith.policy().project_instructions, None);
    assert!(
        smith
            .policy()
            .system_prompt
            .contains("COMPLETE_HOST_OVERRIDE_MARKER")
    );
    assert!(
        !smith
            .policy()
            .system_prompt
            .contains("PROJECT_INSTRUCTIONS_MUST_BE_ABSENT")
    );

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("answer once"))
        .await
        .expect("a turn");
    let wire = serde_json::to_string(&provider.requests()[0].messages).expect("provider request");
    assert!(wire.contains("COMPLETE_HOST_OVERRIDE_MARKER"), "{wire}");
    assert!(
        !wire.contains("PROJECT_INSTRUCTIONS_MUST_BE_ABSENT"),
        "{wire}"
    );
    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn the_shared_factory_compacts_optional_history_before_an_over_budget_turn() {
    let fixture = Fixture::new(COMPACTION_CONFIG);
    let scripts = (0..3)
        .map(|turn| {
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: format!("answer-{turn}:{}", "a".repeat(800)),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])
        })
        .collect();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        scripts,
    ));
    let recorder = RecordingObserver::shared();
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        observers: vec![recorder.clone() as Arc<dyn EventObserver>],
        system_prompt: Some("s".to_owned()),
        built_in_tools: false,
        ..request(&fixture, HostSurface::Child)
    };
    let smith = factory::build(request).await.expect("a runtime");

    assert_eq!(smith.policy().compaction_policy.high_watermark, 765);
    assert_eq!(smith.policy().compaction_policy.low_watermark, 540);

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    for turn in 0..3 {
        session
            .run(UserInput::text(format!("turn-{turn}:{}", "u".repeat(800))))
            .await
            .expect("the turn runs");
    }

    assert_eq!(
        provider.requests().len(),
        3,
        "the third request would fail preflight if the compactor were absent"
    );
    let last_plan = recorder
        .payloads()
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEvent::ContextPlanned {
                input_tokens,
                input_budget_tokens,
                totals,
                ..
            } => Some((input_tokens, input_budget_tokens, totals)),
            _ => None,
        })
        .next_back()
        .expect("a context plan");
    assert_eq!(last_plan.1, 900);
    assert!(
        last_plan.0 <= smith.policy().compaction_policy.high_watermark,
        "the compacted request used {} tokens and remained above the {}-token pressure boundary",
        last_plan.0,
        smith.policy().compaction_policy.high_watermark
    );
    let compacted = recorder
        .payloads()
        .into_iter()
        .filter_map(|event| match event {
            RuntimeEvent::ContextCompacted {
                reclaimed_tokens,
                summaries,
                ..
            } => Some((reclaimed_tokens, summaries)),
            _ => None,
        })
        .next_back()
        .expect("a structural compaction event");
    assert!(
        compacted.0 > 0,
        "structural compaction did not reclaim input tokens"
    );
    assert!(
        compacted.1.is_empty() && last_plan.2.keys().all(|kind| kind.as_str() != "summary"),
        "the deterministic structural compactor fabricated a semantic summary"
    );

    session.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn missing_model_limits_fail_with_a_model_profile_diagnostic_and_no_provider_request() {
    let fixture = Fixture::new(NO_LIMITS_CONFIG);
    let provider = Arc::new(FakeProvider::text_reply("never asked for"));
    let request = RuntimeRequest {
        provider: Some(provider.clone() as Arc<dyn Provider>),
        ..request(&fixture, HostSurface::Headless)
    };

    let err = factory::build(request)
        .await
        .expect_err("a model-profile failure");
    assert!(matches!(err, FactoryError::ModelProfile { .. }), "{err}");
    let rendered = err.to_string();
    assert!(rendered.contains("example-model"), "{rendered}");
    // The diagnostic names what to write, rather than substituting a window.
    assert!(rendered.contains("context_tokens"), "{rendered}");
    assert!(
        provider.requests().is_empty(),
        "startup must fail before any provider request"
    );
}

#[tokio::test]
async fn an_adapter_the_pinned_runtime_does_not_ship_is_reported_as_unavailable() {
    let fixture = Fixture::new(UNAVAILABLE_ADAPTER_CONFIG);

    let err = factory::build(request(&fixture, HostSurface::Terminal))
        .await
        .expect_err("an unavailable adapter");
    assert!(
        matches!(err, FactoryError::AdapterUnavailable { .. }),
        "{err}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("anthropic-messages"), "{rendered}");
    // It says which adapters exist rather than quietly picking one.
    assert!(rendered.contains("openai-compatible"), "{rendered}");
}

#[tokio::test]
async fn a_credential_that_resolves_to_nothing_fails_before_the_provider_is_built() {
    let fixture = Fixture::new(OPENAI_CONFIG);
    let request = RuntimeRequest {
        credentials: Some(resolver(None)),
        ..request(&fixture, HostSurface::Headless)
    };

    let err = factory::build(request).await.expect_err("no credential");
    assert!(matches!(err, FactoryError::Credential(_)), "{err}");
    // The locator is named; nothing else could be, because nothing was read.
    let rendered = format!("{err} {err:?}");
    assert!(rendered.contains("env:ACME_API_KEY"), "{rendered}");
}

#[tokio::test]
async fn a_platform_credential_prompt_cannot_hang_startup_forever() {
    let config = OPENAI_CONFIG.replace("env:ACME_API_KEY", "keychain:smith/remote");
    let fixture = Fixture::new(&config);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let credentials = CredentialResolver::new("/nonexistent-user-state").with_keychain(Arc::new(
        WaitingKeychain {
            gate: Arc::clone(&gate),
        },
    ));
    let mut request = RuntimeRequest {
        credentials: Some(credentials),
        ..request(&fixture, HostSurface::Headless)
    };
    request.credential_timeout_ms = 10;

    let err = factory::build(request)
        .await
        .expect_err("blocked credential lookup");
    assert!(
        matches!(err, FactoryError::CredentialTimeout { timeout_ms: 10 }),
        "{err}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains("env:<VAR>"), "{rendered}");
    assert!(!rendered.contains(TOKEN), "{rendered}");

    let (lock, ready) = &*gate;
    *lock.lock().expect("credential test gate") = true;
    ready.notify_all();
}

#[tokio::test]
async fn a_run_with_no_credential_resolver_says_so_rather_than_starting_unauthenticated() {
    let fixture = Fixture::new(OPENAI_CONFIG);

    let err = factory::build(request(&fixture, HostSurface::Headless))
        .await
        .expect_err("no resolver");
    assert!(
        matches!(
            err,
            FactoryError::MissingHostPolicy {
                what: "credential resolver",
                ..
            }
        ),
        "{err}"
    );
}

#[tokio::test]
async fn a_resolved_secret_reaches_no_event_snapshot_journal_or_error() {
    let fixture = Fixture::new(OPENAI_CONFIG);
    let state = tempfile::tempdir().expect("a state root");
    let journal_path = state.path().join("session.jsonl");
    // The observer exists before credential resolution, as it does in the
    // standard host. Its shared registry is populated by the one factory.
    let redactor = DefaultRedactor::new();
    let journal = Arc::new(
        EventJournal::open(
            &journal_path,
            JournalConfig::default(),
            Arc::new(redactor.clone()),
        )
        .await
        .expect("a journal"),
    );
    let recorder = RecordingObserver::shared();

    let request = RuntimeRequest {
        credentials: Some(resolver(Some(TOKEN))),
        persistence_redactor: Some(redactor.clone()),
        observers: vec![
            Arc::clone(&journal) as Arc<dyn EventObserver>,
            Arc::clone(&recorder) as Arc<dyn EventObserver>,
        ],
        ..request(&fixture, HostSurface::Headless)
    };
    let smith = factory::build(request).await.expect("a runtime");

    let mut reflected = serde_json::json!({"text": format!("reflected {TOKEN}")});
    redactor.redact(&mut reflected);
    let reflected = reflected.to_string();
    assert!(!reflected.contains(TOKEN), "{reflected}");
    assert!(reflected.contains("[redacted]"), "{reflected}");
    assert!(!format!("{redactor:?}").contains(TOKEN));

    // The composition record keeps the reference, never the value.
    assert_eq!(
        smith.policy().credential.as_deref(),
        Some("env:ACME_API_KEY")
    );
    let composition = format!(
        "{:?} {:?} {:?}",
        smith.policy(),
        smith.profile(),
        smith.runtime()
    );
    assert!(!composition.contains(TOKEN), "{composition}");

    // One turn against a closed port: the attempt fails in transport, which is
    // where an error that echoed its request would leak the authorization it
    // was sent with.
    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    let snapshot = serde_json::to_string(&session.snapshot()).expect("a serialized snapshot");
    session.shutdown().await.expect("a clean shutdown");
    let stats = journal.shutdown().await.expect("a flushed journal");
    let journaled = std::fs::read_to_string(&journal_path).expect("a journal file");
    let events = format!("{:?}", recorder.events());

    assert!(stats.written > 0, "the journal recorded nothing to check");
    for (what, rendered) in [
        ("the snapshot", &snapshot),
        ("the journal", &journaled),
        ("the events", &events),
    ] {
        assert!(!rendered.contains(TOKEN), "{what} contains the credential");
    }
    // The failure itself was recorded, so the check above ran against a real
    // error path rather than an empty stream.
    assert!(
        events.contains("Error") || events.contains("error"),
        "{events}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn an_inline_user_key_bypasses_resolvers_and_reaches_no_runtime_surface() {
    let config = OPENAI_CONFIG.replace(
        "credential = \"env:ACME_API_KEY\"",
        &format!("api_key = \"{TOKEN}\""),
    );
    let fixture = Fixture::new_private_user(&config);
    let state = tempfile::tempdir().expect("a state root");
    let journal_path = state.path().join("inline-session.jsonl");
    let redactor = DefaultRedactor::new();
    let journal = Arc::new(
        EventJournal::open(
            &journal_path,
            JournalConfig::default(),
            Arc::new(redactor.clone()),
        )
        .await
        .expect("a journal"),
    );
    let recorder = RecordingObserver::shared();
    let credentials = CredentialResolver::new("/nonexistent-user-state")
        .with_keychain(Arc::new(PanicsIfCredentialResolved))
        .with_environment(Arc::new(PanicsIfCredentialResolved));
    let request = RuntimeRequest {
        credentials: Some(credentials),
        persistence_redactor: Some(redactor.clone()),
        observers: vec![
            Arc::clone(&journal) as Arc<dyn EventObserver>,
            Arc::clone(&recorder) as Arc<dyn EventObserver>,
        ],
        ..request(&fixture, HostSurface::Headless)
    };

    let smith = factory::build(request)
        .await
        .expect("inline-key runtime construction");
    assert!(
        smith.policy().credential.is_none(),
        "runtime policy must not invent a display-safe credential locator"
    );
    let mut reflected = serde_json::json!({"value": TOKEN});
    redactor.redact(&mut reflected);
    assert_eq!(reflected["value"], "[redacted]");

    let session = smith
        .runtime()
        .start_session(StartSession::new())
        .await
        .expect("a session");
    session
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    let snapshot = serde_json::to_string(&session.snapshot()).expect("a serialized snapshot");
    session.shutdown().await.expect("a clean shutdown");
    journal.shutdown().await.expect("a flushed journal");
    let journaled = std::fs::read_to_string(journal_path).expect("a journal file");
    let events = format!("{:?}", recorder.events());
    let composition = format!(
        "{:?} {:?} {:?}",
        smith.policy(),
        smith.profile(),
        smith.runtime()
    );

    for (what, rendered) in [
        ("snapshot", snapshot),
        ("journal", journaled),
        ("events", events),
        ("composition", composition),
        ("redactor", format!("{redactor:?}")),
    ] {
        assert!(!rendered.contains(TOKEN), "{what} contains the inline key");
    }
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
        built.push(factory::build(request).await.expect("a runtime"));
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

#[tokio::test]
async fn catalog_only_selection_resolves_frozen_limits_before_any_provider_request() {
    let fixture = Fixture::new(ZAI_CATALOG_CONFIG);
    let snapshot: CatalogSnapshot =
        serde_json::from_str(EMBEDDED_MODELS_DEV_SEED).expect("embedded snapshot");
    let source = runtime_catalog_source(
        &snapshot,
        "zai",
        "openai-compatible",
        Some("https://api.z.ai/api/coding/paas/v4"),
    )
    .expect("Z.AI catalog source");
    let provider = Arc::new(FakeProvider::text_reply("unused"));
    let request = RuntimeRequest {
        credentials: Some(resolver(Some(TOKEN))),
        provider: Some(provider.clone() as Arc<dyn Provider>),
        catalog_sources: vec![source],
        ..request(&fixture, HostSurface::Headless)
    };

    let smith = factory::build(request)
        .await
        .expect("catalog-backed runtime");
    assert_eq!(
        smith.policy().model_profile.limits,
        ModelLimits::new(200_000, 200_000, 131_072)
    );
    let provenance = smith
        .policy()
        .model_profile
        .provenance_of(ProfileField::ContextTokens)
        .expect("catalog provenance");
    assert_eq!(provenance.source, CatalogSource::CachedRemote);
    assert_eq!(
        provenance.source_revision.as_deref(),
        Some(snapshot.source_revision.as_str())
    );
    assert_eq!(
        provenance.retrieved,
        Some(agent_runtime_core::clock::Timestamp(
            snapshot.retrieved_at_ms
        ))
    );
    assert!(
        provider.requests().is_empty(),
        "catalog-backed preflight must not send a provider request"
    );
}

#[tokio::test]
async fn explicit_limits_beat_catalog_metadata_and_both_provenances_survive() {
    const PARTIAL_LIMITS_CONFIG: &str = r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
max_output_tokens = 2048

[approval]
mode = "allow-all"
"#;

    let fixture = Fixture::new(PARTIAL_LIMITS_CONFIG);
    let cached = StaticSource::new("models.dev", CatalogSource::CachedRemote).with_model(
        "example-model",
        ModelRecord::new()
            .with_limits(ModelLimits::new(128_000, 124_000, 8_192))
            .with_revision("models-dev-r7"),
    );
    let request = RuntimeRequest {
        catalog_sources: vec![Arc::new(cached)],
        ..request(&fixture, HostSurface::Terminal)
    };

    let smith = factory::build(request).await.expect("a runtime");
    let resolution = smith.profile();

    // The configured value wins; the ones configuration was silent about fall
    // through to the catalog.
    assert_eq!(resolution.profile.limits.max_output_tokens, 2_048);
    assert_eq!(resolution.profile.limits.context_tokens, 128_000);
    assert_eq!(
        resolution
            .profile
            .provenance_of(ProfileField::MaxOutputTokens)
            .expect("catalog provenance")
            .source,
        CatalogSource::Explicit
    );
    assert_eq!(
        resolution
            .profile
            .provenance_of(ProfileField::ContextTokens)
            .expect("catalog provenance")
            .source_revision
            .as_deref(),
        Some("models-dev-r7")
    );

    // Both halves of the provenance survive: the shared catalog layer, and the
    // Smith file and key that supplied the winner.
    let offers = resolution.contributions_for(ProfileField::MaxOutputTokens);
    assert_eq!(offers.len(), 2);
    assert_eq!(
        (offers[0].layer, offers[0].value),
        (CatalogSource::Explicit, 2_048)
    );
    assert_eq!(
        (offers[1].layer, offers[1].value),
        (CatalogSource::CachedRemote, 8_192)
    );
    let configured = resolution
        .configured_source(ProfileField::MaxOutputTokens)
        .expect("a Smith configuration source");
    assert_eq!(configured.layer, Layer::ProjectFile);
    assert!(
        configured
            .file
            .as_ref()
            .expect("a file")
            .ends_with(".smith/config.toml"),
        "{configured}"
    );
    assert!(
        resolution
            .configured_source(ProfileField::ContextTokens)
            .is_none()
    );
}

#[tokio::test]
async fn host_policy_a_run_cannot_do_without_fails_before_anything_else() {
    // No workspace: the shared runtime would otherwise deny every tool call
    // with nothing to point the user at.
    let fixture = Fixture::new(FAKE_CONFIG);
    let err = factory::build(RuntimeRequest::new(fixture.config(), HostSurface::Terminal))
        .await
        .expect_err("no workspace");
    assert!(
        matches!(
            err,
            FactoryError::MissingHostPolicy {
                what: "workspace",
                ..
            }
        ),
        "{err}"
    );

    // `approval.mode = "ask"` with nothing to ask: a headless run must fail
    // closed rather than hang on a question nobody receives.
    const ASK_CONFIG: &str = r#"
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
    let fixture = Fixture::new(ASK_CONFIG);
    let err = factory::build(request(&fixture, HostSurface::Headless))
        .await
        .expect_err("no approval surface");
    assert!(
        matches!(
            err,
            FactoryError::MissingHostPolicy {
                what: "approval surface",
                ..
            }
        ),
        "{err}"
    );
    assert!(err.to_string().contains("allow-all"), "{err}");
}
