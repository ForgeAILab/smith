use super::redaction::REDACTED;
use super::*;
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::ids::{AttemptId, RequestId};

#[test]
fn credential_shaped_keys_are_replaced_by_value() {
    let mut line = serde_json::json!({
        "schema_version": 1,
        "arguments": {"path": "src/main.rs", "API_KEY": "sk-live-abc"},
    });
    DefaultRedactor::new().redact(&mut line);
    assert_eq!(line["arguments"]["API_KEY"], REDACTED);
    assert_eq!(line["arguments"]["path"], "src/main.rs");
}

#[test]
fn token_counters_survive_redaction() {
    // The shared vocabulary counts `*_tokens` everywhere. Treating those
    // keys as credential-shaped would silently destroy usage accounting.
    let mut line = serde_json::json!({
        "reserved_tokens": 512,
        "input_budget_tokens": 8000,
        "read_tokens": 40,
        "access_token": "sk-live-abc",
    });
    DefaultRedactor::new().redact(&mut line);
    assert_eq!(line["reserved_tokens"], 512);
    assert_eq!(line["input_budget_tokens"], 8000);
    assert_eq!(line["read_tokens"], 40);
    assert_eq!(line["access_token"], REDACTED);
}

#[test]
fn a_registered_secret_is_replaced_inside_free_text() {
    let mut line = serde_json::json!({"text": "use sk-live-abc for auth"});
    DefaultRedactor::new()
        .with_secret("sk-live-abc")
        .redact(&mut line);
    assert_eq!(line["text"], "use [redacted] for auth");
}

#[test]
fn a_redacted_clone_preserves_operational_fields_and_canonical_arguments() {
    let canonical = serde_json::json!({
        "path": "src/main.rs",
        "limit": 40,
        "api_key": "sk-live-abc",
        "command": "curl -H sk-live-abc https://example.test"
    });
    let redacted = DefaultRedactor::new()
        .with_secret("sk-live-abc")
        .redacted_clone(&canonical);

    assert_eq!(redacted["path"], "src/main.rs");
    assert_eq!(redacted["limit"], 40);
    assert_eq!(redacted["api_key"], REDACTED);
    assert_eq!(
        redacted["command"],
        "curl -H [redacted] https://example.test"
    );
    assert_eq!(canonical["api_key"], "sk-live-abc");
    assert!(
        canonical["command"]
            .as_str()
            .unwrap()
            .contains("sk-live-abc")
    );
}

#[test]
fn an_empty_secret_is_ignored_rather_than_matching_everywhere() {
    let mut line = serde_json::json!({"text": "harmless"});
    DefaultRedactor::new().with_secret("").redact(&mut line);
    assert_eq!(line["text"], "harmless");
}

#[test]
fn registered_secrets_never_appear_in_debug_output() {
    let redactor = DefaultRedactor::new().with_secret("sk-live-abc");
    let rendered = format!("{redactor:?}");
    assert_eq!(rendered, "DefaultRedactor { registered_secrets: 1 }");
    assert!(!rendered.contains("sk-live-abc"));
}

#[test]
fn a_marker_line_is_distinguishable_from_an_event_line() {
    let line = JournalLine::new(JournalRecord::Dropped {
        count: 3,
        before_seq: Some(9),
        lowest_dropped_seq: Some(6),
        highest_dropped_seq: Some(8),
    });
    let json = serde_json::to_value(&line).expect("serializable");
    assert_eq!(json["schema_version"], JOURNAL_SCHEMA_VERSION);
    assert_eq!(json["record"], "dropped");
    assert_eq!(json["count"], 3);
}

