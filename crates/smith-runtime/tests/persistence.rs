//! What survives a restart, and what a crash is allowed to cost.
//!
//! The unit tests in `session` and `journal` cover path safety and redaction
//! rules in isolation. These exercise the two contracts the way a running Smith
//! does: a snapshot written while something else is reading it, a journal
//! appended to from several tasks at once, a file left half-written by a
//! process that died mid-record.

use std::collections::BTreeMap;
use std::sync::Arc;

use agent_runtime::registry::{Fingerprint, RegistryRevision};
use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::content::Message;
use agent_runtime_core::error::ErrorKind;
use agent_runtime_core::event::{EventEnvelope, RuntimeEvent};
use agent_runtime_core::ids::{AttemptId, EventId, RequestId, SessionId, ToolCallId, TurnId};
use agent_runtime_core::manifest::{CapabilityResolution, ModelResolution, RunManifest};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::provider::ModelId;
use agent_runtime_core::store::{
    SessionIdentityState, SessionSnapshot, SessionStore, TurnManifest, VersionedSessionState,
};
use agent_runtime_core::usage::{
    CounterKind, Provenance, UsageDelta, UsageLedger, UsageRecord, UsageSource,
};
use smith_runtime::journal::{
    DefaultRedactor, EventJournal, JournalConfig, JournalRecord, read_journal,
};
use smith_runtime::session::{FileSessionStore, ProjectId, SessionPaths};

/// A store rooted in a temporary directory. The `TempDir` is returned because
/// dropping it deletes everything the test just wrote.
fn store() -> (tempfile::TempDir, FileSessionStore) {
    let directory = tempfile::tempdir().expect("a temp dir");
    let project = ProjectId::new("demo-project").expect("a project id");
    let paths = SessionPaths::new(directory.path(), &project);
    (directory, FileSessionStore::new(paths))
}

fn manifest(model: &str) -> RunManifest {
    RunManifest::new(
        Fingerprint::of("snapshot"),
        Fingerprint::of("view"),
        ModelResolution::new(
            "acme",
            ModelId::new(model),
            Fingerprint::of("profile"),
            BTreeMap::new(),
        ),
        CapabilityResolution::new(RegistryRevision::new("resolver-1")),
        Fingerprint::of("context"),
        Fingerprint::of("cache-plan"),
    )
}

/// A snapshot with every field a resume depends on set to a non-default value,
/// so a round-trip that silently drops one of them fails.
fn populated_snapshot(id: &SessionId) -> SessionSnapshot {
    let mut usage = UsageLedger::new();
    usage.record(UsageRecord {
        source: UsageSource::ProviderAttempt,
        provenance: Provenance {
            attempt: Some("a-1".into()),
            failed: true,
            ..Default::default()
        },
        delta: UsageDelta::new().with(CounterKind::InputUncached, 1_200),
    });
    usage.record(UsageRecord {
        source: UsageSource::ToolLoop,
        provenance: Provenance {
            tool_call: Some(ToolCallId::new("call-7")),
            ..Default::default()
        },
        delta: UsageDelta::new()
            .with(CounterKind::InputCached, 900)
            .with(CounterKind::Output, 64)
            .with(CounterKind::Reasoning, 32),
    });

    SessionSnapshot {
        id: id.clone(),
        history: vec![
            Message::system("you are smith"),
            Message::user("explain the retry policy"),
        ],
        usage,
        identity: SessionIdentityState {
            turn: 4,
            request: 9,
            attempt: 11,
            event: 137,
            tool_call: 6,
            steer: 0,
            event_seq: 138,
        },
        manifests: vec![
            TurnManifest::new(TurnId::new("turn-1"), manifest("acme-small")),
            TurnManifest::new(TurnId::new("turn-2"), manifest("acme-large")),
        ],
        extension_state: BTreeMap::new(),
        updated: Timestamp(1_700_000_000_000),
    }
}

