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
    ArtifactId, ArtifactRead, ArtifactRef, MAX_ARTIFACT_READ_BYTES,
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
use smith_runtime::background_tasks::BackgroundTaskRegistry;
use smith_runtime::checkpoint::{
    CheckpointKey, CheckpointKeyProvider, CheckpointProtectionError, SmithCheckpointStore,
};
use smith_runtime::factory::{HostSurface, MidTurnDurability, RuntimeRequest};
use smith_runtime::host::{HostSessionError, HostSessionRequest, list, start};
use smith_runtime::journal::{DefaultRedactor, JournalLine, JournalRecord, read_journal};
use smith_runtime::session::FileSessionStore;
use smith_runtime::{ChildDurability, ChildState, SpawnOutcome};

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

#[tokio::test]
async fn terminal_and_headless_runs_leave_agent_metadata_outside_the_project() {
    const INSTRUCTION_MARKER: &str = "HOST_DISCOVERED_AGENTS_MARKER";

    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let user_config_dir = home.path().join(".smith");
    std::fs::create_dir_all(&user_config_dir).expect("a user config directory");
    std::fs::write(user_config_dir.join("config.toml"), CONFIG).expect("a user config");
    std::fs::write(project.path().join("tracked.txt"), "project content\n")
        .expect("project fixture");
    std::fs::write(project.path().join("AGENTS.md"), INSTRUCTION_MARKER)
        .expect("project instructions");
    let before = project_tree(project.path());
    let mut instruction_identities = Vec::new();

    for surface in [HostSurface::Terminal, HostSurface::Headless] {
        let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved user configuration")
            .config;
        let provider = Arc::new(FakeProvider::new(
            "example-model",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "done".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ));
        let runtime = RuntimeRequest {
            provider: Some(provider.clone() as Arc<dyn Provider>),
            workspace: Some(Arc::new(
                ProjectWorkspace::new(project.path()).expect("a project workspace"),
            )),
            approval: Some(Arc::new(DenyAll)),
            ..RuntimeRequest::new(config, surface)
        };
        let host = start(
            HostSessionRequest::new(runtime, project.path())
                .checkpoint_keys(test_checkpoint_keys()),
        )
        .await
        .expect("a hosted session");
        let identity = host
            .runtime()
            .policy()
            .project_instructions
            .clone()
            .expect("root AGENTS.md is composition evidence");
        assert_eq!(identity.source, "AGENTS.md");
        instruction_identities.push(identity);
        host.session()
            .run(UserInput::text("answer without changing the checkout"))
            .await
            .expect("the turn completes");
        let wire =
            serde_json::to_string(&provider.requests()[0].messages).expect("a provider request");
        assert!(wire.contains(INSTRUCTION_MARKER), "{wire}");
        assert!(
            host.session()
                .history()
                .iter()
                .all(|message| !message.joined_text().contains(INSTRUCTION_MARKER)),
            "project instructions became canonical conversation history"
        );
        host.shutdown().await.expect("clean shutdown");
    }

    assert_eq!(instruction_identities[0], instruction_identities[1]);

    assert_eq!(project_tree(project.path()), before);
    for forbidden in [".smith", ".omo", "sessions", "timeline", "children"] {
        assert!(
            !project.path().join(forbidden).exists(),
            "Smith created project-local agent metadata `{forbidden}`"
        );
    }
    assert!(
        home.path().join(".smith/sessions").is_dir(),
        "durable state was not redirected to user storage"
    );
}

#[tokio::test]
async fn a_hosted_runtime_freezes_project_instructions_until_reconstruction() {
    let fixture = Fixture::new();
    let instructions = fixture.project.path().join("AGENTS.md");
    std::fs::write(&instructions, "FROZEN_INSTRUCTIONS_A").expect("first instructions");

    let first_provider = Arc::new(FakeProvider::text_reply("first done"));
    let mut first_request = fixture.request(HostSurface::Headless);
    first_request.runtime.provider = Some(first_provider.clone() as Arc<dyn Provider>);
    let first = start(first_request).await.expect("first hosted runtime");
    let first_identity = first
        .runtime()
        .policy()
        .project_instructions
        .clone()
        .expect("first instruction identity");

    std::fs::write(&instructions, "FROZEN_INSTRUCTIONS_B").expect("changed instructions");
    first
        .session()
        .run(UserInput::text("use the frozen instructions"))
        .await
        .expect("first turn");
    let first_wire =
        serde_json::to_string(&first_provider.requests()[0].messages).expect("first request");
    assert!(first_wire.contains("FROZEN_INSTRUCTIONS_A"), "{first_wire}");
    assert!(
        !first_wire.contains("FROZEN_INSTRUCTIONS_B"),
        "{first_wire}"
    );
    let first_cache_identity = first
        .session()
        .snapshot()
        .manifests
        .last()
        .expect("first manifest")
        .manifest
        .cache_plan_fingerprint
        .clone();
    first.shutdown().await.expect("first shutdown");

    let second_provider = Arc::new(FakeProvider::text_reply("second done"));
    let mut second_request = fixture.request(HostSurface::Headless);
    second_request.runtime.provider = Some(second_provider.clone() as Arc<dyn Provider>);
    let second = start(second_request).await.expect("second hosted runtime");
    let second_identity = second
        .runtime()
        .policy()
        .project_instructions
        .clone()
        .expect("second instruction identity");
    assert_ne!(first_identity.revision, second_identity.revision);

    second
        .session()
        .run(UserInput::text("use the reconstructed instructions"))
        .await
        .expect("second turn");
    let second_wire =
        serde_json::to_string(&second_provider.requests()[0].messages).expect("second request");
    assert!(
        second_wire.contains("FROZEN_INSTRUCTIONS_B"),
        "{second_wire}"
    );
    assert!(
        !second_wire.contains("FROZEN_INSTRUCTIONS_A"),
        "{second_wire}"
    );
    let second_cache_identity = second
        .session()
        .snapshot()
        .manifests
        .last()
        .expect("second manifest")
        .manifest
        .cache_plan_fingerprint
        .clone();
    assert_ne!(first_cache_identity, second_cache_identity);
    second.shutdown().await.expect("second shutdown");
}

#[tokio::test]
async fn invalid_project_instructions_fail_before_runtime_or_session_state() {
    let fixture = Fixture::new();
    std::fs::write(fixture.project.path().join("AGENTS.md"), [0xff, 0xfe])
        .expect("invalid UTF-8 instructions");
    let provider = Arc::new(FakeProvider::text_reply("must not run"));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone() as Arc<dyn Provider>);

    let error = start(request)
        .await
        .expect_err("invalid project instructions fail startup");
    assert!(error.to_string().contains("not valid UTF-8"), "{error}");
    assert!(provider.requests().is_empty(), "provider was contacted");
    assert!(
        !fixture.home.path().join(".smith/sessions").exists(),
        "invalid instructions created session state"
    );
}