#[tokio::test]
async fn monitor_lifecycle_markers_are_metadata_only_validated_and_durable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.jsonl");
    let journal = EventJournal::open(&path, JournalConfig::default(), Arc::new(KeepEverything))
        .await
        .unwrap();

    journal
        .record_monitor_started("monitor:build-1")
        .await
        .unwrap();
    journal
        .record_monitor_stopped("monitor:build-1")
        .await
        .unwrap();
    let error = journal
        .record_monitor_started("monitor id contains task content")
        .await
        .expect_err("free text is not a monitor identity");
    assert_eq!(error.kind, ErrorKind::Serialization);
    journal.shutdown().await.unwrap();

    let recovery = read_journal(path).await.unwrap();
    assert_eq!(
        recovery
            .records
            .iter()
            .map(|line| line.record.clone())
            .collect::<Vec<_>>(),
        vec![
            JournalRecord::MonitorStarted {
                monitor: "monitor:build-1".into(),
            },
            JournalRecord::MonitorStopped {
                monitor: "monitor:build-1".into(),
            },
        ]
    );
}

#[tokio::test]
async fn task_lifecycle_markers_are_metadata_only_validated_and_durable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.jsonl");
    let journal = EventJournal::open(&path, JournalConfig::default(), Arc::new(KeepEverything))
        .await
        .unwrap();

    journal.record_task_started("task:1").await.unwrap();
    journal.record_task_exited("task:1").await.unwrap();
    let error = journal
        .record_task_started("task id contains spool output")
        .await
        .expect_err("free text is not a task identity");
    assert_eq!(error.kind, ErrorKind::Serialization);
    journal.shutdown().await.unwrap();

    let recovery = read_journal(&path).await.unwrap();
    assert_eq!(
        recovery
            .records
            .iter()
            .map(|line| line.record.clone())
            .collect::<Vec<_>>(),
        vec![
            JournalRecord::TaskStarted {
                task: "task:1".into(),
            },
            JournalRecord::TaskExited {
                task: "task:1".into(),
            },
        ]
    );

    // Metadata-only: each line carries the schema version, the record
    // tag, and the bare task identity — never a spooled output body.
    let raw = tokio::fs::read_to_string(&path).await.unwrap();
    for line in raw.lines() {
        let value: Value = serde_json::from_str(line).unwrap();
        let keys: std::collections::BTreeSet<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["schema_version", "record", "task"].into_iter().collect()
        );
    }
}

#[tokio::test]
async fn journal_reads_reject_unsupported_or_unvalidated_marker_metadata() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.jsonl");
    let cases = [
        (
            serde_json::json!({
                "schema_version": JOURNAL_SCHEMA_VERSION + 1,
                "record": "dropped",
                "count": 1
            }),
            "unsupported journal schema version",
        ),
        (
            serde_json::json!({
                "schema_version": JOURNAL_SCHEMA_VERSION,
                "record": "ephemeral_work_interrupted",
                "interruption": {
                    "schema_version": EPHEMERAL_INTERRUPTION_SCHEMA_VERSION + 1,
                    "reason": "process_exit"
                }
            }),
            "unsupported ephemeral interruption marker schema",
        ),
        (
            serde_json::json!({
                "schema_version": JOURNAL_SCHEMA_VERSION,
                "record": "monitor_started",
                "monitor": "monitor id with spaces"
            }),
            "monitor id must contain",
        ),
        (
            serde_json::json!({
                "schema_version": JOURNAL_SCHEMA_VERSION,
                "record": "task_started",
                "task": "task id with spaces"
            }),
            "task id must contain",
        ),
        (
            serde_json::json!({
                "schema_version": JOURNAL_SCHEMA_VERSION,
                "record": "ephemeral_work_interrupted",
                "interruption": {
                    "schema_version": EPHEMERAL_INTERRUPTION_SCHEMA_VERSION,
                    "reason": "process_exit",
                    "children": ["child-2", "child-1"],
                    "monitors": ["monitor:build", "monitor:build"]
                }
            }),
            "sorted and unique",
        ),
    ];

    for (record, expected) in cases {
        tokio::fs::write(&path, format!("{record}\n"))
            .await
            .unwrap();
        let error = read_journal(&path)
            .await
            .expect_err("unvalidated persisted metadata must fail closed");
        assert_eq!(error.kind, ErrorKind::Serialization);
        assert!(error.message.contains(expected), "{error}");
    }
}

