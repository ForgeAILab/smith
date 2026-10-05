use super::*;
use crate::journal::JournalLine;
use crate::resume_capsule::{ResumeSummaryOutcome, ResumeSummaryPurpose};
use agent_runtime_core::artifact::{ArtifactDigest, ArtifactId};
use agent_runtime_core::cancel::CancelReason;
use agent_runtime_core::content::ToolCall;
use agent_runtime_core::delegation::WorkspacePolicy;
use agent_runtime_core::ids::{EventId, QuestionId};
use agent_runtime_core::interaction::InteractionSensitivity;
use agent_runtime_core::provider::{
    CacheEndpointIdentity, CacheIdentity, CacheIdentityFragment, ModelId,
};
use serde_json::json;

struct ShellShortcutFixture {
    home: tempfile::TempDir,
    _project: tempfile::TempDir,
    host: HostSession,
}

#[derive(Debug)]
struct ShellShortcutKeys;

impl CheckpointKeyProvider for ShellShortcutKeys {
    fn load_or_create(
        &self,
    ) -> Result<crate::checkpoint::CheckpointKey, crate::checkpoint::CheckpointProtectionError>
    {
        Ok(crate::checkpoint::CheckpointKey::new([0x52; 32]))
    }
}

impl ShellShortcutFixture {
    async fn new(persistent: bool) -> Self {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        let config_dir = home.path().join(".smith");
        std::fs::create_dir_all(&config_dir).expect("config directory");
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
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
"#,
        )
        .expect("config");
        let mut config = smith_config::resolve::resolve(
            &smith_config::resolve::ResolveRequest::new(project.path()).with_home_dir(home.path()),
        )
        .expect("resolution")
        .config;
        config.persistence.enabled.value = persistent;
        let runtime = RuntimeRequest {
            workspace: Some(Arc::new(
                smith_host::ProjectWorkspace::new(project.path()).expect("workspace"),
            )),
            approval: Some(Arc::new(agent_runtime_core::approval::DenyAll)),
            persistence_redactor: Some(DefaultRedactor::new().with_secret("TOP_SECRET_COMMAND")),
            ..RuntimeRequest::new(config, crate::factory::HostSurface::Terminal)
        };
        let host = start(
            HostSessionRequest::new(runtime, project.path())
                .checkpoint_keys(Arc::new(ShellShortcutKeys)),
        )
        .await
        .expect("host");
        Self {
            home,
            _project: project,
            host,
        }
    }

    fn path(&self) -> PathBuf {
        self.host
            .paths()
            .expect("persistent paths")
            .shell(self.host.session().id())
            .expect("shell sidecar")
    }
}

#[tokio::test]
async fn shutdown_closes_snapshot_store_before_empty_session_removal() {
    let fixture = ShellShortcutFixture::new(true).await;
    let host = &fixture.host;
    let store = host
        .snapshot_store
        .as_ref()
        .expect("snapshot store")
        .clone();
    let snapshot = host.snapshot();
    let stats = host.shutdown().await.expect("shutdown");
    assert_eq!(host.shutdown().await.expect("repeated shutdown"), stats);
    let paths = host.paths().expect("paths");
    paths
        .remove_session_files(host.session().id())
        .await
        .expect("remove");

    // This is the same store entry point used by Runtime's detached
    // parent-shutdown watcher, held until after the host has returned.
    let late_write = store.save(&snapshot).await;
    assert!(
        late_write.is_err(),
        "shutdown must reject late catalog saves"
    );
    assert!(
        !paths
            .snapshot(host.session().id())
            .expect("snapshot path")
            .exists()
    );
}

