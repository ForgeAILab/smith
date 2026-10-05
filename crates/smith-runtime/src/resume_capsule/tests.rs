use super::*;
use agent_runtime::registry::Fingerprint;
use agent_runtime_core::artifact::{
    ArtifactProvenance, ArtifactRetention, ArtifactSensitivity, ArtifactWrite,
};

use crate::artifact::SmithArtifactStore;
use crate::session::{ProjectId, SessionPaths};

fn capsule() -> ResumeCapsule {
    ResumeCapsule::new(SessionId::new("session"), Timestamp(1))
}

fn identity(label: &str) -> CacheIdentity {
    CacheIdentity::builder(
        "provider",
        agent_runtime_core::provider::ModelId::new("model"),
        agent_runtime_core::cache::CacheEndpointIdentity::from_opaque(
            "endpoint",
            RegistryRevision::new("endpoint-r1"),
        ),
        RegistryRevision::new("adapter-r1"),
        Fingerprint::of("profile"),
    )
    .provider_key(Fingerprint::of(label))
    .stable_prefix(vec![agent_runtime_core::cache::CacheIdentityFragment::new(
        "system",
        Fingerprint::of("system"),
    )])
    .build()
}

fn artifact_store(root: &std::path::Path) -> SmithArtifactStore {
    SmithArtifactStore::new(SessionPaths::new(
        root,
        &ProjectId::new("resume-artifact-project").expect("project id"),
    ))
}

#[test]
fn capsule_is_versioned_and_redaction_safe() {
    let mut capsule = capsule();
    capsule
        .record_handoff_summary(
            "provider",
            "model",
            RegistryRevision::new("summary-r1"),
            identity("a"),
            Timestamp(2),
            "PRIVATE_PROMPT_BODY should never be serialized",
            vec![SummaryCoverage::new("turns", 1, 4)],
        )
        .unwrap();
    let json = serde_json::to_string(&capsule.redacted_projection()).unwrap();
    assert_eq!(capsule.schema_version, RESUME_CAPSULE_SCHEMA_VERSION);
    assert!(!json.contains("PRIVATE_PROMPT_BODY"));
    assert!(!json.contains("summary body"));
    assert!(json.contains("handoff_checkpoint"));
}

#[test]
fn exact_validation_state_wins_over_conflicting_summary_claim() {
    let mut capsule = capsule();
    let mut state = ExactResumeState {
        watermark: 5,
        ..ExactResumeState::default()
    };
    state.validations.insert(
        "tests".to_owned(),
        ValidationProjection {
            validation: "tests".to_owned(),
            exit_status: Some(1),
            watermark: 5,
        },
    );
    capsule.commit_exact_state(state.clone(), Timestamp(5));
    let mut summary = SemanticSummary::new(
        ResumeSummaryProvenance::ordinary(
            "provider",
            "small-model",
            RegistryRevision::new("r1"),
            Timestamp(6),
        ),
        Some("tests passed"),
    )
    .unwrap();
    summary
        .push_claim(SummaryClaim::ValidationExit {
            validation: "tests".to_owned(),
            exit_status: 0,
        })
        .unwrap();
    capsule.set_summary(summary).unwrap();
    let diagnostics = capsule.summary_conflicts(RecoverySource::CanonicalSnapshot);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        capsule.exact_state.validations["tests"].exit_status,
        Some(1)
    );
}

#[test]
fn protected_watermark_wins_equal_boundary_and_journal_is_not_authority() {
    let capsule = capsule();
    let canonical = ExactStateRecord::new(
        capsule.session_id.clone(),
        7,
        ExactResumeState {
            unresolved_decisions: 2,
            ..Default::default()
        },
    );
    let protected = ExactStateRecord::new(
        capsule.session_id.clone(),
        7,
        ExactResumeState {
            unresolved_decisions: 1,
            ..Default::default()
        },
    );
    let recovered = capsule
        .recover(Some(canonical), Some(protected), Some(99))
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered.authoritative_source,
        RecoverySource::ProtectedCheckpoint
    );
    assert_eq!(recovered.capsule.exact_state.unresolved_decisions, 1);
    assert_eq!(recovered.journal_watermark, Some(99));
}

#[test]
fn recovery_rejects_foreign_sessions_and_misaligned_records() {
    let capsule = capsule();
    let foreign = ExactStateRecord::new(
        SessionId::new("other-session"),
        2,
        ExactResumeState::default(),
    );
    assert_eq!(
        capsule.recover(Some(foreign), None, None).unwrap_err(),
        ResumeCapsuleError::InvalidSerializedForm
    );

    let mut misaligned =
        ExactStateRecord::new(capsule.session_id.clone(), 3, ExactResumeState::default());
    misaligned.state.watermark = 4;
    assert_eq!(
        capsule.recover(None, Some(misaligned), None).unwrap_err(),
        ResumeCapsuleError::InvalidSerializedForm
    );
}