fn project_tree(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn visit(root: &std::path::Path, current: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let mut entries = std::fs::read_dir(current)
            .expect("read project tree")
            .collect::<Result<Vec<_>, _>>()
            .expect("read project entry");
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            out.push(
                path.strip_prefix(root)
                    .expect("project-relative path")
                    .to_owned(),
            );
            if path.is_dir() {
                visit(root, &path, out);
            }
        }
    }

    let mut paths = Vec::new();
    visit(root, root, &mut paths);
    paths
}

#[tokio::test]
async fn protected_checkpoint_availability_is_explicit_and_encrypted() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a hosted session");
    assert_eq!(
        host.runtime().policy().mid_turn_durability,
        MidTurnDurability::Available
    );
    let session_id = host.session().id().clone();
    host.session()
        .run(UserInput::text("checkpoint secret marker"))
        .await
        .expect("the turn runs");
    let checkpoint = host
        .paths()
        .unwrap()
        .checkpoint(&session_id)
        .expect("checkpoint path");
    let bytes = tokio::fs::read(checkpoint)
        .await
        .expect("encrypted checkpoint");
    assert!(
        !bytes
            .windows("checkpoint secret marker".len())
            .any(|window| { window == b"checkpoint secret marker" })
    );
    host.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn configured_clear_checkpoint_key_makes_children_durable_without_keychain_fallback() {
    const KEY: &str = "5151515151515151515151515151515151515151515151515151515151515151";
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let config_dir = home.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a user config directory");
    let config_path = config_dir.join("config.toml");
    std::fs::write(
        &config_path,
        format!("{CONFIG}\n[persistence]\nenabled = true\ncheckpoint_key = \"{KEY}\"\n"),
    )
    .expect("a private user config");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600))
            .expect("private config permissions");
    }
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("the configured checkpoint key resolves")
        .config;
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: "durable child".to_owned(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])],
    ));
    let runtime = RuntimeRequest {
        provider: Some(provider),
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a project workspace"),
        )),
        approval: Some(Arc::new(AllowAll)),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };
    // Deliberately do not call HostSessionRequest::checkpoint_keys: startup
    // must select the resolved inline key before considering the platform
    // credential service.
    let host = start(HostSessionRequest::new(runtime, project.path()))
        .await
        .expect("the configured non-prompt key initializes persistence");
    assert_eq!(
        host.runtime().policy().mid_turn_durability,
        MidTurnDurability::Available
    );
    let coordinator = host
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("a delegation coordinator");
    let child = match coordinator
        .spawn(ChildSpec {
            task: UserInput::text("verify configured child durability"),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(1),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the child starts")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the child completes");
    assert_eq!(
        coordinator.status(&child).expect("child status").durability,
        ChildDurability::Durable
    );
    host.shutdown().await.expect("the host shuts down");
}