#[tokio::test]
async fn a_saved_snapshot_round_trips_with_identity_counters_usage_and_manifests() {
    let (_directory, store) = store();
    let id = SessionId::new("s-round-trip");
    let original = populated_snapshot(&id);

    store.save(&original).await.expect("the snapshot saves");
    let loaded = store
        .load(&id)
        .await
        .expect("the snapshot loads")
        .expect("a snapshot exists");

    assert_eq!(loaded, original);
    // Spelled out as well as compared, because equality on the whole struct
    // would still pass if a future field defaulted on both sides.
    assert_eq!(loaded.identity.event_seq, 138);
    assert_eq!(loaded.identity.attempt, 11);
    assert_eq!(loaded.usage.records().len(), 2);
    assert!(loaded.usage.records()[0].provenance.failed);
    assert_eq!(loaded.usage.total().get(CounterKind::InputCached), 900);
    assert_eq!(loaded.manifests.len(), 2);
    assert_eq!(loaded.manifests[1].turn, TurnId::new("turn-2"));
    assert_eq!(
        loaded.manifests[1].manifest.model.model,
        ModelId::new("acme-large")
    );
    assert!(loaded.extension_state.is_empty());
}

#[tokio::test]
async fn ordinary_json_omits_sensitive_extension_state_but_preserves_explicitly_safe_state() {
    let (_directory, store) = store();
    let id = SessionId::new("s-extension-state");
    let secret = "private-memory-value-27cf";
    let mut original = populated_snapshot(&id);
    original.extension_state.insert(
        "smith.todo".into(),
        VersionedSessionState::new(
            RegistryRevision::new("todo-state-1"),
            serde_json::json!({"completed": 2}),
        )
        .redaction_safe(),
    );
    original.extension_state.insert(
        "smith.memory".into(),
        VersionedSessionState::new(
            RegistryRevision::new("memory-state-1"),
            serde_json::json!({"content": secret}),
        ),
    );

    store.save(&original).await.expect("the snapshot saves");

    let path = store.paths().snapshot(&id).expect("a snapshot path");
    let bytes = tokio::fs::read(path).await.expect("snapshot bytes");
    let text = String::from_utf8(bytes).expect("snapshot JSON");
    assert!(!text.contains(secret), "{text}");
    assert!(!text.contains("smith.memory"), "{text}");
    assert!(text.contains("smith.todo"), "{text}");

    let loaded = store
        .load(&id)
        .await
        .expect("the snapshot loads")
        .expect("a snapshot exists");
    assert_eq!(
        loaded
            .extension_state
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["smith.todo"]
    );
    assert_eq!(
        loaded.extension_state["smith.todo"].value,
        serde_json::json!({"completed": 2})
    );
}

#[tokio::test]
async fn ordinary_json_load_does_not_reintroduce_legacy_sensitive_extension_state() {
    let (_directory, store) = store();
    let id = SessionId::new("s-legacy-sensitive-state");
    store
        .save(&populated_snapshot(&id))
        .await
        .expect("a seed snapshot");
    let path = store.paths().snapshot(&id).expect("a snapshot path");
    let bytes = tokio::fs::read(&path).await.expect("snapshot bytes");
    let mut stored: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot JSON");
    stored["snapshot"]["extension_state"] = serde_json::json!({
        "legacy.memory": {
            "revision": "memory-state-0",
            "sensitivity": "sensitive",
            "value": {"content": "legacy-private-memory-a41e"}
        },
        "legacy.todo": {
            "revision": "todo-state-0",
            "sensitivity": "redaction_safe",
            "value": {"pending": 1}
        }
    });
    tokio::fs::write(
        &path,
        serde_json::to_vec(&stored).expect("rewritten snapshot JSON"),
    )
    .await
    .expect("a legacy record");

    let loaded = store
        .load(&id)
        .await
        .expect("the snapshot loads")
        .expect("a snapshot exists");
    assert!(!loaded.extension_state.contains_key("legacy.memory"));
    assert_eq!(
        loaded.extension_state["legacy.todo"].value,
        serde_json::json!({"pending": 1})
    );
}

#[tokio::test]
async fn session_snapshot_v1_compatibility_fixture_is_stable() {
    let (_directory, store) = store();
    let id = SessionId::new("session-fixture");
    store
        .save(&populated_snapshot(&id))
        .await
        .expect("the snapshot saves");
    let path = store.paths().snapshot(&id).expect("a snapshot path");
    let actual: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(path).await.expect("snapshot fixture bytes"))
            .expect("serialized snapshot JSON");
    let expected: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-snapshot-v1.json"))
            .expect("valid compatibility fixture");

    assert_eq!(actual, expected);
}