#[test]
fn total_serialized_capsule_size_is_bounded() {
    let mut capsule = capsule();
    capsule.exact_state.changed_files = (0..MAX_CHANGED_FILES)
        .map(|index| ChangedFileProjection {
            path: format!("{index:04}-{}", "x".repeat(MAX_METADATA_BYTES - 6)),
            additions: 0,
            deletions: 0,
            digest: Some(Fingerprint::of(index.to_string())),
        })
        .collect();
    assert_eq!(
        capsule.validate().unwrap_err(),
        ResumeCapsuleError::ProjectionLimit
    );
}

#[test]
fn persisted_handoff_identity_must_match_the_cache_baseline() {
    let mut capsule = capsule();
    capsule.cache.prior_identity = Some(identity("baseline"));
    capsule.semantic_summary = Some(
        SemanticSummary::new(
            ResumeSummaryProvenance::handoff(
                "provider",
                "model",
                RegistryRevision::new("summary-r1"),
                identity("tampered"),
                Timestamp(2),
            ),
            Some("bounded"),
        )
        .unwrap(),
    );
    assert_eq!(
        capsule.validate().unwrap_err(),
        ResumeCapsuleError::HandoffIdentityMismatch
    );
}

#[test]
fn cold_resume_makes_warmth_unknown_and_interrupts_only_live_children() {
    let mut capsule = capsule();
    let child_running = ChildId::new("running");
    let child_done = ChildId::new("done");
    capsule.exact_state.children.insert(
        child_running.clone(),
        ChildResumeProjection {
            child: child_running.clone(),
            task_digest: Some(Fingerprint::of("private task")),
            state: ChildLifecycleState::Running,
            terminal_outcome: None,
            watermark: 4,
        },
    );
    capsule.exact_state.children.insert(
        child_done.clone(),
        ChildResumeProjection {
            child: child_done.clone(),
            task_digest: None,
            state: ChildLifecycleState::Completed,
            terminal_outcome: Some(ChildTerminalOutcome {
                state: ChildLifecycleState::Completed,
                result_digest: None,
                watermark: 5,
            }),
            watermark: 5,
        },
    );
    capsule.cache.prior_identity = Some(identity("warm-before-restart"));
    capsule.cache.provider_warmth = ResumeCacheWarmth::WarmObserved;
    capsule.cache.guaranteed_until = Some(Timestamp(99));
    capsule.cache.last_meaningful_activity_at = Some(Timestamp(42));
    capsule.cache.idle_compaction_interval_id = Some("root-turn:7".to_owned());
    capsule.cache.idle_compaction_attempted = true;
    let cold = capsule.cold_resume();
    assert_eq!(cold.provider_warmth, ResumeCacheWarmth::Unknown);
    assert!(!cold.prewarm_requested);
    assert_eq!(cold.interrupted_children, vec![child_running]);
    assert_eq!(
        cold.capsule.cache.provider_warmth,
        ResumeCacheWarmth::Unknown
    );
    assert_eq!(cold.capsule.cache.guaranteed_until, None);
    assert_eq!(
        cold.capsule.cache.prior_identity,
        capsule.cache.prior_identity
    );
    assert_eq!(
        cold.capsule.cache.last_meaningful_activity_at,
        Some(Timestamp(42))
    );
    assert_eq!(
        cold.capsule.cache.idle_compaction_interval_id.as_deref(),
        Some("root-turn:7")
    );
    assert!(cold.capsule.cache.idle_compaction_attempted);
    assert_eq!(
        cold.capsule.exact_state.children[&child_done].state,
        ChildLifecycleState::Completed
    );
}

#[test]
fn handoff_and_ordinary_summary_routes_remain_distinct() {
    let id = identity("parent");
    let mut capsule = capsule();
    capsule
        .record_handoff_summary(
            "provider",
            "model",
            RegistryRevision::new("handoff-r1"),
            id.clone(),
            Timestamp(2),
            "handoff",
            vec![],
        )
        .unwrap();
    assert_eq!(
        capsule
            .semantic_summary
            .as_ref()
            .unwrap()
            .provenance
            .purpose,
        ResumeSummaryPurpose::HandoffCheckpoint
    );
    assert_eq!(
        capsule
            .semantic_summary
            .as_ref()
            .unwrap()
            .provenance
            .cache_identity,
        Some(id)
    );
    capsule
        .record_ordinary_summary(
            "other-provider",
            "small-model",
            RegistryRevision::new("summary-r2"),
            Timestamp(3),
            "ordinary",
            vec![],
        )
        .unwrap();
    let summary = capsule.semantic_summary.as_ref().unwrap();
    assert_eq!(
        summary.provenance.purpose,
        ResumeSummaryPurpose::OrdinarySummary
    );
    assert_eq!(summary.provenance.cache_identity, None);
}