#[tokio::test]
async fn unavailable_checkpoint_key_keeps_completed_turn_persistence_honest() {
    let fixture = Fixture::new();
    let mut request = fixture.request(HostSurface::Headless);
    request.checkpoint_keys = Some(Arc::new(UnavailableCheckpointKeys));
    let host = start(request)
        .await
        .expect("completed-turn persistence remains");
    assert_eq!(
        host.runtime().policy().mid_turn_durability,
        MidTurnDurability::Unavailable
    );
    let session_id = host.session().id().clone();
    host.session()
        .run(UserInput::text("completed turn"))
        .await
        .expect("a turn without false checkpoint durability");
    let paths = host.paths().unwrap().clone();
    host.shutdown().await.expect("clean shutdown");
    assert!(paths.snapshot(&session_id).unwrap().is_file());
    assert!(!paths.checkpoint(&session_id).unwrap().exists());
}

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
async fn persistent_sessions_use_recoverable_semantic_summaries_with_disjoint_usage() {
    let fixture = Fixture::new();
    let main = |index: usize| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: format!("answer {index}"),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let summary = |text: &str, input: u64, output: u64| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta { text: text.into() },
            usage_event(input, output),
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            main(0),
            main(1),
            main(2),
            main(3),
            main(4),
            main(5),
            summary("SUMMARY_ONE", 40, 5),
            main(6),
            summary("SUMMARY_TWO", 50, 6),
        ],
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    // Pinned for the same reason as the fallback test: the scripted sequence
    // encodes a cadence, and the scripted streams report no usage, so the
    // completed-turn floor is what decides here.
    let mut summary_config = smith_runtime::summary::SmithSemanticSummaryConfig::standard();
    summary_config.policy.min_turns = 6;
    request.runtime.semantic_summary = Some(summary_config);
    let host = start(request).await.expect("a hosted session");
    let summary_policy = host
        .runtime()
        .policy()
        .semantic_summary
        .as_ref()
        .expect("persistent hosts enable the standard summary policy");
    assert_eq!(summary_policy.purpose, "context.semantic_summary");
    assert_eq!(summary_policy.min_turns, 6);
    assert_eq!(summary_policy.trigger_percent, 85);
    assert_eq!(summary_policy.retain_turns, 2);
    // Resolved from the model's declared input ceiling rather than guessed, so
    // the pressure comparison has something authoritative to measure against.
    assert_eq!(summary_policy.input_budget_tokens, 124_000);

    for index in 0..7 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .unwrap_or_else(|error| panic!("turn {index} failed: {error}"));
    }

    let requests = provider.requests();
    assert_eq!(requests.len(), 9);
    let first_summary = &requests[6];
    assert!(first_summary.tools.is_empty());
    assert_eq!(
        first_summary.tool_choice,
        agent_runtime_core::provider::ToolChoice::None
    );
    assert_eq!(first_summary.max_output_tokens, Some(2_048));
    let first_summary_wire =
        serde_json::to_string(&first_summary.messages).expect("first summary request");
    assert!(first_summary_wire.contains("context.semantic_summary"));
    assert!(first_summary_wire.contains("request 0"));
    assert!(first_summary_wire.contains("answer 3"));
    assert!(
        !first_summary_wire.contains("request 4"),
        "the retained exact suffix must stay out of summary input"
    );

    let projected_wire =
        serde_json::to_string(&requests[7].messages).expect("projected main request");
    assert!(projected_wire.contains("SUMMARY_ONE"));
    assert!(projected_wire.contains("request 4"));
    assert!(projected_wire.contains("answer 5"));
    assert!(projected_wire.contains("request 6"));
    assert!(
        !projected_wire.contains("request 0"),
        "the covered prefix must not coexist with its summary"
    );

    let second_summary_wire =
        serde_json::to_string(&requests[8].messages).expect("second summary request");
    assert!(second_summary_wire.contains("request 0"));
    assert!(second_summary_wire.contains("answer 4"));
    assert!(
        !second_summary_wire.contains("SUMMARY_ONE"),
        "semantic work must summarize canonical originals, not a prior projection"
    );

    let snapshot = host.session().snapshot();
    let state = snapshot
        .extension_state
        .get("harness.semantic_summary")
        .expect("protected summary state");
    assert_eq!(state.sensitivity, SessionStateSensitivity::Sensitive);
    assert_eq!(state.value["purpose"], "context.semantic_summary");
    assert_eq!(state.value["summary"], "SUMMARY_TWO");
    assert_eq!(state.value["omit_prefix"], 10);
    assert_eq!(
        snapshot
            .usage
            .records()
            .iter()
            .filter(|record| record.source == UsageSource::SemanticSummary)
            .count(),
        2
    );
    assert_eq!(
        snapshot
            .usage
            .total_for(UsageSource::SemanticSummary)
            .get(CounterKind::InputUncached),
        90
    );
    assert_eq!(
        snapshot
            .usage
            .total_for(UsageSource::SemanticSummary)
            .get(CounterKind::Output),
        11
    );
    assert_eq!(snapshot.manifests.len(), 7);
    assert_eq!(snapshot.manifests[6].manifest.summaries.len(), 1);
    assert_eq!(
        snapshot.manifests[6].manifest.summaries[0]
            .covered
            .iter()
            .map(|segment| segment.as_str())
            .collect::<Vec<_>>(),
        vec![
            "history:0",
            "history:1",
            "history:2",
            "history:3",
            "history:4",
            "history:5",
            "history:6",
            "history:7",
        ]
    );

    let source: ArtifactRef = serde_json::from_value(state.value["source_artifact"].clone())
        .expect("typed source artifact");
    assert_eq!(source.provenance.session, *host.session().id());
    assert_eq!(source.provenance.purpose, "context.semantic_summary");
    let store = host
        .runtime()
        .artifact_store()
        .expect("the protected original store");
    let mut offset = 0;
    let mut original = Vec::new();
    loop {
        let page = store
            .read(ArtifactRead {
                session: host.session().id().clone(),
                id: source.id.clone(),
                offset,
                limit: MAX_ARTIFACT_READ_BYTES,
            })
            .await
            .expect("a protected original page");
        original.extend_from_slice(&page.bytes);
        let Some(next) = page.next_offset else {
            break;
        };
        offset = next;
    }
    let original: Vec<agent_runtime_core::content::Message> =
        serde_json::from_slice(&original).expect("recoverable canonical originals");
    assert_eq!(original.len(), 10);
    assert_eq!(original[0].joined_text(), "request 0");
    assert_eq!(original[9].joined_text(), "answer 4");

    host.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_failed_semantic_summary_preserves_the_structural_history_plan() {
    let fixture = Fixture::new();
    let main = |index: usize| {
        ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: format!("answer {index}"),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])
    };
    let failed_summary = || {
        ScriptedStream::new(vec![ProviderStreamEvent::Error {
            error: ProviderError::new(ProviderErrorKind::Server, "summary unavailable"),
        }])
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            main(0),
            main(1),
            main(2),
            main(3),
            main(4),
            main(5),
            failed_summary(),
            main(6),
            failed_summary(),
        ],
    ));
    let observer = RecordingObserver::shared();
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    request.runtime.observers.push(observer.clone());
    // This test scripts an exact provider sequence, so it pins the cadence it
    // depends on instead of inheriting the product default. Pressure is not
    // measurable here — the scripted streams report no usage — so the floor is
    // what decides, and it must match the script.
    let mut summary_config = smith_runtime::summary::SmithSemanticSummaryConfig::standard();
    summary_config.policy.min_turns = 6;
    request.runtime.semantic_summary = Some(summary_config);
    let host = start(request).await.expect("a hosted session");

    for index in 0..7 {
        host.session()
            .run(UserInput::text(format!("request {index}")))
            .await
            .unwrap_or_else(|error| panic!("turn {index} failed: {error}"));
    }

    let requests = provider.requests();
    assert_eq!(requests.len(), 9);
    let seventh_main = serde_json::to_string(&requests[7].messages).expect("seventh main request");
    assert!(seventh_main.contains("request 0"));
    assert!(seventh_main.contains("answer 5"));
    assert!(seventh_main.contains("request 6"));
    assert!(
        !seventh_main.contains("SUMMARY_"),
        "failed summary output must never alter the deterministic structural plan"
    );
    assert!(
        !host
            .session()
            .snapshot()
            .extension_state
            .contains_key("harness.semantic_summary")
    );
    let fallbacks = observer
        .payloads()
        .into_iter()
        .filter(|event| {
            matches!(
                event,
                RuntimeEvent::Downgrade { capability, detail }
                    if capability == "semantic_summary"
                        && detail == "summary_model_unavailable"
            )
        })
        .count();
    assert_eq!(fallbacks, 2);

    host.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn todo_lifecycle_is_checkpointed_renderable_and_restored_as_context() {
    let fixture = Fixture::new();
    let write = |call: &str, items: serde_json::Value| {
        let mut events = tool_call_fragments(
            0,
            call,
            "write_todos",
            &serde_json::json!({ "items": items }).to_string(),
        );
        events.push(ProviderStreamEvent::Finish {
            reason: FinishReason::ToolCalls,
        });
        ScriptedStream::new(events)
    };
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            write(
                "plan-1",
                serde_json::json!([
                    {"id":"inspect","text":"Inspect the implementation","status":"in_progress"},
                    {"id":"change","text":"Implement the change","status":"pending"},
                    {"id":"verify","text":"Run focused tests","status":"pending"}
                ]),
            ),
            write(
                "plan-2",
                serde_json::json!([
                    {"id":"inspect","text":"Inspect the implementation","status":"completed"},
                    {"id":"change","text":"Implement the change","status":"completed"},
                    {"id":"verify","text":"Run focused tests","status":"in_progress"}
                ]),
            ),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "plan updated".into(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let observer = RecordingObserver::shared();
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    request.runtime.observers.push(observer.clone());
    let host = start(request).await.expect("a hosted session");
    assert!(
        host.runtime()
            .policy()
            .tools
            .iter()
            .any(|tool| tool == "write_todos")
    );

    host.session()
        .run(UserInput::text(
            "Use write_todos to track this genuinely multi-step edit.",
        ))
        .await
        .expect("the todo lifecycle completes");
    let payloads = observer.payloads();
    let plan_events = payloads
        .iter()
        .cloned()
        .filter_map(|event| match event {
            RuntimeEvent::PlanUpdated {
                revision,
                sensitivity,
                counts,
                items,
            } => Some((revision, sensitivity, counts, items)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        plan_events.len(),
        3,
        "events: {payloads:?}; requests: {:?}",
        provider.requests()
    );
    assert_eq!(plan_events[0].0, 1);
    assert_eq!(plan_events[1].0, 2);
    assert_eq!(plan_events[2].0, 3);
    assert_eq!(
        plan_events[1].1,
        agent_runtime_core::event::PlanSensitivity::Public
    );
    assert_eq!(plan_events[1].2["in_progress"], 1);
    assert_eq!(
        plan_events[1].3.as_ref().expect("public item projection")[2].text,
        "Run focused tests"
    );
    assert_eq!(plan_events[2].2["in_progress"], 0);
    assert_eq!(plan_events[2].2["pending"], 0);
    assert_eq!(plan_events[2].2["cancelled"], 1);
    let terminal_verify = &plan_events[2].3.as_ref().expect("terminal public plan")[2];
    assert_eq!(
        terminal_verify.status,
        agent_runtime_core::event::PlanItemStatus::Cancelled
    );
    assert_eq!(
        terminal_verify.reason.as_deref(),
        Some("turn_ended_unfinished")
    );

    let snapshot = host.session().snapshot();
    let state = snapshot
        .extension_state
        .get("harness.todo.state")
        .expect("checkpointed todo state");
    assert_eq!(state.sensitivity, SessionStateSensitivity::RedactionSafe);
    assert_eq!(state.value["revision"], 3);
    let session = host.session().id().clone();
    host.shutdown().await.expect("clean shutdown");

    let resumed_provider = Arc::new(FakeProvider::text_reply("resumed with plan"));
    let mut resume = fixture
        .request(HostSurface::Headless)
        .resume(session.clone());
    resume.runtime.provider = Some(resumed_provider.clone());
    let resumed = start(resume).await.expect("the todo session resumes");
    resumed
        .session()
        .run(UserInput::text("Continue the multi-step work."))
        .await
        .expect("the restored plan contributes to the next request");
    let requests = resumed_provider.requests();
    let wire = serde_json::to_string(&requests[0].messages).expect("provider messages");
    assert!(wire.contains(r#"<todo_plan revision=\"3\">"#), "{wire}");
    assert!(wire.contains("[cancelled] verify"), "{wire}");
    assert!(wire.contains("Run focused tests"), "{wire}");
    resumed.shutdown().await.expect("clean resumed shutdown");
}

#[tokio::test]
async fn terminal_resume_merges_protected_sensitive_state_over_the_plaintext_snapshot() {
    const SECRET: &str = "protected-memory-state-8f31";

    let fixture = Fixture::new();
    let session = SessionId::new("session-terminal-extension-state");
    let turn = TurnId::new("turn-1");
    let input = UserInput::text("retain protected component state");
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap();
    let mut snapshot = SessionSnapshot {
        id: session.clone(),
        history: vec![input.clone().into_message()],
        usage: UsageLedger::new(),
        identity: SessionIdentityState {
            turn: 1,
            event: 10,
            event_seq: 11,
            ..SessionIdentityState::default()
        },
        manifests: Vec::new(),
        extension_state: Default::default(),
        updated: Timestamp(4),
    };
    snapshot.extension_state.insert(
        "smith.todo".into(),
        VersionedSessionState::new(
            RegistryRevision::new("todo-state-1"),
            serde_json::json!({"pending": 1}),
        )
        .redaction_safe(),
    );
    snapshot.extension_state.insert(
        "smith.memory".into(),
        VersionedSessionState::new(
            RegistryRevision::new("memory-state-1"),
            serde_json::json!({"content": SECRET}),
        ),
    );

    let session_store = FileSessionStore::new(paths.clone());
    session_store
        .save(&snapshot)
        .await
        .expect("the ordinary snapshot saves");
    let ordinary = session_store
        .load(&session)
        .await
        .expect("the ordinary snapshot loads")
        .expect("an ordinary snapshot exists");
    assert!(ordinary.extension_state.contains_key("smith.todo"));
    assert!(!ordinary.extension_state.contains_key("smith.memory"));
    let ordinary_bytes = tokio::fs::read(paths.snapshot(&session).expect("ordinary snapshot path"))
        .await
        .expect("ordinary snapshot bytes");
    assert!(
        !ordinary_bytes
            .windows(SECRET.len())
            .any(|window| window == SECRET.as_bytes())
    );

    let checkpoint_store =
        SmithCheckpointStore::initialize_with(paths.clone(), test_checkpoint_keys())
            .await
            .expect("a protected checkpoint store");
    let accepted = TurnCheckpoint::accepted(
        turn,
        input,
        snapshot.clone(),
        0,
        Deadline::never(),
        1,
        7,
        Timestamp(1),
    )
    .expect("an accepted checkpoint");
    checkpoint_store
        .save(&accepted)
        .await
        .expect("accepted checkpoint");
    let completing = accepted
        .transition(
            TurnState::Completing {
                finish: TurnFinish::Completed,
                visible_output: false,
                provider_error_kind: None,
            },
            snapshot.clone(),
            8,
            Timestamp(2),
        )
        .expect("a completing checkpoint");
    checkpoint_store
        .save(&completing)
        .await
        .expect("completing checkpoint");
    let publishing = completing
        .transition(
            TurnState::PublishingTerminal {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
            snapshot.clone(),
            9,
            Timestamp(3),
        )
        .expect("a publishing checkpoint");
    checkpoint_store
        .save(&publishing)
        .await
        .expect("publishing checkpoint");
    let terminal = publishing
        .transition(
            TurnState::Terminal {
                finish: TurnFinish::Completed,
                visible_output: false,
            },
            snapshot,
            10,
            Timestamp(4),
        )
        .expect("a terminal checkpoint");
    checkpoint_store
        .save(&terminal)
        .await
        .expect("terminal checkpoint");

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect("the terminal session resumes");
    let restored = resumed.session().snapshot();
    assert_eq!(
        restored.extension_state["smith.memory"].value,
        serde_json::json!({"content": SECRET})
    );
    assert_eq!(
        restored.extension_state["smith.todo"].value,
        serde_json::json!({"pending": 1})
    );
    resumed.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn a_checkpoint_only_first_turn_is_resumable_without_a_completed_snapshot() {
    let fixture = Fixture::new();
    let session = SessionId::new("session-first-turn-crash");
    let turn = TurnId::new("turn-1");
    let input = UserInput::text("resume the first accepted turn");
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path()).unwrap();
    let mut snapshot = SessionSnapshot {
        id: session.clone(),
        history: vec![input.clone().into_message()],
        usage: UsageLedger::new(),
        identity: SessionIdentityState {
            turn: 1,
            event: 7,
            event_seq: 7,
            ..SessionIdentityState::default()
        },
        manifests: Vec::new(),
        extension_state: Default::default(),
        updated: Timestamp::ZERO,
    };
    let checkpoint = TurnCheckpoint::accepted(
        turn,
        input,
        snapshot.clone(),
        0,
        Deadline::never(),
        1,
        7,
        Timestamp::ZERO,
    )
    .unwrap();
    let checkpoint_store =
        SmithCheckpointStore::initialize_with(paths.clone(), test_checkpoint_keys())
            .await
            .unwrap();
    checkpoint_store.save(&checkpoint).await.unwrap();
    assert!(
        !paths.snapshot(&session).unwrap().exists(),
        "the fixture must model a crash before the first completed snapshot"
    );

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect("the protected checkpoint proves the session exists");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            snapshot = resumed.session().snapshot();
            if snapshot
                .history
                .iter()
                .any(|message| message.role == agent_runtime_core::content::Role::Assistant)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the accepted turn resumed");
    resumed.shutdown().await.expect("clean shutdown");
    assert!(paths.snapshot(&session).unwrap().is_file());
}

#[tokio::test]
async fn only_one_host_can_own_a_persistent_session_lifecycle() {
    let fixture = Fixture::new();
    let first = start(fixture.request(HostSurface::Headless))
        .await
        .expect("the first host");
    first
        .session()
        .run(UserInput::text("establish a resumable session"))
        .await
        .expect("a completed turn");
    let session = first.session().id().clone();

    let error = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session.clone()),
    )
    .await
    .expect_err("a second active owner must fail instead of waiting");
    assert!(
        matches!(
            error,
            HostSessionError::Runtime(ref error)
                if error.kind == agent_runtime_core::error::ErrorKind::Conflict
                    && error.message.contains("already active")
        ),
        "{error}"
    );

    first.shutdown().await.expect("release the lifecycle lease");
    let second = start(fixture.request(HostSurface::Headless).resume(session))
        .await
        .expect("the lease is recoverable after shutdown");
    second.shutdown().await.expect("clean second shutdown");
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
async fn project_configuration_cannot_silently_grant_tool_authority() {
    let home = tempfile::tempdir().expect("a user root");
    let project = tempfile::tempdir().expect("a project root");
    let config_dir = project.path().join(".smith");
    std::fs::create_dir_all(&config_dir).expect("a project config directory");
    std::fs::write(
        config_dir.join("config.toml"),
        format!("{CONFIG}\n[approval]\nmode = \"allow-all\"\nauto_approve = [\"edit\"]\n"),
    )
    .expect("a project config");
    let config = resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
        .expect("resolved configuration")
        .config;
    let runtime = RuntimeRequest {
        workspace: Some(Arc::new(
            ProjectWorkspace::new(project.path()).expect("a workspace"),
        )),
        ..RuntimeRequest::new(config, HostSurface::Headless)
    };

    let error = start(HostSessionRequest::new(runtime, project.path()))
        .await
        .expect_err("project config must not grant authority");
    assert!(
        matches!(
            error,
            HostSessionError::ProjectGrantedAuthority {
                setting: "approval.mode",
                ..
            }
        ),
        "{error}"
    );
    assert!(
        !home.path().join(".smith/sessions").exists(),
        "authority failure created session state"
    );
}

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
async fn a_session_is_saved_listed_and_resumed_with_its_canonical_history() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    let session_id = host.session().id().clone();
    let paths = host.paths().expect("persistent paths").clone();

    host.session()
        .run(UserInput::text("remember this"))
        .await
        .expect("the turn runs");
    host.session()
        .run(UserInput::text("and preserve every manifest"))
        .await
        .expect("the second turn runs");
    assert!(
        host.session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("remember this")),
        "the canonical history did not retain the user input"
    );
    assert_eq!(host.session().snapshot().manifests.len(), 2);
    assert!(
        paths
            .snapshot(&session_id)
            .expect("snapshot path")
            .is_file(),
        "a completed turn must be persisted before orderly shutdown"
    );
    host.shutdown().await.expect("a clean first shutdown");

    assert!(
        paths
            .snapshot(&session_id)
            .expect("snapshot path")
            .is_file()
    );
    assert!(paths.journal(&session_id).expect("journal path").is_file());
    let listings = list(&fixture.config(), fixture.project.path())
        .await
        .expect("session listings");
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].id, session_id);

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session_id.clone()),
    )
    .await
    .expect("a resumed hosted session");
    assert_eq!(resumed.session().id(), &session_id);
    assert_eq!(
        resumed.session().snapshot().manifests.len(),
        2,
        "resume discarded historical manifests"
    );
    assert!(
        resumed
            .session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("remember this")),
        "resume discarded the prior canonical history"
    );
    resumed
        .session()
        .run(UserInput::text("append a third manifest"))
        .await
        .expect("the resumed turn runs");
    assert_eq!(
        resumed.session().snapshot().manifests.len(),
        3,
        "the resumed turn replaced historical manifests"
    );
    resumed.shutdown().await.expect("a clean resumed shutdown");

    let stored = FileSessionStore::new(paths.clone())
        .load(&session_id)
        .await
        .expect("saved snapshot")
        .expect("snapshot remains present");
    assert_eq!(
        stored.manifests.len(),
        3,
        "the resumed save lost historical manifests"
    );

    let recovery = read_journal(paths.journal(&session_id).expect("journal path"))
        .await
        .expect("a readable journal");
    assert!(
        recovery.events().len() >= 4,
        "create and resume did not both append canonical lifecycle events"
    );
    assert!(
        recovery.truncated_tail.is_none(),
        "ordered shutdown left a partial journal record"
    );
}