#[tokio::test]
async fn a_session_that_was_never_saved_loads_as_absent() {
    let (_directory, store) = store();
    let loaded = store
        .load(&SessionId::new("s-missing"))
        .await
        .expect("a missing session is not an error");
    assert!(loaded.is_none());
}

#[tokio::test]
async fn a_snapshot_from_an_unknown_schema_version_is_refused_rather_than_partially_parsed() {
    let (_directory, store) = store();
    let id = SessionId::new("s-future");
    store.paths().ensure_directory().await.expect("a directory");
    let path = store.paths().snapshot(&id).expect("a path");
    // A plausible future record: the version moved and the payload gained a
    // field this build knows nothing about.
    tokio::fs::write(
        &path,
        br#"{"schema_version":7,"snapshot":{"id":"s-future","history":[],"updated":1,"compaction":{}}}"#,
    )
    .await
    .expect("a written file");

    let error = store.load(&id).await.expect_err("an error");
    assert_eq!(error.kind, ErrorKind::Serialization);
    assert!(
        error.message.contains("schema version 7"),
        "the error names the version it found: {}",
        error.message
    );
    assert!(
        error.message.contains("version 1"),
        "the error names the version it supports: {}",
        error.message
    );
}

/// A snapshot big enough that a non-atomic write would leave a torn file wide
/// open to a concurrent reader: it cannot reach the disk in a single page.
fn bulky(id: &SessionId, generation: usize) -> SessionSnapshot {
    let mut snapshot = populated_snapshot(id);
    snapshot.history = (0..400)
        .map(|line| Message::user(format!("{generation}:{line}:{}", "x".repeat(256))))
        .collect();
    snapshot
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_concurrent_reader_never_observes_a_partially_written_snapshot() {
    let (directory, store) = store();
    let store = Arc::new(store);
    let id = SessionId::new("s-atomic");

    store.save(&bulky(&id, 0)).await.expect("a seed snapshot");

    let writer = tokio::spawn({
        let store = Arc::clone(&store);
        let id = id.clone();
        async move {
            for generation in 1..40 {
                store.save(&bulky(&id, generation)).await.expect("a save");
            }
        }
    });
    let reader = tokio::spawn({
        let store = Arc::clone(&store);
        let id = id.clone();
        async move {
            for _ in 0..400 {
                let loaded = store
                    .load(&id)
                    .await
                    .expect("a snapshot is always readable");
                assert!(loaded.is_some(), "the snapshot never disappears");
                tokio::task::yield_now().await;
            }
        }
    });
    writer.await.expect("the writer finishes");
    reader.await.expect("the reader finishes");

    // The temporary files the writes went through are all gone.
    let session_directory = directory.path().join("sessions/demo-project");
    let mut entries = tokio::fs::read_dir(&session_directory)
        .await
        .expect("a readable directory");
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.expect("an entry") {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    assert_eq!(names, vec!["s-atomic.snapshot.json".to_owned()]);
}

#[tokio::test]
async fn listing_enumerates_saved_sessions_most_recently_updated_first() {
    let (_directory, store) = store();
    for (id, updated) in [("s-old", 10u64), ("s-newest", 300), ("s-middle", 200)] {
        let mut snapshot = populated_snapshot(&SessionId::new(id));
        snapshot.updated = Timestamp(updated);
        store.save(&snapshot).await.expect("a save");
    }

    let listed = store.list().await.expect("a listing");
    let ids: Vec<&str> = listed.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(ids, ["s-newest", "s-middle", "s-old"]);
    assert_eq!(listed[0].updated, Some(Timestamp(300)));
    assert!(listed.iter().all(|entry| entry.schema_version == 1));
    assert_eq!(listed[0].turn_count, Some(4));
    assert_eq!(listed[0].provider.as_deref(), Some("acme"));
    assert_eq!(listed[0].model.as_deref(), Some("acme-large"));
    assert_eq!(
        listed[0].user_preview.as_deref(),
        Some("explain the retry policy")
    );
}

#[tokio::test]
async fn older_snapshot_without_listing_metadata_remains_selectable_with_unknown_fields() {
    let (_directory, store) = store();
    let id = SessionId::new("s-before-listing-metadata");
    store
        .save(&populated_snapshot(&id))
        .await
        .expect("a snapshot");
    let path = store.paths().snapshot(&id).expect("snapshot path");
    let bytes = tokio::fs::read(&path).await.expect("snapshot bytes");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot JSON");
    value.as_object_mut().expect("envelope").remove("listing");
    tokio::fs::write(&path, serde_json::to_vec(&value).expect("older JSON"))
        .await
        .expect("rewritten older snapshot");

    let listed = store.list().await.expect("a listing");
    let [entry] = listed.as_slice() else {
        panic!("expected one listing: {listed:?}");
    };
    assert_eq!(entry.id, id);
    assert!(entry.updated.is_some());
    assert_eq!(entry.turn_count, None);
    assert_eq!(entry.provider, None);
    assert_eq!(entry.model, None);
    assert_eq!(entry.user_preview, None);
}

/// One envelope per sequence number, with a turn so markers have something to
/// attribute a gap to.
fn envelope(seq: u64, payload: RuntimeEvent) -> EventEnvelope {
    EventEnvelope::new(
        seq,
        EventId::new(format!("e-{seq}")),
        SessionId::new("s-journal"),
        Some(TurnId::new("turn-1")),
        Timestamp(seq),
        payload,
    )
}

fn text(seq: u64, body: &str) -> EventEnvelope {
    envelope(
        seq,
        RuntimeEvent::TextDelta {
            request: RequestId::new("request-journal"),
            attempt: AttemptId::new("attempt-journal"),
            text: body.to_owned(),
        },
    )
}

/// A journal in a temporary directory, opened through the session layout so
/// the test exercises the same path resolution production uses.
async fn open_journal(
    directory: &tempfile::TempDir,
    config: JournalConfig,
    redactor: DefaultRedactor,
) -> (std::path::PathBuf, EventJournal) {
    let project = ProjectId::new("demo-project").expect("a project id");
    let paths = SessionPaths::new(directory.path(), &project);
    let id = SessionId::new("s-journal");
    let path = paths.journal(&id).expect("a path");
    let journal = EventJournal::for_session(&paths, &id, config, Arc::new(redactor))
        .await
        .expect("an open journal");
    (path, journal)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn journal_lines_stay_whole_under_concurrent_appends() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) = open_journal(
        &directory,
        JournalConfig {
            queue_capacity: 8192,
            ..JournalConfig::default()
        },
        DefaultRedactor::new(),
    )
    .await;
    let journal = Arc::new(journal);

    let mut workers = Vec::new();
    for worker in 0..8u64 {
        let observer: Arc<dyn EventObserver> = Arc::clone(&journal) as Arc<dyn EventObserver>;
        workers.push(tokio::spawn(async move {
            for index in 0..50u64 {
                observer.observe(&text(worker * 50 + index, &"payload ".repeat(24)));
            }
        }));
    }
    for worker in workers {
        worker.await.expect("a worker finishes");
    }

    let stats = journal.shutdown().await.expect("a clean shutdown");
    assert_eq!(stats.written, 400);
    assert_eq!(stats.dropped, 0);

    // Every line is independently parseable and the file ends on a record
    // boundary — the two things interleaved writes would destroy.
    let raw = tokio::fs::read_to_string(&path).await.expect("a journal");
    assert!(raw.ends_with('\n'));
    for line in raw.lines() {
        serde_json::from_str::<serde_json::Value>(line).expect("a whole line");
    }

    let recovery = read_journal(&path).await.expect("a readable journal");
    assert!(recovery.truncated_tail.is_none());
    let mut sequences: Vec<u64> = recovery.events().iter().map(|event| event.seq).collect();
    sequences.sort_unstable();
    assert_eq!(sequences, (0..400).collect::<Vec<u64>>());
}

#[tokio::test]
async fn an_oversized_record_becomes_a_marker_instead_of_a_truncated_line() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) = open_journal(
        &directory,
        JournalConfig {
            queue_capacity: 32,
            max_record_bytes: 512,
        },
        DefaultRedactor::new(),
    )
    .await;

    let huge = "z".repeat(8_000);
    journal.observe(&text(0, &huge));
    journal.observe(&text(1, "small enough"));
    let stats = journal.shutdown().await.expect("a clean shutdown");
    assert_eq!(stats.oversized, 1);
    assert_eq!(stats.written, 1);

    let recovery = read_journal(&path).await.expect("a readable journal");
    assert_eq!(recovery.records.len(), 2);
    match &recovery.records[0].record {
        JournalRecord::Oversized {
            seq,
            turn,
            event,
            bytes,
            ..
        } => {
            assert_eq!(*seq, 0);
            assert_eq!(turn.as_ref(), Some(&TurnId::new("turn-1")));
            assert_eq!(event, "text_delta");
            assert!(*bytes > 8_000, "the marker reports the real size: {bytes}");
        }
        other => panic!("expected an oversized marker, got {other:?}"),
    }
    assert_eq!(recovery.events().len(), 1);

    // No fragment of the oversized payload was written, whole or partial.
    let raw = tokio::fs::read_to_string(&path).await.expect("a journal");
    assert!(!raw.contains(&"z".repeat(64)));
}