#[test]
fn a_real_parent_identity_change_retires_only_identity_bound_handoffs() {
    let mut capsule = capsule();
    let first = identity("first");
    capsule.cache.prior_identity = Some(first.clone());
    capsule
        .record_handoff_summary(
            "provider",
            "model",
            RegistryRevision::new("handoff-r1"),
            first.clone(),
            Timestamp(2),
            "handoff",
            vec![],
        )
        .unwrap();

    assert!(!capsule.retire_handoff_if_identity_changed(Some(&first)));
    assert!(capsule.semantic_summary.is_some());
    assert!(capsule.retire_handoff_if_identity_changed(Some(&identity("second"))));
    assert!(capsule.semantic_summary.is_none());

    capsule
        .record_ordinary_summary(
            "summary-provider",
            "summary-model",
            RegistryRevision::new("ordinary-r2"),
            Timestamp(3),
            "ordinary",
            vec![],
        )
        .unwrap();
    assert!(!capsule.retire_handoff_if_identity_changed(None));
    assert_eq!(
        capsule
            .semantic_summary
            .as_ref()
            .map(|summary| summary.provenance.purpose),
        Some(ResumeSummaryPurpose::OrdinarySummary)
    );
}

#[test]
fn synthetic_turns_have_no_canonical_recent_turn_slot() {
    let mut capsule = capsule();
    capsule
        .push_recent_turn(RecentTurnProjection {
            turn: TurnId::new("real"),
            role: RecentTurnRole::Assistant,
            content_digest: Some(Fingerprint::of("real")),
        })
        .unwrap();
    let json = serde_json::to_string(&capsule.redacted_projection()).unwrap();
    assert!(json.contains("real"));
    assert!(!json.contains("ping"));
    assert!(!json.contains("pong"));
}

#[test]
fn v0_capsule_migrates_to_current_schema() {
    let value = serde_json::json!({
        "session_id": "session",
        "created_at": 1,
    });
    let migrated = ResumeCapsule::from_json_value(value).unwrap();
    assert_eq!(migrated.schema_version, RESUME_CAPSULE_SCHEMA_VERSION);
    assert_eq!(migrated.session_id, SessionId::new("session"));
}

#[test]
fn summary_body_is_bounded() {
    let provenance = ResumeSummaryProvenance::ordinary(
        "provider",
        "model",
        RegistryRevision::new("r1"),
        Timestamp(1),
    );
    let body = "x".repeat(MAX_SUMMARY_BYTES + 1);
    assert_eq!(
        SemanticSummary::new(provenance, Some(body)).unwrap_err(),
        ResumeCapsuleError::SummaryTooLarge
    );
}

#[test]
fn persistence_markers_commit_only_after_the_prepared_save_succeeds() {
    let slot = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(1));
    slot.update(|capsule| capsule.exact_state.watermark = 7);

    let (prepared, persisted) = slot
        .prepare_versioned_state(Timestamp(9))
        .expect("prepare a bounded capsule write");
    assert_eq!(prepared.last_persisted_watermark, 7);
    assert_eq!(prepared.last_persisted_at, Some(Timestamp(9)));
    assert_eq!(persisted.value["last_persisted_watermark"], 7);
    assert_eq!(persisted.value["last_persisted_at"], 9);

    // A failed store must not make the live projection claim that this
    // watermark reached durable storage.
    let live = slot.snapshot();
    assert_eq!(live.last_persisted_watermark, 0);
    assert_eq!(live.last_persisted_at, None);

    assert!(slot.commit_persisted(&prepared));
    let committed = slot.snapshot();
    assert_eq!(committed.last_persisted_watermark, 7);
    assert_eq!(committed.last_persisted_at, Some(Timestamp(9)));
    assert!(slot.versioned_state().is_ok());
}

