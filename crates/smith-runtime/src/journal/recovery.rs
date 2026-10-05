use super::*;

/// An incomplete final record found at the end of a journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TruncatedTail {
    /// The byte offset the last complete record ended at.
    pub offset: u64,
    /// How many bytes of the incomplete record were present.
    pub bytes: usize,
}

/// The result of reading a journal from disk.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JournalRecovery {
    /// Every complete record, in file order.
    pub records: Vec<JournalLine>,
    /// The incomplete final record, when the file ended mid-write.
    pub truncated_tail: Option<TruncatedTail>,
}

impl JournalRecovery {
    /// The canonical runtime envelopes, with markers filtered out.
    pub fn events(&self) -> Vec<&EventEnvelope> {
        self.records
            .iter()
            .filter_map(|line| match &line.record {
                JournalRecord::Event { event } => Some(event),
                _ => None,
            })
            .collect()
    }

    /// Derives a monotonic runtime identity floor from every complete record.
    ///
    /// Marker identities participate where available, so an oversized event
    /// still prevents reuse of its sequence and event id. Dropped markers
    /// deliberately cannot recreate identities that were never persisted.
    pub fn identity_floor(&self) -> SessionIdentityState {
        identity_floor(self.records.iter())
    }
}

/// Result of reconciling a nonterminal checkpoint with its journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JournalReconciliation {
    /// Identity counters derived from the retained durable prefix.
    pub identity_floor: SessionIdentityState,
    /// Number of complete records at or after the checkpoint watermark that
    /// were removed before runtime resume.
    pub truncated_records: usize,
    /// Whether the retained prefix contains an explicit dropped/oversized gap.
    pub retained_gap: bool,
}

/// Rewrites a nonterminal session journal to the exact checkpoint boundary.
///
/// Events with sequence `>= event_sequence` are presentation-only tail: the
/// resumed turn will emit its canonical continuation again. Keeping that tail
/// would duplicate commit/terminal events in replay. Terminal checkpoints do
/// not use this function because their later `TurnCompleted` event is valid.
pub async fn reconcile_nonterminal_journal(
    path: impl AsRef<Path>,
    event_sequence: u64,
) -> Result<JournalReconciliation, RuntimeError> {
    let path = path.as_ref();
    let recovery = read_journal(path).await?;
    let original_len = recovery.records.len();
    let mut retained = Vec::with_capacity(original_len);
    for line in recovery.records {
        let keep = match &line.record {
            JournalRecord::Event { event } => event.seq < event_sequence,
            JournalRecord::Oversized { seq, .. } => *seq < event_sequence,
            JournalRecord::Dropped {
                before_seq: Some(seq),
                ..
            } => *seq <= event_sequence,
            // An end-of-stream drop marker has no ordering identity. It could
            // describe the discarded tail, so a nonterminal resume cannot
            // safely retain it as part of the exact prefix.
            JournalRecord::Dropped {
                before_seq: None, ..
            } => false,
            JournalRecord::EphemeralWorkInterrupted { .. } => true,
            JournalRecord::MonitorStarted { .. }
            | JournalRecord::MonitorStopped { .. }
            | JournalRecord::TaskStarted { .. }
            | JournalRecord::TaskExited { .. } => true,
        };
        if keep {
            retained.push(line);
        }
    }

    let reconciliation = JournalReconciliation {
        identity_floor: identity_floor(retained.iter()),
        truncated_records: original_len.saturating_sub(retained.len()),
        retained_gap: retained.iter().any(|line| {
            matches!(
                line.record,
                JournalRecord::Dropped { .. } | JournalRecord::Oversized { .. }
            )
        }),
    };

    if reconciliation.truncated_records > 0 || recovery.truncated_tail.is_some() {
        let mut bytes = Vec::new();
        for line in &retained {
            serde_json::to_writer(&mut bytes, line).map_err(|error| {
                RuntimeError::new(
                    ErrorKind::Serialization,
                    format!("journal prefix could not be serialized: {error}"),
                )
            })?;
            bytes.push(b'\n');
        }
        write_private_atomically(path, &bytes).await?;
    }
    Ok(reconciliation)
}