#[tokio::test]
async fn a_registered_secret_never_reaches_the_journal() {
    const SECRET: &str = "sk-live-9f8e7d6c5b4a3928";

    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) = open_journal(
        &directory,
        JournalConfig::default(),
        DefaultRedactor::new().with_secret(SECRET),
    )
    .await;

    // Two different routes a credential can take into the journal: echoed back
    // in model output, and carried in a tool call's arguments.
    journal.observe(&text(0, &format!("authenticate with {SECRET} first")));
    journal.observe(&envelope(
        1,
        RuntimeEvent::ToolCallRequested {
            call: ToolCallId::new("call-1"),
            name: "shell".to_owned(),
            argument_keys: vec![
                "api_key".to_owned(),
                "command".to_owned(),
                "max_tokens".to_owned(),
            ],
            argument_fingerprint: Fingerprint::of("tool arguments"),
            arguments: Some(serde_json::json!({
                "command": "curl https://api.example.com",
                "api_key": SECRET,
                "max_tokens": 512,
            })),
        },
    ));
    journal.shutdown().await.expect("a clean shutdown");

    let raw = tokio::fs::read_to_string(&path).await.expect("a journal");
    assert!(
        !raw.contains(SECRET),
        "the journal still contains the secret"
    );
    assert!(raw.contains("[redacted]"));

    // Redaction replaced values without damaging the records around them.
    let recovery = read_journal(&path).await.expect("a readable journal");
    assert_eq!(recovery.events().len(), 2);
    let RuntimeEvent::ToolCallRequested { arguments, .. } = &recovery.events()[1].payload else {
        panic!("expected the tool call to survive redaction");
    };
    let arguments = arguments.as_ref().expect("raw test arguments survive");
    assert_eq!(arguments["api_key"], "[redacted]");
    assert_eq!(arguments["command"], "curl https://api.example.com");
    assert_eq!(arguments["max_tokens"], 512);
}