#[test]
fn an_older_save_cannot_mark_a_newer_capsule_as_durable() {
    let slot = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(1));
    slot.update(|capsule| capsule.exact_state.watermark = 7);
    let (prepared, _) = slot
        .prepare_versioned_state(Timestamp(9))
        .expect("prepare the older write");
    slot.update(|capsule| capsule.exact_state.watermark = 8);

    assert!(!slot.commit_persisted(&prepared));
    assert_eq!(slot.snapshot().last_persisted_watermark, 0);
}

#[test]
fn fallible_slot_updates_publish_all_or_nothing() {
    let slot = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(1));
    let before = slot.snapshot();

    let result = slot.try_update_atomic(|candidate| {
        candidate.cache.idle_compaction_attempted = true;
        Err(ResumeCapsuleError::InvalidSerializedForm)
    });

    assert_eq!(
        result.unwrap_err(),
        ResumeCapsuleError::InvalidSerializedForm
    );
    assert_eq!(slot.snapshot(), before);
}

#[tokio::test]
async fn ordinary_summary_artifact_restores_without_serializing_the_body() {
    let root = tempfile::tempdir().expect("artifact root");
    let store = artifact_store(root.path());
    let session = SessionId::new("session");
    let body = "PRIVATE_ORDINARY_SUMMARY_BODY";
    let reference = store
        .put(ArtifactWrite {
            bytes: body.as_bytes().to_vec(),
            media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(
                session.clone(),
                RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
            ),
            idempotency_key: "ordinary-summary-r1".to_owned(),
        })
        .await
        .expect("write summary artifact");
    let slot = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
    slot.update(|capsule| {
        capsule.record_ordinary_summary(
            "provider",
            "summary-model",
            RegistryRevision::new("summary-r1"),
            Timestamp(2),
            body,
            vec![SummaryCoverage::new("canonical_events", 0, 4)],
        )?;
        capsule.attach_summary_artifact(reference)
    })
    .expect("attach summary artifact");

    let persisted = slot.versioned_state().expect("versioned capsule");
    let serialized = serde_json::to_string(&persisted.value).expect("capsule JSON");
    assert!(!serialized.contains(body));
    assert!(serialized.contains(RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE));

    let restored = ResumeCapsuleSlot::new(session, Timestamp(9));
    assert!(
        restored
            .restore_versioned_state(&persisted, RecoverySource::CanonicalSnapshot)
            .expect("restore capsule")
    );
    assert!(
        restore_summary_artifact(&restored, &store)
            .await
            .expect("restore summary body")
    );
    let snapshot = restored.snapshot();
    let summary = snapshot.semantic_summary.expect("summary metadata");
    assert_eq!(
        summary.provenance.purpose,
        ResumeSummaryPurpose::OrdinarySummary
    );
    assert_eq!(
        summary.body.as_ref().map(ProtectedSummaryText::as_str),
        Some(body)
    );
}

#[tokio::test]
async fn runtime_summary_state_artifact_round_trips_as_sensitive_extension_state() {
    let root = tempfile::tempdir().expect("artifact root");
    let store = artifact_store(root.path());
    let session = SessionId::new("session");
    let state = VersionedSessionState::new(
        RegistryRevision::new("harness.semantic-summary:model-r1"),
        serde_json::json!({
            "purpose": "context.semantic_summary",
            "summary": "PRIVATE_RUNTIME_SUMMARY"
        }),
    );
    let bytes = serde_json::to_vec(&state).expect("serialize runtime state");
    let reference = store
        .put(ArtifactWrite {
            bytes,
            media_type: RESUME_RUNTIME_SUMMARY_STATE_MEDIA_TYPE.to_owned(),
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(
                session.clone(),
                RESUME_RUNTIME_SUMMARY_STATE_ARTIFACT_PURPOSE,
            ),
            idempotency_key: "runtime-summary-state-r1".to_owned(),
        })
        .await
        .expect("write runtime summary state");
    let mut public_reference = reference.clone();
    public_reference.sensitivity = ArtifactSensitivity::Public;
    let rejected = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
    assert_eq!(
        rejected
            .update(|capsule| { capsule.attach_runtime_summary_state_artifact(public_reference) })
            .unwrap_err(),
        ResumeCapsuleError::InvalidSerializedForm,
        "a public reference must never authorize reading protected Runtime state"
    );
    assert!(rejected.snapshot().latest_summary_state_artifact.is_none());

    let slot = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
    slot.update(|capsule| capsule.attach_runtime_summary_state_artifact(reference))
        .expect("attach runtime state reference");
    let persisted = slot.versioned_state().expect("versioned capsule");
    let restored = ResumeCapsuleSlot::new(session, Timestamp(2));
    restored
        .restore_versioned_state(&persisted, RecoverySource::CanonicalSnapshot)
        .expect("restore capsule");
    let recovered = restore_runtime_summary_state(&restored, &store)
        .await
        .expect("read runtime summary state")
        .expect("runtime summary state exists");
    assert_eq!(recovered.sensitivity, SessionStateSensitivity::Sensitive);
    assert_eq!(recovered.value["purpose"], "context.semantic_summary");
    assert_eq!(recovered.value["summary"], "PRIVATE_RUNTIME_SUMMARY");
}