#[tokio::test]
async fn snapshot_store_close_drains_a_save_whose_caller_was_aborted() {
    let fixture = ShellShortcutFixture::new(true).await;
    let host = &fixture.host;
    // Stop real schedulers so the hook belongs to the explicit save.
    host.shutdown().await.expect("shutdown");
    let original = host.snapshot_store.as_ref().expect("snapshot store");
    let store = Arc::new(RedactingSessionStore::new(
        FileSessionStore::new(host.paths().expect("paths").clone()),
        original.redactor.clone(),
        original.reasoning.clone(),
        None,
        None,
        None,
    ));
    let pause = Arc::new(SavePause::default());
    *store.save_pause.lock().expect("pause lock") = Some(pause.clone());
    let snapshot = host.snapshot();
    let writer = tokio::spawn({
        let store = store.clone();
        async move { store.save(&snapshot).await }
    });
    pause.started.notified().await;
    writer.abort();
    assert!(writer.await.expect_err("aborted caller").is_cancelled());
    let close = store.close();
    tokio::pin!(close);
    assert!(
        futures_util::poll!(&mut close).is_pending(),
        "close must wait for the owned save"
    );
    pause.release.notify_one();
    close.await;
    let paths = host.paths().expect("paths");
    paths
        .remove_session_files(host.session().id())
        .await
        .expect("remove");
    assert!(
        !paths
            .snapshot(host.session().id())
            .expect("snapshot path")
            .exists()
    );
}