#[tokio::test]
async fn an_incomplete_final_record_is_truncated_on_read_and_reported() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) =
        open_journal(&directory, JournalConfig::default(), DefaultRedactor::new()).await;
    journal.observe(&text(0, "first"));
    journal.observe(&text(1, "second"));
    journal.shutdown().await.expect("a clean shutdown");

    // A crash mid-write: the record's bytes reach the file in order, so what
    // is missing is the tail of the object and its newline.
    let complete = tokio::fs::read_to_string(&path).await.expect("a journal");
    let partial = r#"{"schema_version":1,"record":"event","event":{"schema_version":2,"seq":2"#;
    tokio::fs::write(&path, format!("{complete}{partial}"))
        .await
        .expect("a crashed journal");

    let recovery = read_journal(&path).await.expect("a readable journal");
    assert_eq!(recovery.records.len(), 2, "prior records are preserved");
    assert_eq!(recovery.events()[1].seq, 1);
    let tail = recovery
        .truncated_tail
        .expect("the incomplete record is reported, not silently dropped");
    assert_eq!(tail.offset, complete.len() as u64);
    assert_eq!(tail.bytes, partial.len());
}

#[tokio::test]
async fn reopening_a_crashed_journal_repairs_the_tail_before_appending() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) =
        open_journal(&directory, JournalConfig::default(), DefaultRedactor::new()).await;
    journal.observe(&text(0, "before the crash"));
    journal.shutdown().await.expect("a clean shutdown");

    let complete = tokio::fs::read_to_string(&path).await.expect("a journal");
    tokio::fs::write(&path, format!("{complete}{{\"schema_version\":1,\"rec"))
        .await
        .expect("a crashed journal");

    let (_, reopened) =
        open_journal(&directory, JournalConfig::default(), DefaultRedactor::new()).await;
    let recovered = reopened
        .recovered_tail()
        .expect("the repair is surfaced, not performed silently");
    assert_eq!(recovered.offset, complete.len() as u64);

    reopened.observe(&text(1, "after the crash"));
    reopened.shutdown().await.expect("a clean shutdown");

    // Appending after the repair produced parseable records, not JSON glued
    // onto a broken object.
    let recovery = read_journal(&path).await.expect("a readable journal");
    assert!(recovery.truncated_tail.is_none());
    let sequences: Vec<u64> = recovery.events().iter().map(|event| event.seq).collect();
    assert_eq!(sequences, [0, 1]);
}