#[tokio::test]
async fn journal_events_between_returns_exactly_the_missed_range() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");

    let all = host.timeline_events().await.expect("timeline events");
    assert!(all.len() >= 3, "a completed turn appends lifecycle events");

    // A lagged live subscriber asks for an interior range: the healing read
    // must return exactly those events, in order, and nothing else.
    let first = all[1].seq;
    let last = all[all.len() - 2].seq;
    let range = host
        .journal_events_between(first, last)
        .await
        .expect("a ranged journal read");
    assert_eq!(
        range.iter().map(|event| event.seq).collect::<Vec<_>>(),
        all.iter()
            .map(|event| event.seq)
            .filter(|seq| (first..=last).contains(seq))
            .collect::<Vec<_>>(),
    );

    let beyond = host
        .journal_events_between(u64::MAX - 1, u64::MAX)
        .await
        .expect("an empty ranged journal read");
    assert!(
        beyond.is_empty(),
        "a range the journal never saw must come back empty, not invented"
    );
    host.shutdown().await.expect("a clean shutdown");
}

#[tokio::test]
async fn journal_events_between_serves_a_recent_range_from_the_ring_without_reading_the_journal() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");

    let all = host.timeline_events().await.expect("timeline events");
    assert!(all.len() >= 3, "a completed turn appends lifecycle events");
    let first = all[1].seq;
    let last = all[all.len() - 2].seq;

    // The ring observed every one of these events synchronously as they were
    // emitted, well within its bound. Delete the durable journal now: if the
    // read below fell back to disk instead of the ring, it would fail
    // outright rather than quietly reading the wrong thing.
    let journal_path = host
        .paths()
        .expect("persistent paths")
        .journal(host.session().id())
        .expect("journal path");
    std::fs::remove_file(&journal_path).expect("the journal file is removable");

    let range = host
        .journal_events_between(first, last)
        .await
        .expect("a ring-served read that never touches the now-missing journal file");
    assert_eq!(
        range.iter().map(|event| event.seq).collect::<Vec<_>>(),
        all.iter()
            .map(|event| event.seq)
            .filter(|seq| (first..=last).contains(seq))
            .collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn journal_events_between_falls_back_to_the_journal_when_the_ring_cannot_cover_the_range() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("a new hosted session");
    let session_id = host.session().id().clone();
    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    let all = host.timeline_events().await.expect("timeline events");
    assert!(all.len() >= 3, "a completed turn appends lifecycle events");
    let first = all[1].seq;
    let last = all[all.len() - 2].seq;
    host.shutdown().await.expect("a clean shutdown");

    // A resumed process starts with an empty in-memory ring: this range
    // predates it entirely, so only the durable journal has it.
    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session_id.clone()),
    )
    .await
    .expect("a resumed hosted session");
    let range = resumed
        .journal_events_between(first, last)
        .await
        .expect("a journal-fallback read after resume");
    assert_eq!(
        range.iter().map(|event| event.seq).collect::<Vec<_>>(),
        all.iter()
            .map(|event| event.seq)
            .filter(|seq| (first..=last).contains(seq))
            .collect::<Vec<_>>(),
        "the fallback must still return exactly the requested range"
    );
    resumed.shutdown().await.expect("a clean resumed shutdown");
}