fn identity_floor<'a>(records: impl IntoIterator<Item = &'a JournalLine>) -> SessionIdentityState {
    let mut floor = SessionIdentityState::default();
    for line in records {
        match &line.record {
            JournalRecord::Event { event } => {
                floor.event_seq = floor.event_seq.max(event.seq.saturating_add(1));
                floor.event = floor.event.max(id_number(event.id.as_str(), "evt-"));
                if let Some(turn) = &event.turn {
                    floor.turn = floor.turn.max(id_number(turn.as_str(), "turn-"));
                }
                match &event.payload {
                    agent_runtime_core::event::RuntimeEvent::ProviderAttemptStarted {
                        request,
                        attempt,
                        ..
                    }
                    | agent_runtime_core::event::RuntimeEvent::TextDelta {
                        request, attempt, ..
                    }
                    | agent_runtime_core::event::RuntimeEvent::ReasoningDelta {
                        request,
                        attempt,
                        ..
                    }
                    | agent_runtime_core::event::RuntimeEvent::ProviderAttemptOutputCommitted {
                        request,
                        attempt,
                    }
                    | agent_runtime_core::event::RuntimeEvent::ProviderAttemptOutputDiscarded {
                        request,
                        attempt,
                    } => {
                        floor.request = floor.request.max(id_number(request.as_str(), "req-"));
                        floor.attempt = floor.attempt.max(id_number(attempt.as_str(), "att-"));
                    }
                    agent_runtime_core::event::RuntimeEvent::ProviderAttemptFinished {
                        attempt,
                        ..
                    } => {
                        floor.attempt = floor.attempt.max(id_number(attempt.as_str(), "att-"));
                    }
                    agent_runtime_core::event::RuntimeEvent::ToolCallRequested { call, .. }
                    | agent_runtime_core::event::RuntimeEvent::ToolCallCompleted { call, .. } => {
                        floor.tool_call = floor.tool_call.max(id_number(call.as_str(), "call-"));
                    }
                    _ => {}
                }
            }
            JournalRecord::Oversized { seq, id, turn, .. } => {
                floor.event_seq = floor.event_seq.max(seq.saturating_add(1));
                floor.event = floor.event.max(id_number(id.as_str(), "evt-"));
                if let Some(turn) = turn {
                    floor.turn = floor.turn.max(id_number(turn.as_str(), "turn-"));
                }
            }
            JournalRecord::Dropped {
                before_seq: Some(seq),
                ..
            } => {
                floor.event_seq = floor.event_seq.max(*seq);
            }
            JournalRecord::Dropped {
                before_seq: None, ..
            } => {}
            JournalRecord::EphemeralWorkInterrupted { .. } => {}
            JournalRecord::MonitorStarted { .. }
            | JournalRecord::MonitorStopped { .. }
            | JournalRecord::TaskStarted { .. }
            | JournalRecord::TaskExited { .. } => {}
        }
    }
    floor
}

pub(super) fn validate_monitor_id(monitor: String) -> Result<String, RuntimeError> {
    validate_ephemeral_id("monitor", &monitor, MAX_MONITOR_ID_CHARS)?;
    Ok(monitor)
}

pub(super) fn validate_task_id(task: String) -> Result<String, RuntimeError> {
    validate_ephemeral_id("task", &task, MAX_TASK_ID_CHARS)?;
    Ok(task)
}

fn validate_journal_line(line: &JournalLine) -> Result<(), RuntimeError> {
    if line.schema_version != JOURNAL_SCHEMA_VERSION {
        return Err(RuntimeError::new(
            ErrorKind::Serialization,
            format!("unsupported journal schema version {}", line.schema_version),
        ));
    }
    match &line.record {
        JournalRecord::EphemeralWorkInterrupted { interruption } => {
            validate_ephemeral_interruption(interruption)
        }
        JournalRecord::MonitorStarted { monitor } | JournalRecord::MonitorStopped { monitor } => {
            validate_ephemeral_id("monitor", monitor, MAX_MONITOR_ID_CHARS)
        }
        JournalRecord::TaskStarted { task } | JournalRecord::TaskExited { task } => {
            validate_ephemeral_id("task", task, MAX_TASK_ID_CHARS)
        }
        JournalRecord::Event { .. }
        | JournalRecord::Oversized { .. }
        | JournalRecord::Dropped { .. } => Ok(()),
    }
}

pub(super) fn validate_ephemeral_interruption(
    interruption: &EphemeralWorkInterruption,
) -> Result<(), RuntimeError> {
    if interruption.schema_version != EPHEMERAL_INTERRUPTION_SCHEMA_VERSION {
        return Err(RuntimeError::new(
            ErrorKind::Serialization,
            format!(
                "unsupported ephemeral interruption marker schema {}",
                interruption.schema_version
            ),
        ));
    }
    if interruption
        .children
        .len()
        .saturating_add(interruption.monitors.len())
        .saturating_add(interruption.tasks.len())
        > MAX_INTERRUPTED_WORK_IDS
    {
        return Err(RuntimeError::new(
            ErrorKind::Serialization,
            format!(
                "ephemeral interruption marker exceeds the {MAX_INTERRUPTED_WORK_IDS}-identity bound"
            ),
        ));
    }
    for child in &interruption.children {
        validate_ephemeral_id("child", child.as_str(), MAX_EPHEMERAL_CHILD_ID_CHARS)?;
    }
    for monitor in &interruption.monitors {
        validate_ephemeral_id("monitor", monitor, MAX_MONITOR_ID_CHARS)?;
    }
    for task in &interruption.tasks {
        validate_ephemeral_id("task", task, MAX_TASK_ID_CHARS)?;
    }
    if !strictly_sorted(&interruption.children)
        || !strictly_sorted(&interruption.monitors)
        || !strictly_sorted(&interruption.tasks)
    {
        return Err(RuntimeError::new(
            ErrorKind::Serialization,
            "ephemeral interruption identities must be sorted and unique",
        ));
    }
    Ok(())
}