#[tokio::test]
async fn malformed_summary_artifact_never_becomes_live_text() {
    let root = tempfile::tempdir().expect("artifact root");
    let store = artifact_store(root.path());
    let session = SessionId::new("session");
    let reference = store
        .put(ArtifactWrite {
            bytes: vec![0xff, 0xfe],
            media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(
                session.clone(),
                RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
            ),
            idempotency_key: "invalid-utf8-summary".to_owned(),
        })
        .await
        .expect("write invalid UTF-8 artifact");
    let slot = ResumeCapsuleSlot::new(session, Timestamp(1));
    slot.update(|capsule| {
        capsule.record_ordinary_summary(
            "provider",
            "summary-model",
            RegistryRevision::new("summary-r1"),
            Timestamp(2),
            "live body discarded before simulated restart",
            vec![],
        )?;
        capsule.attach_summary_artifact(reference)
    })
    .expect("attach artifact metadata");
    let persisted = slot.versioned_state().expect("versioned capsule");
    let restored = ResumeCapsuleSlot::new(SessionId::new("session"), Timestamp(3));
    restored
        .restore_versioned_state(&persisted, RecoverySource::ProtectedCheckpoint)
        .expect("restore metadata");

    assert!(
        !restore_summary_artifact(&restored, &store)
            .await
            .expect("invalid UTF-8 is an optional summary failure")
    );
    let snapshot = restored.snapshot();
    let summary = snapshot.semantic_summary.expect("summary metadata");
    assert_eq!(summary.provenance.outcome, ResumeSummaryOutcome::Missing);
    assert!(summary.provenance.summary_artifact.is_none());
    assert!(summary.body.is_none());
}

#[tokio::test]
async fn missing_summary_artifact_degrades_without_losing_exact_state() {
    let source_root = tempfile::tempdir().expect("source artifact root");
    let missing_root = tempfile::tempdir().expect("missing artifact root");
    let source = artifact_store(source_root.path());
    let missing = artifact_store(missing_root.path());
    let session = SessionId::new("session");
    let reference = source
        .put(ArtifactWrite {
            bytes: b"summary body".to_vec(),
            media_type: RESUME_SUMMARY_MEDIA_TYPE.to_owned(),
            sensitivity: ArtifactSensitivity::Sensitive,
            retention: ArtifactRetention::Session,
            provenance: ArtifactProvenance::new(
                session.clone(),
                RESUME_IDLE_SUMMARY_ARTIFACT_PURPOSE,
            ),
            idempotency_key: "missing-summary-r1".to_owned(),
        })
        .await
        .expect("write summary artifact");
    let slot = ResumeCapsuleSlot::new(session.clone(), Timestamp(1));
    slot.update(|capsule| {
        capsule.record_ordinary_summary(
            "provider",
            "summary-model",
            RegistryRevision::new("summary-r1"),
            Timestamp(2),
            "summary body",
            vec![],
        )?;
        capsule.attach_summary_artifact(reference)?;
        let state = ExactResumeState {
            watermark: 7,
            unresolved_decisions: 2,
            ..ExactResumeState::default()
        };
        assert!(capsule.commit_exact_state(state, Timestamp(7)));
        Ok::<_, ResumeCapsuleError>(())
    })
    .expect("attach summary and exact state");

    let persisted = slot.versioned_state().expect("versioned capsule");
    let restored = ResumeCapsuleSlot::new(session, Timestamp(3));
    restored
        .restore_versioned_state(&persisted, RecoverySource::ProtectedCheckpoint)
        .expect("restore metadata");
    let exact_before = restored.snapshot().exact_state.clone();

    assert!(
        !restore_summary_artifact(&restored, &missing)
            .await
            .expect("missing artifact is an optional summary failure")
    );
    let snapshot = restored.snapshot();
    assert_eq!(snapshot.exact_state, exact_before);
    let summary = snapshot.semantic_summary.expect("summary metadata");
    assert_eq!(summary.provenance.outcome, ResumeSummaryOutcome::Missing);
    assert!(summary.provenance.summary_artifact.is_none());
    assert!(summary.body.is_none());
}