#[tokio::test]
async fn durable_child_follow_up_survives_a_full_smith_host_restart() {
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "remembered parser constraints".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
            ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "follow-up regression risk".to_owned(),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ]),
        ],
    ));
    let mut first_request = fixture.request(HostSurface::Headless);
    first_request.runtime.provider = Some(provider.clone());
    let first = start(first_request).await.expect("the first Smith host");
    let parent = first.session().id().clone();
    let first_coordinator = first
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("the first coordinator");
    let child = match first_coordinator
        .spawn(ChildSpec {
            task: UserInput::text("Inspect the parser and retain its important constraints."),
            model: ChildModelSelection::Inherit,
            limits: ChildLimits::turns(3),
            tools: ToolViewScope::ReadOnly,
            workspace: WorkspacePolicy::ReadOnlyView,
        })
        .await
        .expect("the durable child starts")
    {
        SpawnOutcome::Spawned { child, .. } => child,
        other => panic!("expected a spawned child, got {other:?}"),
    };
    first_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the first task completes");
    let before = first_coordinator
        .status(&child)
        .expect("first child status");
    assert_eq!(before.durability, ChildDurability::Durable);
    assert_eq!(before.state, ChildState::Idle);
    first.shutdown().await.expect("the first host shuts down");

    let mut resume_request = fixture
        .request(HostSurface::Headless)
        .resume(parent.clone());
    resume_request.runtime.provider = Some(provider.clone());
    let resumed = start(resume_request)
        .await
        .expect("the same Smith host session resumes");
    assert!(
        resumed.recovered_ephemeral_work().is_none(),
        "a durable catalog child was mislabeled legacy ephemeral"
    );
    let resumed_coordinator = resumed
        .runtime()
        .delegation()
        .and_then(|delegation| delegation.coordinator())
        .expect("the resumed coordinator");
    let recovered = resumed_coordinator
        .status(&child)
        .expect("the same child identity is retained");
    assert_eq!(recovered.parent, parent);
    assert_eq!(recovered.session, before.session);
    assert_eq!(recovered.state, ChildState::Idle);

    resumed_coordinator
        .follow_up(
            &child,
            UserInput::text("Identify the highest-risk regression using that retained review."),
        )
        .await
        .expect("the recovered child accepts a follow-up");
    resumed_coordinator
        .wait_task_outcome(&child)
        .await
        .expect("the follow-up completes");
    let after = resumed_coordinator
        .status(&child)
        .expect("follow-up status");
    assert_eq!(after.session, before.session);
    assert_eq!(after.turns_used, 2);

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let follow_up_wire =
        serde_json::to_string(&requests[1].messages).expect("follow-up provider request");
    assert!(
        follow_up_wire.contains("Inspect the parser"),
        "{follow_up_wire}"
    );
    assert!(
        follow_up_wire.contains("remembered parser constraints"),
        "{follow_up_wire}"
    );
    assert!(
        follow_up_wire.contains("highest-risk regression"),
        "{follow_up_wire}"
    );
    resumed
        .shutdown()
        .await
        .expect("the resumed host shuts down");
}