fn validate_ephemeral_id(label: &str, value: &str, max_chars: usize) -> Result<(), RuntimeError> {
    let chars = value.chars().count();
    if chars == 0
        || chars > max_chars
        || value.trim() != value
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._:-".contains(character))
    {
        return Err(RuntimeError::new(
            ErrorKind::Serialization,
            format!(
                "{label} id must contain 1..={max_chars} ASCII letters, digits, `.`, `_`, `:`, or `-`"
            ),
        ));
    }
    Ok(())
}

fn strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

fn id_number(id: &str, prefix: &str) -> u64 {
    id.strip_prefix(prefix)
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

/// Reads every complete record from a journal, reporting an incomplete tail.
///
/// A missing file reads as an empty journal — a session that crashed before
/// its first event is not a corrupt session. A *complete* line that does not
/// parse is an error rather than a silent skip: that is real corruption, and
/// only the unterminated final line is attributable to a crash.
pub async fn read_journal(path: impl AsRef<Path>) -> Result<JournalRecovery, RuntimeError> {
    let path = path.as_ref();
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(JournalRecovery::default());
        }
        Err(err) => {
            return Err(RuntimeError::new(
                ErrorKind::Internal,
                format!("cannot read journal `{}`: {err}", path.display()),
            ));
        }
    };

    let mut recovery = JournalRecovery::default();
    let mut offset = 0usize;
    let mut line_number = 0usize;
    loop {
        match bytes[offset..].iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                let line = &bytes[offset..offset + index];
                offset += index + 1;
                line_number += 1;
                if line.is_empty() {
                    continue;
                }
                let parsed: JournalLine = serde_json::from_slice(line).map_err(|err| {
                    RuntimeError::new(
                        ErrorKind::Serialization,
                        format!(
                            "journal `{}` line {line_number} is not a readable record: {err}",
                            path.display()
                        ),
                    )
                })?;
                validate_journal_line(&parsed).map_err(|error| {
                    RuntimeError::new(
                        ErrorKind::Serialization,
                        format!(
                            "journal `{}` line {line_number} failed validation: {}",
                            path.display(),
                            error.message
                        ),
                    )
                })?;
                recovery.records.push(parsed);
            }
            None => {
                let remainder = bytes.len() - offset;
                if remainder > 0 {
                    recovery.truncated_tail = Some(TruncatedTail {
                        offset: offset as u64,
                        bytes: remainder,
                    });
                }
                break;
            }
        }
    }
    Ok(recovery)
}

/// Truncates a trailing partial line so appends resume on a record boundary.
///
/// Deliberately byte-level: it must repair a file whose complete records this
/// build may not even be able to parse, so it looks only for the last record
/// terminator.
pub(super) async fn repair_incomplete_tail(
    path: &Path,
) -> Result<Option<TruncatedTail>, RuntimeError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(RuntimeError::new(
                ErrorKind::Internal,
                format!("cannot inspect journal `{}`: {err}", path.display()),
            ));
        }
    };
    if bytes.is_empty() || bytes.last() == Some(&b'\n') {
        return Ok(None);
    }

    let boundary = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let tail = TruncatedTail {
        offset: boundary as u64,
        bytes: bytes.len() - boundary,
    };

    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .await
        .map_err(|err| {
            RuntimeError::new(
                ErrorKind::Internal,
                format!("cannot repair journal `{}`: {err}", path.display()),
            )
        })?;
    file.set_len(tail.offset).await.map_err(|err| {
        RuntimeError::new(
            ErrorKind::Internal,
            format!("cannot truncate journal `{}`: {err}", path.display()),
        )
    })?;
    file.sync_all().await.map_err(|err| {
        RuntimeError::new(
            ErrorKind::Internal,
            format!("cannot sync repaired journal `{}`: {err}", path.display()),
        )
    })?;
    Ok(Some(tail))
}