#[tokio::test]
async fn shutdown_drains_every_queued_record() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) = open_journal(
        &directory,
        JournalConfig {
            queue_capacity: 4096,
            ..JournalConfig::default()
        },
        DefaultRedactor::new(),
    )
    .await;

    // A current-thread runtime: this loop never awaits, so the writer task
    // cannot have run. Everything asserted below was still queued when
    // shutdown was called.
    for seq in 0..500 {
        journal.observe(&text(seq, "queued"));
    }
    assert_eq!(journal.dropped(), 0);

    let stats = journal.shutdown().await.expect("a clean shutdown");
    assert_eq!(stats.written, 500);
    assert_eq!(stats.dropped, 0);

    let recovery = read_journal(&path).await.expect("a readable journal");
    assert_eq!(recovery.events().len(), 500);
    assert!(recovery.truncated_tail.is_none());
}

#[tokio::test]
async fn queue_overflow_is_recorded_as_a_marker_rather_than_lost_silently() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) = open_journal(
        &directory,
        JournalConfig {
            queue_capacity: 4,
            ..JournalConfig::default()
        },
        DefaultRedactor::new(),
    )
    .await;

    // Same trick as above, used to guarantee the overflow: the writer cannot
    // drain while this loop runs, so everything past the fourth record is
    // rejected.
    for seq in 0..20 {
        journal.observe(&text(seq, "burst"));
    }
    assert_eq!(journal.dropped(), 16);

    let stats = journal.shutdown().await.expect("a clean shutdown");
    assert_eq!(stats.written, 4);
    assert_eq!(stats.dropped, 16);

    let recovery = read_journal(&path).await.expect("a readable journal");
    let lost: u64 = recovery
        .records
        .iter()
        .filter_map(|line| match &line.record {
            JournalRecord::Dropped { count, .. } => Some(*count),
            _ => None,
        })
        .sum();
    assert_eq!(lost, 16, "the journal states exactly how much it lost");
    assert_eq!(recovery.events().len(), 4);
}

#[tokio::test]
async fn flush_makes_queued_records_readable_without_ending_the_session() {
    let directory = tempfile::tempdir().expect("a temp dir");
    let (path, journal) =
        open_journal(&directory, JournalConfig::default(), DefaultRedactor::new()).await;

    journal.observe(&text(0, "before the flush"));
    journal.flush().await.expect("a flush");
    assert_eq!(
        read_journal(&path)
            .await
            .expect("a readable journal")
            .events()
            .len(),
        1
    );

    journal.observe(&text(1, "after the flush"));
    let stats = journal.shutdown().await.expect("a clean shutdown");
    assert_eq!(stats.written, 2);
}