#[tokio::test]
async fn resume_marks_unresolved_ephemeral_work_interrupted_without_restarting_it() {
    let fixture = Fixture::new();
    let first = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a first host");
    let session_id = first.session().id().clone();
    let paths = first.paths().expect("persistent paths").clone();
    first.shutdown().await.expect("persist the base session");

    // Replace the orderly terminal tail with the state a crashed process
    // would have left: a child was spawned and no task-resolution event was
    // committed. The saved session/checkpoint remains the authoritative
    // resumable root state.
    let journal_path = paths.journal(&session_id).expect("journal path");
    let recovery = read_journal(&journal_path)
        .await
        .expect("the first journal reads");
    let mut records = recovery
        .records
        .into_iter()
        .filter(|line| {
            !matches!(
                line.record,
                JournalRecord::Event {
                    event: EventEnvelope {
                        payload: RuntimeEvent::SessionShutdown,
                        ..
                    }
                }
            )
        })
        .collect::<Vec<_>>();
    let mut next_seq = records
        .iter()
        .filter_map(|line| match &line.record {
            JournalRecord::Event { event } => Some(event.seq),
            _ => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let spawned = |child: &ChildId| RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 2,
        max_tokens: None,
        deadline_ms: None,
    };
    let running = ChildId::new("child-running-before-crash");
    let completed = ChildId::new("child-completed-before-crash");
    let needs_input = ChildId::new("child-needs-input-before-crash");
    let stopped = ChildId::new("child-stopped-before-crash");
    let failed = ChildId::new("child-failed-before-crash");
    {
        let mut append = |payload| {
            let seq = next_seq;
            next_seq = next_seq.saturating_add(1);
            records.push(JournalLine::new(JournalRecord::Event {
                event: EventEnvelope::new(
                    seq,
                    EventId::new(format!("evt-before-crash-{seq}")),
                    session_id.clone(),
                    None,
                    Timestamp(50),
                    payload,
                ),
            }));
        };
        append(spawned(&running));
        append(spawned(&completed));
        append(RuntimeEvent::ChildCompleted {
            child: completed.clone(),
            result: "available for follow-up".to_owned(),
        });
        append(spawned(&needs_input));
        append(RuntimeEvent::ChildNeedsInput {
            child: needs_input.clone(),
            child_session: SessionId::new("child-session-before-crash"),
            turn: TurnId::new("child-turn-before-crash"),
            call: ToolCallId::new("child-call-before-crash"),
            request: InteractionRequestId::new("child-request-before-crash"),
            question_ids: vec![QuestionId::new("child-question-before-crash")],
            sensitivity: InteractionSensitivity::Sensitive,
        });
        append(spawned(&stopped));
        append(RuntimeEvent::ChildStopped {
            child: stopped,
            reason: CancelReason::Shutdown,
        });
        append(spawned(&failed));
        append(RuntimeEvent::ChildFailed {
            child: failed,
            error: agent_runtime_core::error::RuntimeError::internal("child failed"),
        });
    }
    let running_monitor = "monitor:build-before-crash".to_owned();
    let stopped_monitor = "monitor:lint-before-crash".to_owned();
    records.push(JournalLine::new(JournalRecord::MonitorStarted {
        monitor: running_monitor.clone(),
    }));
    records.push(JournalLine::new(JournalRecord::MonitorStarted {
        monitor: stopped_monitor.clone(),
    }));
    records.push(JournalLine::new(JournalRecord::MonitorStopped {
        monitor: stopped_monitor,
    }));
    let mut bytes = Vec::new();
    for line in &records {
        serde_json::to_writer(&mut bytes, line).expect("a serializable journal line");
        bytes.push(b'\n');
    }
    tokio::fs::write(&journal_path, bytes)
        .await
        .expect("the crash fixture is installed");

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session_id.clone()),
    )
    .await
    .expect("the interrupted session resumes");
    let interruption = resumed
        .recovered_ephemeral_work()
        .expect("an explicit interruption marker");
    assert_eq!(
        interruption.children,
        [completed.clone(), needs_input.clone(), running.clone()]
    );
    assert_eq!(
        interruption.monitors.as_slice(),
        std::slice::from_ref(&running_monitor)
    );
    assert!(
        resumed
            .runtime()
            .delegation()
            .and_then(|delegation| delegation.coordinator())
            .expect("a fresh coordinator")
            .list()
            .is_empty(),
        "resume recreated an ephemeral child"
    );
    resumed
        .shutdown()
        .await
        .expect("the resumed host shuts down");

    let recovery = read_journal(&journal_path)
        .await
        .expect("the reconciled journal reads");
    let markers = recovery
        .records
        .iter()
        .filter_map(|line| match &line.record {
            JournalRecord::EphemeralWorkInterrupted { interruption } => Some(interruption),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].children, [completed, needs_input, running]);
    assert_eq!(markers[0].monitors, [running_monitor]);

    // The marker participates in the next scan, so the same prior work is not
    // reported or appended twice.
    let resumed_again = start(fixture.request(HostSurface::Headless).resume(session_id))
        .await
        .expect("a second resume");
    assert!(resumed_again.recovered_ephemeral_work().is_none());
    resumed_again
        .shutdown()
        .await
        .expect("the second resume shuts down");
}