#[tokio::test]
async fn nonterminal_reconciliation_keeps_only_the_strict_watermark_prefix() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.jsonl");
    let session = SessionId::new("session-1");
    let retained = EventEnvelope::new(
        3,
        EventId::new("evt-4"),
        session.clone(),
        Some(TurnId::new("turn-2")),
        Timestamp::ZERO,
        RuntimeEvent::ProviderAttemptOutputCommitted {
            request: RequestId::new("req-5"),
            attempt: AttemptId::new("att-6"),
        },
    );
    let discarded = EventEnvelope::new(
        4,
        EventId::new("evt-5"),
        session,
        Some(TurnId::new("turn-2")),
        Timestamp::ZERO,
        RuntimeEvent::TurnCompleted {
            finish: agent_runtime_core::event::TurnFinish::Completed,
            visible_output: true,
        },
    );
    let records = [
        JournalLine::new(JournalRecord::Dropped {
            count: 1,
            before_seq: Some(3),
            lowest_dropped_seq: Some(2),
            highest_dropped_seq: Some(2),
        }),
        JournalLine::new(JournalRecord::Event { event: retained }),
        JournalLine::new(JournalRecord::Event { event: discarded }),
        JournalLine::new(JournalRecord::Dropped {
            count: 2,
            before_seq: None,
            lowest_dropped_seq: Some(5),
            highest_dropped_seq: Some(6),
        }),
    ];
    let mut bytes = Vec::new();
    for line in records {
        serde_json::to_writer(&mut bytes, &line).unwrap();
        bytes.push(b'\n');
    }
    write_private_atomically(&path, &bytes).await.unwrap();

    let reconciled = reconcile_nonterminal_journal(&path, 4).await.unwrap();

    assert_eq!(reconciled.truncated_records, 2);
    assert!(reconciled.retained_gap);
    assert_eq!(reconciled.identity_floor.event_seq, 4);
    assert_eq!(reconciled.identity_floor.event, 4);
    assert_eq!(reconciled.identity_floor.turn, 2);
    assert_eq!(reconciled.identity_floor.request, 5);
    assert_eq!(reconciled.identity_floor.attempt, 6);
    let recovered = read_journal(&path).await.unwrap();
    assert_eq!(recovered.records.len(), 2);
    assert_eq!(
        recovered
            .events()
            .into_iter()
            .map(|event| event.seq)
            .collect::<Vec<_>>(),
        vec![3]
    );
}

#[tokio::test]
async fn a_sticky_writer_failure_is_returned_by_every_flush() {
    let root = tempfile::tempdir().unwrap();
    let journal = EventJournal::open(
        root.path().join("session.jsonl"),
        JournalConfig::default(),
        Arc::new(KeepEverything),
    )
    .await
    .unwrap();
    *journal.failure.lock().unwrap() =
        Some(RuntimeError::internal("injected prior append failure"));

    for _ in 0..2 {
        let error = journal.flush().await.unwrap_err();
        assert!(error.message.contains("injected prior append failure"));
    }
    assert!(journal.shutdown().await.is_err());
}

#[tokio::test]
async fn checkpoint_flush_gives_queued_drops_an_exact_reconciliation_boundary() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("session.jsonl");
    let journal = EventJournal::open(&path, JournalConfig::default(), Arc::new(KeepEverything))
        .await
        .unwrap();
    journal.dropped.store(3, Ordering::Relaxed);
    *journal.dropped_seq_range.lock().unwrap() = Some((12, 14));

    journal.flush_before(9).await.unwrap();
    journal.shutdown().await.unwrap();

    let before = read_journal(&path).await.unwrap();
    assert!(before.records.iter().any(|line| {
        matches!(
            line.record,
            JournalRecord::Dropped {
                count: 3,
                before_seq: Some(9),
                lowest_dropped_seq: Some(12),
                highest_dropped_seq: Some(14),
            }
        )
    }));
    let reconciled = reconcile_nonterminal_journal(&path, 9).await.unwrap();
    assert!(reconciled.retained_gap);
    assert_eq!(reconciled.truncated_records, 0);
    let after = read_journal(&path).await.unwrap();
    assert_eq!(after.records, before.records);
}