#[tokio::test]
async fn shell_shortcuts_round_trip_in_file_order() {
    let fixture = ShellShortcutFixture::new(true).await;
    let host = &fixture.host;
    let history = host.session().history();
    host.record_shell_shortcut(2, Some("call-1"), "ls", false, Some("one\ntwo"));
    host.record_shell_shortcut(0, None, "rejected", true, None);
    assert_eq!(
        host.saved_shell_shortcuts(),
        [
            SavedShellShortcut {
                schema_version: 1,
                anchor: 2,
                call: Some("call-1".to_owned()),
                command: "ls".to_owned(),
                is_error: false,
                result: Some("one\ntwo".to_owned()),
            },
            SavedShellShortcut {
                schema_version: 1,
                anchor: 0,
                call: None,
                command: "rejected".to_owned(),
                is_error: true,
                result: None,
            },
        ]
    );
    let text = std::fs::read_to_string(fixture.path()).expect("sidecar");
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert!(text.ends_with('\n'));
    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("record");
    assert!(second.get("call").is_none());
    assert!(second.get("result").is_none());
    assert_eq!(host.session().history(), history);
    host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn shell_shortcuts_ignore_truncated_invalid_and_future_records() {
    let fixture = ShellShortcutFixture::new(true).await;
    let host = &fixture.host;
    host.record_shell_shortcut(0, None, "first", false, Some("first result"));
    let mut future = host.saved_shell_shortcuts()[0].clone();
    future.schema_version = 2;
    let mut file = OpenOptions::new()
        .append(true)
        .open(fixture.path())
        .expect("sidecar");
    file.write_all(b"invalid json\n").expect("invalid record");
    append_shell_shortcut(&fixture.path(), &future).expect("future record");
    host.record_shell_shortcut(1, Some("call-2"), "second", true, Some("failed"));
    let expected = host.saved_shell_shortcuts();
    assert_eq!(expected.len(), 2);
    let final_line = serde_json::to_vec(&expected[0]).expect("complete JSON without newline");
    file.write_all(&final_line).expect("unterminated record");
    assert_eq!(host.saved_shell_shortcuts(), expected);
    file.write_all(b"\n{\"schema_version\":1,\"anchor\":")
        .expect("truncated record");
    let mut with_completed_line = expected.clone();
    with_completed_line.push(expected[0].clone());
    assert_eq!(host.saved_shell_shortcuts(), with_completed_line);
    host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn shell_shortcuts_redact_command_and_result() {
    let fixture = ShellShortcutFixture::new(true).await;
    fixture.host.record_shell_shortcut(
        0,
        Some("call-1"),
        "printf TOP_SECRET_COMMAND",
        false,
        Some("result TOP_SECRET_COMMAND"),
    );
    let records = fixture.host.saved_shell_shortcuts();
    assert_eq!(records[0].command, "printf [redacted]");
    assert_eq!(records[0].result.as_deref(), Some("result [redacted]"));
    let text = std::fs::read_to_string(fixture.path()).expect("sidecar");
    assert!(!text.contains("TOP_SECRET_COMMAND"));
    fixture.host.shutdown().await.expect("shutdown");
}

#[cfg(unix)]
#[tokio::test]
async fn shell_shortcuts_are_created_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = ShellShortcutFixture::new(true).await;
    fixture
        .host
        .record_shell_shortcut(0, None, "ls", false, None);
    assert_eq!(
        std::fs::metadata(fixture.path())
            .expect("sidecar metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn shell_shortcuts_do_not_persist_without_session_paths() {
    let fixture = ShellShortcutFixture::new(false).await;
    assert!(fixture.host.paths().is_none());
    fixture
        .host
        .record_shell_shortcut(0, None, "ls", false, Some("files"));
    assert!(fixture.host.saved_shell_shortcuts().is_empty());
    assert!(!fixture.home.path().join(".smith/sessions").exists());
    fixture.host.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn shell_shortcuts_write_failure_is_nonfatal() {
    let fixture = ShellShortcutFixture::new(true).await;
    std::fs::create_dir(fixture.path()).expect("unwritable sidecar");
    fixture
        .host
        .record_shell_shortcut(0, None, "ls", false, Some("files"));
    assert!(fixture.host.saved_shell_shortcuts().is_empty());
    fixture.host.shutdown().await.expect("shutdown");
}

fn journal_event(seq: u64, payload: RuntimeEvent) -> JournalLine {
    JournalLine::new(JournalRecord::Event {
        event: EventEnvelope::new(
            seq,
            EventId::new(format!("evt-{seq}")),
            SessionId::new("session-recovery"),
            None,
            agent_runtime_core::clock::Timestamp(seq),
            payload,
        ),
    })
}

fn cache_identity() -> CacheIdentity {
    CacheIdentity::builder(
        "provider",
        ModelId::new("model"),
        CacheEndpointIdentity::from_opaque(
            "endpoint",
            agent_runtime::registry::RegistryRevision::new("endpoint-r1"),
        ),
        agent_runtime::registry::RegistryRevision::new("adapter-r1"),
        Fingerprint::of("profile"),
    )
    .provider_key(Fingerprint::of("account"))
    .stable_prefix(vec![CacheIdentityFragment::new(
        "system",
        Fingerprint::of("system"),
    )])
    .build()
}

fn artifact_reference(
    session: &SessionId,
    id: &str,
    purpose: &str,
    media_type: &str,
) -> ArtifactRef {
    ArtifactRef {
        id: ArtifactId::new(id).expect("bounded artifact id"),
        digest: ArtifactDigest::new("sha256", "aa").expect("valid digest"),
        media_type: media_type.to_owned(),
        byte_length: 8,
        sensitivity: ArtifactSensitivity::Sensitive,
        retention: ArtifactRetention::Session,
        provenance: ArtifactProvenance::new(session.clone(), purpose),
    }
}

#[test]
fn stale_runtime_ordinary_summary_cannot_replace_a_handoff_during_persist_prepare() {
    let session = SessionId::new("session-handoff-persist");
    let identity = cache_identity();
    let slot = ResumeCapsuleSlot::new(session.clone(), agent_runtime_core::clock::Timestamp(1));
    let handoff_artifact = artifact_reference(
        &session,
        "handoff-artifact",
        crate::resume_capsule::RESUME_SUMMARY_ARTIFACT_PURPOSE,
        RESUME_SUMMARY_MEDIA_TYPE,
    );
    let runtime_state = artifact_reference(
        &session,
        "runtime-state",
        RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
        RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE,
    );
    slot.update(|capsule| {
        capsule
            .attach_runtime_summary_state_artifact(runtime_state.clone())
            .expect("prior ordinary Runtime state artifact");
        capsule.cache.prior_identity = Some(identity.clone());
        capsule
            .record_handoff_summary(
                "provider",
                "model",
                agent_runtime::registry::RegistryRevision::new("handoff-r1"),
                identity,
                agent_runtime_core::clock::Timestamp(2),
                "handoff body",
                vec![SummaryCoverage::new("canonical_events", 0, 3)],
            )
            .expect("valid handoff summary");
        capsule
            .attach_summary_artifact(handoff_artifact)
            .expect("valid handoff artifact");
    });
    let handoff = slot
        .snapshot()
        .semantic_summary
        .expect("handoff projection");

    let persistence = RuntimeSummaryPersistence {
        state_artifact: runtime_state,
        ordinary: Some(OrdinarySummaryPersistence {
            model: "summary-model".to_owned(),
            revision: agent_runtime::registry::RegistryRevision::new("ordinary-r2"),
            body: "stale ordinary body".to_owned(),
            usage: SummaryUsage::default(),
            artifact: artifact_reference(
                &session,
                "ordinary-artifact",
                RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                RESUME_SUMMARY_MEDIA_TYPE,
            ),
            coverage: vec![SummaryCoverage::new("canonical_history", 0, 2)],
        }),
    };
    slot.try_update_atomic(|capsule| {
        project_runtime_summary_persistence(
            capsule,
            persistence,
            Some("summary-provider"),
            agent_runtime_core::clock::Timestamp(3),
        )
    })
    .expect("runtime summary state reference projects atomically");

    let after = slot.snapshot();
    assert_eq!(after.semantic_summary.as_ref(), Some(&handoff));
    assert!(after.latest_summary_state_artifact.is_some());
    let (_, state) = slot
        .prepare_versioned_state(agent_runtime_core::clock::Timestamp(4))
        .expect("handoff capsule remains persistable");
    assert_eq!(
        state.value["semantic_summary"]["provenance"]["purpose"],
        "handoff_checkpoint"
    );
    assert_eq!(
        state.value["semantic_summary"]["provenance"]["provider"],
        "provider"
    );
    assert_eq!(
        state.value["semantic_summary"]["provenance"]["model"],
        "model"
    );

    let newer_runtime_state = artifact_reference(
        &session,
        "runtime-state-newer",
        RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
        RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE,
    );
    slot.try_update_atomic(|capsule| {
        project_runtime_summary_persistence(
            capsule,
            RuntimeSummaryPersistence {
                state_artifact: newer_runtime_state.clone(),
                ordinary: Some(OrdinarySummaryPersistence {
                    model: "summary-model-newer".to_owned(),
                    revision: agent_runtime::registry::RegistryRevision::new("ordinary-r3"),
                    body: "newer ordinary body".to_owned(),
                    usage: SummaryUsage::default(),
                    artifact: artifact_reference(
                        &session,
                        "ordinary-artifact-newer",
                        RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                        RESUME_SUMMARY_MEDIA_TYPE,
                    ),
                    coverage: vec![SummaryCoverage::new("canonical_history", 0, 4)],
                }),
            },
            Some("summary-provider"),
            agent_runtime_core::clock::Timestamp(5),
        )
    })
    .expect("newer ordinary Runtime state replaces the older handoff");
    let newer = slot.snapshot();
    let newer_summary = newer.semantic_summary.expect("newer ordinary projection");
    assert_eq!(
        newer_summary.provenance.purpose,
        ResumeSummaryPurpose::OrdinarySummary
    );
    assert_eq!(newer_summary.provenance.provider, "summary-provider");
    assert_eq!(newer_summary.provenance.model, "summary-model-newer");
    assert_eq!(newer_summary.provenance.cache_identity, None);
    assert_eq!(
        newer.latest_summary_state_artifact.as_ref(),
        Some(&newer_runtime_state)
    );
}

#[test]
fn stale_runtime_success_cannot_replace_a_newer_failed_idle_projection() {
    let session = SessionId::new("session-idle-failure-persist");
    let runtime_state = artifact_reference(
        &session,
        "runtime-state-before-failure",
        RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
        RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE,
    );
    let slot = ResumeCapsuleSlot::new(session.clone(), agent_runtime_core::clock::Timestamp(1));
    slot.update(|capsule| {
        capsule
            .attach_runtime_summary_state_artifact(runtime_state.clone())
            .expect("prior successful Runtime summary state");
        capsule
            .record_failed_ordinary_summary(
                "summary-provider",
                "summary-model",
                agent_runtime::registry::RegistryRevision::new("failed-r2"),
                agent_runtime_core::clock::Timestamp(3),
                vec![SummaryCoverage::new("canonical_events", 0, 4)],
            )
            .expect("failed idle projection");
    });
    let failed = slot
        .snapshot()
        .semantic_summary
        .expect("failed summary metadata");

    slot.try_update_atomic(|capsule| {
        project_runtime_summary_persistence(
            capsule,
            RuntimeSummaryPersistence {
                state_artifact: runtime_state,
                ordinary: Some(OrdinarySummaryPersistence {
                    model: "summary-model".to_owned(),
                    revision: agent_runtime::registry::RegistryRevision::new("successful-r1"),
                    body: "older successful body".to_owned(),
                    usage: SummaryUsage::default(),
                    artifact: artifact_reference(
                        &session,
                        "older-success-artifact",
                        RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
                        RESUME_SUMMARY_MEDIA_TYPE,
                    ),
                    coverage: vec![SummaryCoverage::new("canonical_history", 0, 2)],
                }),
            },
            Some("summary-provider"),
            agent_runtime_core::clock::Timestamp(4),
        )
    })
    .expect("unchanged Runtime state is recognized as stale");

    let after = slot.snapshot();
    assert_eq!(after.semantic_summary.as_ref(), Some(&failed));
    assert_eq!(
        after
            .semantic_summary
            .as_ref()
            .map(|summary| summary.provenance.outcome),
        Some(ResumeSummaryOutcome::Failed)
    );
    slot.prepare_versioned_state(agent_runtime_core::clock::Timestamp(5))
        .expect("failed metadata remains persistable");
}

#[test]
fn canonical_tool_calls_are_resolved_by_stable_id_without_exposing_arguments() {
    let history = vec![Message::assistant(vec![
        ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("call-read"),
            name: "read".to_owned(),
            arguments: json!({"path": "src/lib.rs"}),
        }),
        ContentPart::ToolCall(ToolCall {
            id: ToolCallId::new("call-shell"),
            name: "shell".to_owned(),
            arguments: json!({
                "command": "printf TOP_SECRET_COMMAND",
                "cwd": "crates/smith-cli"
            }),
        }),
    ])];

    let redactor = DefaultRedactor::new().with_secret("TOP_SECRET_COMMAND");
    assert!(
        tool_call_display_from_history(&[], &ToolCallId::new("call-shell"), &redactor).is_none(),
        "request-time lookup can race canonical history visibility"
    );
    let display =
        tool_call_display_from_history(&history, &ToolCallId::new("call-shell"), &redactor)
            .expect("matching canonical call");
    assert_eq!(
        display.invocation(),
        "Bash(printf [redacted] · cwd crates/smith-cli)"
    );
    assert!(!display.invocation().contains("TOP_SECRET_COMMAND"));
    let ContentPart::ToolCall(canonical) = &history[0].content[1] else {
        panic!("expected canonical tool call");
    };
    assert_eq!(canonical.arguments["command"], "printf TOP_SECRET_COMMAND");
    let resumed = tool_call_displays_from_history(&history, &redactor);
    assert_eq!(
        resumed
            .iter()
            .find(|(call, _)| call.as_str() == "call-shell")
            .map(|(_, display)| display),
        Some(&display),
        "completion retry and resume must converge on the same projection"
    );
    assert!(
        tool_call_display_from_history(&history, &ToolCallId::new("missing"), &redactor).is_none()
    );
}

#[test]
fn only_terminal_children_are_removed_from_ephemeral_recovery() {
    let child = ChildId::new("child-1");
    let spawned = RuntimeEvent::ChildSpawned {
        child: child.clone(),
        workspace: WorkspacePolicy::ReadOnlyView,
        max_turns: 1,
        max_tokens: None,
        deadline_ms: None,
    };
    let resolutions = [
        (
            RuntimeEvent::ChildCompleted {
                child: child.clone(),
                result: "done".to_owned(),
            },
            true,
        ),
        (
            RuntimeEvent::ChildStopped {
                child: child.clone(),
                reason: CancelReason::Shutdown,
            },
            false,
        ),
        (
            RuntimeEvent::ChildFailed {
                child: child.clone(),
                error: RuntimeError::internal("failed"),
            },
            false,
        ),
        (
            RuntimeEvent::ChildNeedsInput {
                child: child.clone(),
                child_session: SessionId::new("child-session"),
                turn: TurnId::new("turn-1"),
                call: ToolCallId::new("call-1"),
                request: InteractionRequestId::new("interaction-1"),
                question_ids: vec![QuestionId::new("question-1")],
                sensitivity: InteractionSensitivity::Sensitive,
            },
            true,
        ),
    ];

    for (index, (resolution, remains_ephemeral)) in resolutions.into_iter().enumerate() {
        let recovery = JournalRecovery {
            records: vec![
                journal_event(0, spawned.clone()),
                journal_event(u64::try_from(index).unwrap_or(0) + 1, resolution),
            ],
            truncated_tail: None,
        };
        let interruption = unresolved_ephemeral_work(&recovery);
        if remains_ephemeral {
            assert_eq!(
                interruption
                    .expect("a follow-up-capable child remains ephemeral")
                    .children
                    .as_slice(),
                std::slice::from_ref(&child)
            );
        } else {
            assert!(
                interruption.is_none(),
                "a terminal child was treated as live"
            );
        }
    }
}

#[test]
fn monitor_lifecycle_and_prior_interruption_are_reconciled_exactly_once() {
    let running = "monitor:build".to_owned();
    let stopped = "monitor:lint".to_owned();
    let recovery = JournalRecovery {
        records: vec![
            JournalLine::new(JournalRecord::MonitorStarted {
                monitor: running.clone(),
            }),
            JournalLine::new(JournalRecord::MonitorStarted {
                monitor: stopped.clone(),
            }),
            JournalLine::new(JournalRecord::MonitorStopped { monitor: stopped }),
        ],
        truncated_tail: None,
    };
    let interruption = unresolved_ephemeral_work(&recovery)
        .expect("the unresolved monitor is interrupted on recovery");
    assert_eq!(
        interruption.monitors.as_slice(),
        std::slice::from_ref(&running)
    );
    assert!(interruption.children.is_empty());

    let mut reconciled = recovery;
    reconciled
        .records
        .push(JournalLine::new(JournalRecord::EphemeralWorkInterrupted {
            interruption,
        }));
    assert!(
        unresolved_ephemeral_work(&reconciled).is_none(),
        "the persisted recovery marker must prevent duplicate interruption"
    );
}

#[test]
fn a_background_task_started_without_a_terminal_marker_is_reported_and_never_duplicated() {
    let running = "task:build".to_owned();
    let exited = "task:lint".to_owned();
    let recovery = JournalRecovery {
        records: vec![
            JournalLine::new(JournalRecord::TaskStarted {
                task: running.clone(),
            }),
            JournalLine::new(JournalRecord::TaskStarted {
                task: exited.clone(),
            }),
            JournalLine::new(JournalRecord::TaskExited { task: exited }),
        ],
        truncated_tail: None,
    };
    let interruption = unresolved_ephemeral_work(&recovery)
        .expect("a task started with no terminal marker is interrupted on recovery");
    assert_eq!(
        interruption.tasks.as_slice(),
        std::slice::from_ref(&running)
    );
    assert!(interruption.children.is_empty());
    assert!(interruption.monitors.is_empty());

    let mut reconciled = recovery;
    reconciled
        .records
        .push(JournalLine::new(JournalRecord::EphemeralWorkInterrupted {
            interruption,
        }));
    assert!(
        unresolved_ephemeral_work(&reconciled).is_none(),
        "the persisted recovery marker must prevent duplicate interruption"
    );
}