/// A background task start marker with no terminal marker is the same
/// crash shape as an unresolved monitor or child: resume must report it
/// through `recovered_ephemeral_work` and must never spawn a process for it,
/// since the in-memory registry that could own such a process starts empty
/// in every new Smith process.
#[tokio::test]
async fn resume_marks_an_unresolved_background_task_interrupted_without_spawning_it() {
    let fixture = Fixture::new();
    let first = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a first host");
    let session_id = first.session().id().clone();
    let paths = first.paths().expect("persistent paths").clone();
    first.shutdown().await.expect("persist the base session");

    let journal_path = paths.journal(&session_id).expect("journal path");
    let recovery = read_journal(&journal_path)
        .await
        .expect("the first journal reads");
    let mut records = recovery
        .records
        .into_iter()
        .filter(|line| {
            !matches!(
                line.record,
                JournalRecord::Event {
                    event: EventEnvelope {
                        payload: RuntimeEvent::SessionShutdown,
                        ..
                    }
                }
            )
        })
        .collect::<Vec<_>>();

    let task = "task:before-crash".to_owned();
    records.push(JournalLine::new(JournalRecord::TaskStarted {
        task: task.clone(),
    }));

    let mut bytes = Vec::new();
    for line in &records {
        serde_json::to_writer(&mut bytes, line).expect("a serializable journal line");
        bytes.push(b'\n');
    }
    tokio::fs::write(&journal_path, bytes)
        .await
        .expect("the crash fixture is installed");

    let resumed = start(
        fixture
            .request(HostSurface::Headless)
            .resume(session_id.clone()),
    )
    .await
    .expect("the interrupted session resumes");
    let interruption = resumed
        .recovered_ephemeral_work()
        .expect("an explicit interruption marker");
    assert_eq!(interruption.tasks.as_slice(), std::slice::from_ref(&task));
    assert!(
        BackgroundTaskRegistry::global()
            .running_tasks(&session_id)
            .is_empty(),
        "resume must never spawn a process for a recovered background task"
    );

    resumed
        .shutdown()
        .await
        .expect("the resumed host shuts down");

    let recovery = read_journal(&journal_path)
        .await
        .expect("the reconciled journal reads");
    let markers = recovery
        .records
        .iter()
        .filter_map(|line| match &line.record {
            JournalRecord::EphemeralWorkInterrupted { interruption } => Some(interruption),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].tasks, [task]);
}

/// The host must never let a background task outlive its process: shutdown
/// signals every running task and waits, bounded, for its worker to kill the
/// owned process group and record the terminal journal marker — it must not
/// wait out the task's own duration.
#[tokio::test]
async fn shutdown_kills_a_running_background_task_within_the_grace_period() {
    let fixture = Fixture::new();
    let host = start(fixture.request(HostSurface::Headless))
        .await
        .expect("a hosted session");
    let session_id = host.session().id().clone();

    BackgroundTaskRegistry::global()
        .spawn_background_task(
            &session_id,
            "sleep 30".to_owned(),
            fixture.project.path().to_path_buf(),
            None,
        )
        .await
        .expect("a background task spawns");
    assert_eq!(
        BackgroundTaskRegistry::global()
            .running_tasks(&session_id)
            .len(),
        1,
        "the task is registered as running before shutdown"
    );

    let started = std::time::Instant::now();
    host.shutdown().await.expect("shutdown completes");
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "shutdown must not wait out the task's own 30s sleep: {elapsed:?}"
    );
    assert!(
        BackgroundTaskRegistry::global()
            .running_tasks(&session_id)
            .is_empty(),
        "shutdown must kill the task's owned process group before returning"
    );

    let paths = host.paths().expect("persistent paths");
    let journal_path = paths.journal(&session_id).expect("journal path");
    let recovery = read_journal(&journal_path)
        .await
        .expect("the journal reads");
    let started_markers = recovery
        .records
        .iter()
        .filter(|line| matches!(&line.record, JournalRecord::TaskStarted { .. }))
        .count();
    let exited_markers = recovery
        .records
        .iter()
        .filter(|line| matches!(&line.record, JournalRecord::TaskExited { .. }))
        .count();
    assert_eq!(started_markers, 1);
    assert_eq!(
        exited_markers, 1,
        "the worker's terminal marker must land before shutdown closes the journal"
    );
}

/// A background task's terminal notification is model-facing content, so it
/// must reach the parent the same way a completed child's result does: as
/// must-deliver injected content picked up at the next safe boundary, never
/// mutating a response already in flight.
#[tokio::test]
async fn a_background_task_terminal_notification_reaches_the_parent_model_at_a_safe_boundary() {
    let fixture = Fixture::new();
    let provider = Arc::new(FakeProvider::new(
        "example-model",
        Capabilities::basic_streaming(),
        vec![ScriptedStream::new(vec![
            ProviderStreamEvent::TextDelta {
                text: "done".to_owned(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::Stop,
            },
        ])],
    ));
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(provider.clone());
    let host = start(request).await.expect("a hosted session");
    let session_id = host.session().id().clone();

    BackgroundTaskRegistry::global()
        .spawn_background_task(
            &session_id,
            "true".to_owned(),
            fixture.project.path().to_path_buf(),
            None,
        )
        .await
        .expect("a background task spawns");

    // The worker task runs concurrently; wait for it to leave the running
    // set, then give it one more beat to finish injecting the notification,
    // the same margin `HostSession::shutdown` gives it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !BackgroundTaskRegistry::global()
        .running_tasks(&session_id)
        .is_empty()
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the trivial background task did not reach a terminal state in time"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    host.session()
        .run(UserInput::text("anything"))
        .await
        .expect("the parent turn runs");

    let deliveries = provider.requests()[0]
        .messages
        .iter()
        .filter_map(|message| {
            let text = message.joined_text();
            text.contains(r#""type":"background_task_terminal""#)
                .then(|| serde_json::from_str::<serde_json::Value>(&text).expect("typed delivery"))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        deliveries.len(),
        1,
        "the terminal notification is injected exactly once"
    );
    assert_eq!(deliveries[0]["status"], "exited");
    assert_eq!(deliveries[0]["exit_code"], 0);

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

    assert_eq!(invocation, "Edit(tracked.txt)");
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

#[tokio::test]
async fn registered_credentials_are_removed_from_persisted_history_and_events() {
    const SECRET: &str = "sk-live-persistence-secret";

    let fixture = Fixture::new();
    let mut request = fixture.request(HostSurface::Headless);
    request.runtime.provider = Some(Arc::new(FakeProvider::text_reply(format!(
        "reflected {SECRET}"
    ))) as Arc<dyn Provider>);
    request.runtime.persistence_redactor = Some(DefaultRedactor::new().with_secret(SECRET));
    let host = start(request).await.expect("a hosted session");
    let paths = host.paths().expect("persistent paths").clone();
    let session_id = host.session().id().clone();

    host.session()
        .run(UserInput::text("hello"))
        .await
        .expect("the turn runs");
    host.shutdown().await.expect("a clean shutdown");

    let snapshot = std::fs::read_to_string(
        paths
            .snapshot(&session_id)
            .expect("a persisted snapshot path"),
    )
    .expect("a persisted snapshot");
    let journal = std::fs::read_to_string(
        paths
            .journal(&session_id)
            .expect("a persisted journal path"),
    )
    .expect("a persisted journal");
    for (name, persisted) in [("snapshot", snapshot), ("journal", journal)] {
        assert!(!persisted.contains(SECRET), "{name} leaked the credential");
        assert!(
            persisted.contains("[redacted]"),
            "{name} did not retain an explicit redaction marker"
        );
    }

    let mut resume = fixture.request(HostSurface::Headless);
    resume.runtime.persistence_redactor = Some(DefaultRedactor::new().with_secret(SECRET));
    let resumed = start(
        HostSessionRequest::new(resume.runtime, fixture.project.path())
            .checkpoint_keys(test_checkpoint_keys())
            .resume(session_id),
    )
    .await
    .expect("a redacted session still resumes");
    let history = resumed
        .session()
        .history()
        .iter()
        .map(|message| message.joined_text())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!history.contains(SECRET), "{history}");
    assert!(history.contains("[redacted]"), "{history}");
    resumed.shutdown().await.expect("a clean resumed shutdown");
}

#[tokio::test]
async fn provider_switch_rebuilds_and_resumes_the_same_canonical_session() {
    const SWITCH_CONFIG: &str = r#"
default_profile = "first"

[profiles.first]
provider = "alpha"
model = "model-a"

[profiles.second]
provider = "beta"
model = "model-b"

[providers.alpha]
kind = "fake"

[providers.beta]
kind = "fake"

[models."alpha/model-a"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."beta/model-b"]
context_tokens = 64000
max_input_tokens = 60000
max_output_tokens = 2048
"#;

    let fixture = Fixture::with_config(SWITCH_CONFIG);
    let first = start(fixture.request(HostSurface::Terminal))
        .await
        .expect("the first runtime");
    let session_id = first.session().id().clone();
    first
        .session()
        .run(UserInput::text("first turn"))
        .await
        .expect("the first turn runs");
    first.shutdown().await.expect("the first runtime saved");

    let switched = resolve(
        &ResolveRequest::new(fixture.project.path())
            .with_home_dir(fixture.home.path())
            .with_cli(Overrides {
                profile: Some("second".into()),
                ..Overrides::default()
            }),
    )
    .expect("the second profile resolves")
    .config;
    let second = start(
        fixture
            .request_with_config(switched, HostSurface::Terminal)
            .resume(session_id.clone()),
    )
    .await
    .expect("the rebuilt runtime resumes");
    assert_eq!(second.session().id(), &session_id);
    assert_eq!(second.runtime().policy().provider_name, "beta");
    assert_eq!(second.runtime().policy().model.as_str(), "model-b");
    assert!(
        second
            .session()
            .history()
            .iter()
            .any(|message| message.joined_text().contains("first turn")),
        "the switch discarded canonical history"
    );

    second
        .session()
        .run(UserInput::text("second turn"))
        .await
        .expect("the second turn runs");
    let snapshot = second.session().snapshot();
    assert_eq!(
        snapshot
            .manifests
            .last()
            .expect("a second manifest")
            .manifest
            .model
            .provider,
        "beta"
    );
    second.shutdown().await.expect("the switched runtime saved");
}

#[tokio::test]
async fn resume_refuses_an_unknown_identity_instead_of_creating_it() {
    let fixture = Fixture::new();
    let missing = SessionId::new("session-does-not-exist");
    let paths = smith_runtime::host::paths(&fixture.config(), fixture.project.path())
        .expect("session paths");

    let error = start(
        fixture
            .request(HostSurface::Terminal)
            .resume(missing.clone()),
    )
    .await
    .expect_err("resume must not create a missing session");
    assert!(
        matches!(
            error,
            HostSessionError::SessionNotFound { ref session } if session == &missing
        ),
        "{error}"
    );
    assert!(
        list(&fixture.config(), fixture.project.path())
            .await
            .expect("session listings")
            .is_empty(),
        "a failed resume left session state behind"
    );
    assert!(
        !paths.directory().exists(),
        "a missing resume identity created `{}`",
        paths.directory().display()
    );
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

#[tokio::test]
async fn preflight_failure_does_not_create_a_journal_or_session_directory() {
    let fixture = Fixture::new();
    let config = fixture.config();
    let sessions_dir = config.persistence.sessions_dir.value.clone();
    let runtime = RuntimeRequest::new(config, HostSurface::Headless);

    let error = start(HostSessionRequest::new(runtime, fixture.project.path()))
        .await
        .expect_err("a missing workspace must fail preflight");
    assert!(
        matches!(error, HostSessionError::Factory(_)),
        "unexpected error: {error}"
    );
    assert!(
        !sessions_dir.exists(),
        "failed preflight created persistence state at {}",
        sessions_dir.display()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn non_utf8_project_paths_keep_distinct_session_partitions() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let root = tempfile::tempdir().expect("a root");
    let first = root.path().join(OsString::from_vec(vec![b'p', 0x80]));
    let second = root.path().join(OsString::from_vec(vec![b'p', 0x81]));
    std::fs::create_dir(&first).expect("first project");
    std::fs::create_dir(&second).expect("second project");

    assert_ne!(
        smith_runtime::host::project_id(first).expect("first id"),
        smith_runtime::host::project_id(second).expect("second id"),
        "lossy path conversion merged distinct projects"
    );
}
