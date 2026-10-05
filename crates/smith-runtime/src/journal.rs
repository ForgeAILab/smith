//! The canonical JSON Lines event journal.
//!
//! Every event Agent Runtime emits is appended, one complete JSON object per
//! line, to `<session-id>.jsonl` beside that session's snapshot. The snapshot
//! is the *state* a resume starts from; the journal is the *history* that
//! explains how it got there and the only record that survives a crash between
//! two saves.
//!
//! Four properties are load-bearing, and each one exists because of a specific
//! way a naive append-only log fails:
//!
//! - **`observe` never touches the disk.** The shared [`EventObserver`]
//!   contract is synchronous and is called on the runtime's path, so a slow
//!   fsync here would stall a provider stream. Events cross a bounded channel
//!   to a single writer task; the hot path does a clone and a non-blocking
//!   send.
//! - **Loss is recorded, never silent.** The channel is bounded, so a burst
//!   that outruns the disk must go somewhere. Blocking the runtime is not an
//!   option and growing without limit is not either, so the overflow policy is
//!   to drop and *count*: the next record written is preceded by a
//!   [`JournalRecord::Dropped`] marker naming how many were lost, which
//!   sequence number resumes the record, and — since the queue is FIFO, so an
//!   overflow always rejects the newest arrivals rather than anything already
//!   queued — the lowest and highest sequence actually rejected. A reader can
//!   always tell a complete journal from a lossy one, and locate exactly what
//!   the loss removed.
//! - **A record is written whole or not at all.** The line is serialized
//!   completely, then written in a single `write_all` of `record + "\n"`. A
//!   record too large for the configured bound is replaced by a
//!   [`JournalRecord::Oversized`] marker carrying its size and identity —
//!   never truncated mid-object, which would produce a line no reader can
//!   parse.
//! - **Secrets are removed before serialization reaches the disk.**
//!   [`Redactor`] is an explicit seam rather than an incidental filter, because
//!   "no secret in the journal" is a property that has to be provable in a
//!   test rather than argued from the absence of a known leak.
//!
//! # Flush and shutdown
//!
//! [`EventJournal::flush`] waits until every record queued *before the call*
//! has reached the file and been synced; the channel preserves order, so
//! nothing later can jump ahead of it. [`EventJournal::shutdown`] does the same
//! and then stops the writer, returning the run's [`JournalStats`]. A clean
//! Smith exit calls `shutdown` inside its grace period. Dropping the journal
//! without shutting it down abandons whatever is still queued — the writer task
//! has no way to outlive the process — so shutdown is the contract, not an
//! optimization.
//!
//! # Crash recovery
//!
//! A crash truncates the final write, which leaves a last line with no
//! terminating newline — the only signature a partial record can have, since
//! bytes reach the file in order. [`read_journal`] reports that tail through
//! [`JournalRecovery::truncated_tail`] and returns every complete record before
//! it, and [`EventJournal::open`] physically truncates it so appends resume on
//! a record boundary instead of concatenating new JSON onto broken JSON.

use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use agent_runtime_core::clock::Timestamp;
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::event::EventEnvelope;
use agent_runtime_core::ids::{ChildId, EventId, SessionId, TurnId};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::store::{Secret, SessionIdentityState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};

use crate::private_storage::write_private_atomically;
use crate::session::SessionPaths;

/// The schema version stamped on every journal line.
///
/// Present on markers as well as events so a reader can identify what it is
/// parsing from the first field, without inferring it from the file name.
pub const JOURNAL_SCHEMA_VERSION: u32 = 1;

/// Schema for Smith-owned ephemeral-work recovery markers.
pub const EPHEMERAL_INTERRUPTION_SCHEMA_VERSION: u32 = 1;
/// Maximum length of one metadata-only monitor identity.
pub const MAX_MONITOR_ID_CHARS: usize = 128;
/// Maximum length of one metadata-only background task identity.
pub const MAX_TASK_ID_CHARS: usize = 128;
/// Maximum length of one metadata-only child identity in recovery markers.
pub const MAX_EPHEMERAL_CHILD_ID_CHARS: usize = 128;
/// Defensive bound on process-owned identities in one recovery marker.
pub const MAX_INTERRUPTED_WORK_IDS: usize = 1_024;

/// One line of the journal: an explicit schema version and one record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalLine {
    /// The journal vocabulary version.
    pub schema_version: u32,
    /// The record itself.
    #[serde(flatten)]
    pub record: JournalRecord,
}

impl JournalLine {
    /// Stamps `record` with the current schema version.
    pub fn new(record: JournalRecord) -> Self {
        Self {
            schema_version: JOURNAL_SCHEMA_VERSION,
            record,
        }
    }
}

/// What a journal line can be.
///
/// Only [`JournalRecord::Event`] carries runtime history; every other record is
/// an explicitly tagged Smith marker. A reader can therefore never mistake
/// recovery reconciliation or an observability gap for a shared runtime event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
// Runtime's canonical event envelope intentionally remains inline so the
// existing JSONL wire shape and recovery path stay unchanged as it grows.
#[allow(clippy::large_enum_variant)]
pub enum JournalRecord {
    /// A canonical runtime event envelope, exactly as the runtime emitted it.
    Event {
        /// The envelope.
        event: EventEnvelope,
    },
    /// An event whose serialized form exceeded the configured record bound.
    ///
    /// Carries enough identity to attribute the gap to a turn and a position
    /// in the sequence; the payload itself is not persisted at any size.
    Oversized {
        /// The dropped event's sequence number.
        seq: u64,
        /// The dropped event's id.
        id: EventId,
        /// The owning session.
        session: SessionId,
        /// The owning turn, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn: Option<TurnId>,
        /// When the event was emitted.
        timestamp: Timestamp,
        /// The event variant's canonical discriminant, e.g. `"text_delta"`.
        event: String,
        /// The serialized size, in bytes, of the record that was replaced.
        bytes: usize,
    },
    /// Records the queue rejected because the writer could not keep up.
    Dropped {
        /// How many records were lost.
        count: u64,
        /// The sequence number of the first record written after the writer
        /// noticed the loss, or `None` when it was accounted for at flush or
        /// shutdown with no record following it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_seq: Option<u64>,
        /// The lowest sequence number `observe` actually rejected since the
        /// previous marker.
        ///
        /// `before_seq` names where the journal *resumes* — an old record
        /// still draining from a full queue — never the sequence of anything
        /// that was lost. The queue is FIFO, so an overflow always rejects
        /// the newest arrivals, the tail of the burst; this field and
        /// `highest_dropped_seq` locate that tail directly, from the
        /// sequence numbers `observe` saw rejected. `None` on a marker a
        /// build before this field existed wrote, or when no rejected event
        /// carried a usable sequence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lowest_dropped_seq: Option<u64>,
        /// The highest sequence number `observe` actually rejected since the
        /// previous marker. See `lowest_dropped_seq`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        highest_dropped_seq: Option<u64>,
    },
    /// A prior process owned ephemeral work that cannot survive resume.
    ///
    /// This is Smith orchestration metadata, not a fabricated runtime child
    /// event. Child/question content has no field here.
    EphemeralWorkInterrupted {
        /// Independently versioned Smith marker payload.
        interruption: EphemeralWorkInterruption,
    },
    /// A process-owned monitor identity became live.
    ///
    /// This is metadata only. It does not imply that Smith Runtime implements
    /// monitor execution; the future executor calls this lifecycle seam.
    MonitorStarted {
        /// Stable process-owned monitor identity.
        monitor: String,
    },
    /// A process-owned monitor identity reached an orderly terminal boundary.
    MonitorStopped {
        /// Stable process-owned monitor identity.
        monitor: String,
    },
    /// A process-owned background task identity became live.
    TaskStarted {
        /// Stable process-owned background task identity.
        task: String,
    },
    /// A process-owned background task reached an orderly terminal boundary.
    TaskExited {
        /// Stable process-owned background task identity.
        task: String,
    },
}

/// Why recovered ephemeral work was ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EphemeralInterruptionReason {
    /// The prior Smith process exited without a terminal work record.
    ProcessExit,
}

/// Metadata-only reconciliation of process-owned work found during resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EphemeralWorkInterruption {
    /// Independent payload schema.
    pub schema_version: u32,
    /// Why the prior work cannot continue.
    pub reason: EphemeralInterruptionReason,
    /// Child identities in deterministic order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ChildId>,
    /// Monitor identities in deterministic order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub monitors: Vec<String>,
    /// Background task identities in deterministic order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<String>,
}

impl EphemeralWorkInterruption {
    /// Builds a deterministic process-exit marker.
    pub fn process_exit(
        children: impl IntoIterator<Item = ChildId>,
        monitors: impl IntoIterator<Item = String>,
        tasks: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut children = children.into_iter().collect::<Vec<_>>();
        children.sort();
        children.dedup();
        let mut monitors = monitors.into_iter().collect::<Vec<_>>();
        monitors.sort();
        monitors.dedup();
        let mut tasks = tasks.into_iter().collect::<Vec<_>>();
        tasks.sort();
        tasks.dedup();
        Self {
            schema_version: EPHEMERAL_INTERRUPTION_SCHEMA_VERSION,
            reason: EphemeralInterruptionReason::ProcessExit,
            children,
            monitors,
            tasks,
        }
    }

    /// Whether the marker has no work identities.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty() && self.monitors.is_empty() && self.tasks.is_empty()
    }
}

/// What one journal run wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JournalStats {
    /// Event records written in full.
    pub written: u64,
    /// Events replaced by an [`JournalRecord::Oversized`] marker.
    pub oversized: u64,
    /// Events the bounded queue rejected.
    pub dropped: u64,
}

/// How the journal is bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalConfig {
    /// How many records may be queued before `observe` starts dropping.
    ///
    /// This is the entire back-pressure budget: it buys the writer time to
    /// absorb a burst without ever making the runtime wait on a disk. Each
    /// queued item is a boxed [`EventEnvelope`], so raising this bound costs
    /// one pointer-sized slot per unit — not a copy of the record inline in
    /// the channel — which is why it can be sized for a real burst rather
    /// than trimmed to save memory.
    pub queue_capacity: usize,
    /// The largest serialized record written verbatim.
    pub max_record_bytes: usize,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            // Measured provider streams have bursted to 1046 events in a
            // single millisecond, and the previous bound of 1024 measured
            // 52% event loss on one real session — the queue was full before
            // the writer got a turn. Boxing each queued item (see
            // `queue_capacity`'s doc) keeps this bound cheap to raise.
            queue_capacity: 16_384,
            // Large enough for a realistic tool-call payload, small enough
            // that one pathological event cannot dominate a session's file.
            max_record_bytes: 64 * 1024,
        }
    }
}

/// What the writer task is asked to do. Records and control messages share one
/// channel so a flush can never be reordered ahead of the records it is
/// supposed to wait for.
enum JournalCommand {
    /// Append one event. Boxed to keep the queued item small.
    Record(Box<EventEnvelope>),
    /// Append and durably sync one Smith-owned marker.
    Marker {
        record: Box<JournalRecord>,
        reply: oneshot::Sender<Result<(), RuntimeError>>,
    },
    /// Sync everything written so far, then acknowledge.
    Flush {
        /// The next event sequence at a checkpoint boundary. `None` is an
        /// ordinary presentation-only flush.
        before_seq: Option<u64>,
        reply: oneshot::Sender<Result<(), RuntimeError>>,
    },
    /// Sync, stop, and report.
    Shutdown(oneshot::Sender<Result<JournalStats, RuntimeError>>),
}

/// An [`EventObserver`] that appends canonical events to a JSON Lines file.
#[derive(Debug)]
pub struct EventJournal {
    commands: mpsc::Sender<JournalCommand>,
    dropped: Arc<AtomicU64>,
    /// The `(lowest, highest)` sequence rejected since the writer last drained
    /// it into a marker. A `Mutex` rather than a pair of atomics: the two
    /// numbers must update together, and only the already-exceptional
    /// overflow branch of `observe` ever takes the lock, so it never contends
    /// with the accepted-path clone-and-`try_send`.
    dropped_seq_range: Arc<Mutex<Option<(u64, u64)>>>,
    failure: Arc<Mutex<Option<RuntimeError>>>,
    recovered_tail: Option<TruncatedTail>,
    /// Cached so a second `shutdown` reports the same result instead of
    /// failing against a closed channel.
    finished: Mutex<Option<JournalStats>>,
}

impl EventJournal {
    /// Opens (or creates) the journal at `path`.
    ///
    /// An incomplete final record left by a previous crash is truncated first
    /// and reported through [`EventJournal::recovered_tail`], so the first
    /// appended record begins on a record boundary.
    pub async fn open(
        path: impl AsRef<Path>,
        config: JournalConfig,
        redactor: Arc<dyn Redactor>,
    ) -> Result<Self, RuntimeError> {
        if config.queue_capacity == 0 {
            return Err(RuntimeError::new(
                ErrorKind::Config,
                "journal queue capacity must be at least 1",
            ));
        }
        let path = path.as_ref().to_path_buf();

        let recovered_tail = repair_incomplete_tail(&path).await?;
        if let Some(tail) = &recovered_tail {
            tracing::warn!(
                path = %path.display(),
                offset = tail.offset,
                bytes = tail.bytes,
                "truncated an incomplete final journal record left by a previous run"
            );
        }

        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|err| {
                RuntimeError::new(
                    ErrorKind::Internal,
                    format!("cannot open journal `{}`: {err}", path.display()),
                )
            })?;

        let (commands, receiver) = mpsc::channel(config.queue_capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_seq_range = Arc::new(Mutex::new(None));
        let failure = Arc::new(Mutex::new(None));
        let writer = Writer {
            file,
            config,
            redactor,
            dropped: Arc::clone(&dropped),
            dropped_seq_range: Arc::clone(&dropped_seq_range),
            failure: Arc::clone(&failure),
            stats: JournalStats::default(),
        };
        tokio::spawn(writer.run(receiver));

        Ok(Self {
            commands,
            dropped,
            dropped_seq_range,
            failure,
            recovered_tail,
            finished: Mutex::new(None),
        })
    }

    /// Opens the journal for `session` under the layout `paths` describes,
    /// creating the session directory if needed.
    pub async fn for_session(
        paths: &SessionPaths,
        session: &SessionId,
        config: JournalConfig,
        redactor: Arc<dyn Redactor>,
    ) -> Result<Self, RuntimeError> {
        paths.ensure_directory().await?;
        Self::open(paths.journal(session)?, config, redactor).await
    }

    /// The incomplete final record truncated when this journal was opened.
    pub fn recovered_tail(&self) -> Option<&TruncatedTail> {
        self.recovered_tail.as_ref()
    }

    /// How many records the bounded queue has rejected so far.
    ///
    /// Live, so a host can surface sustained overflow while a session is still
    /// running rather than only in the shutdown report.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// The first append or sync failure observed by the writer.
    ///
    /// Failures are sticky: a later successful filesystem operation cannot
    /// make a checkpoint barrier believe earlier events reached durable
    /// storage.
    pub fn failure(&self) -> Option<RuntimeError> {
        self.failure
            .lock()
            .expect("journal failure state poisoned")
            .clone()
    }

    /// Appends and syncs one metadata-only ephemeral-work recovery marker.
    pub async fn record_ephemeral_interruption(
        &self,
        interruption: EphemeralWorkInterruption,
    ) -> Result<(), RuntimeError> {
        validate_ephemeral_interruption(&interruption)?;
        if interruption.is_empty() {
            return Ok(());
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(JournalCommand::Marker {
                record: Box::new(JournalRecord::EphemeralWorkInterrupted { interruption }),
                reply,
            })
            .await
            .map_err(|_| closed())?;
        response.await.map_err(|_| closed())?
    }

    /// Appends and syncs one metadata-only monitor-start marker.
    ///
    /// This method deliberately does not start a task. It is the durable
    /// identity boundary a future monitor executor must call after accepting
    /// process-owned work.
    pub async fn record_monitor_started(
        &self,
        monitor: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        let monitor = validate_monitor_id(monitor.into())?;
        self.record_marker(JournalRecord::MonitorStarted { monitor })
            .await
    }

    /// Appends and syncs one metadata-only monitor-stop marker.
    ///
    /// Calling this before process exit prevents recovery from reporting the
    /// monitor as interrupted.
    pub async fn record_monitor_stopped(
        &self,
        monitor: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        let monitor = validate_monitor_id(monitor.into())?;
        self.record_marker(JournalRecord::MonitorStopped { monitor })
            .await
    }

    /// Appends and syncs one metadata-only task-start marker.
    pub async fn record_task_started(&self, task: impl Into<String>) -> Result<(), RuntimeError> {
        let task = validate_task_id(task.into())?;
        self.record_marker(JournalRecord::TaskStarted { task })
            .await
    }

    /// Appends and syncs one metadata-only task-exit marker.
    pub async fn record_task_exited(&self, task: impl Into<String>) -> Result<(), RuntimeError> {
        let task = validate_task_id(task.into())?;
        self.record_marker(JournalRecord::TaskExited { task }).await
    }

    async fn record_marker(&self, record: JournalRecord) -> Result<(), RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(JournalCommand::Marker {
                record: Box::new(record),
                reply,
            })
            .await
            .map_err(|_| closed())?;
        response.await.map_err(|_| closed())?
    }

    /// Waits until every record queued before this call has been written and
    /// synced.
    pub async fn flush(&self) -> Result<(), RuntimeError> {
        self.flush_at(None).await
    }

    /// Flushes through an exact checkpoint watermark.
    ///
    /// If observer backpressure dropped records since the last append, the
    /// persisted marker names `event_sequence` as the first sequence after
    /// those losses. Nonterminal reconciliation can then retain the gap as
    /// part of the checkpoint prefix instead of mistaking it for an unordered
    /// shutdown tail.
    pub async fn flush_before(&self, event_sequence: u64) -> Result<(), RuntimeError> {
        self.flush_at(Some(event_sequence)).await
    }

    async fn flush_at(&self, before_seq: Option<u64>) -> Result<(), RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(JournalCommand::Flush { before_seq, reply })
            .await
            .map_err(|_| closed())?;
        response.await.map_err(|_| closed())?
    }

    /// Drains the queue, syncs, stops the writer, and reports what was written.
    ///
    /// Calling it twice returns the same statistics; events observed after it
    /// are counted as dropped but are not journaled, because the session's file
    /// is closed.
    pub async fn shutdown(&self) -> Result<JournalStats, RuntimeError> {
        if let Some(stats) = *self.finished.lock().expect("journal state poisoned") {
            return Ok(stats);
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(JournalCommand::Shutdown(reply))
            .await
            .map_err(|_| closed())?;
        let stats = response.await.map_err(|_| closed())??;
        *self.finished.lock().expect("journal state poisoned") = Some(stats);
        Ok(stats)
    }
}

impl EventObserver for EventJournal {
    fn observe(&self, event: &EventEnvelope) {
        // Non-blocking by construction: a clone and a `try_send`. Everything
        // expensive — redaction, serialization, the write — happens in the
        // writer task.
        if self
            .commands
            .try_send(JournalCommand::Record(Box::new(event.clone())))
            .is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            // `event.seq` is exactly the identity a `Dropped` marker needs to
            // name what was actually lost — the queue is FIFO, so this
            // rejected send is always the newest arrival, never one already
            // sitting in the channel.
            let mut range = self
                .dropped_seq_range
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *range = Some(match *range {
                Some((lowest, highest)) => (lowest.min(event.seq), highest.max(event.seq)),
                None => (event.seq, event.seq),
            });
        }
    }
}

fn closed() -> RuntimeError {
    RuntimeError::new(
        ErrorKind::Internal,
        "the journal writer is no longer running",
    )
}

/// The single task that owns the file. One writer is what makes a line atomic:
/// no two appends can interleave because there is only ever one in flight.
struct Writer {
    file: tokio::fs::File,
    config: JournalConfig,
    redactor: Arc<dyn Redactor>,
    dropped: Arc<AtomicU64>,
    dropped_seq_range: Arc<Mutex<Option<(u64, u64)>>>,
    failure: Arc<Mutex<Option<RuntimeError>>>,
    stats: JournalStats,
}

impl Writer {
    async fn run(mut self, mut receiver: mpsc::Receiver<JournalCommand>) {
        while let Some(command) = receiver.recv().await {
            match command {
                JournalCommand::Record(event) => {
                    if let Err(err) = self.append(*event).await {
                        self.remember_failure(&err);
                        // A failing disk must not wedge the runtime: keep
                        // draining so `observe` never blocks, and report the
                        // failure where an operator will see it.
                        tracing::error!(%err, "journal record was not written");
                    }
                }
                JournalCommand::Marker { record, reply } => {
                    let result = match self.append_marker(*record).await {
                        Ok(()) => self.sync(None).await,
                        Err(error) => Err(error),
                    };
                    if let Err(error) = &result {
                        self.remember_failure(error);
                    }
                    let _ = reply.send(result);
                }
                JournalCommand::Flush { before_seq, reply } => {
                    let _ = reply.send(self.sync(before_seq).await);
                }
                JournalCommand::Shutdown(reply) => {
                    let result = self.sync(None).await.map(|()| self.stats);
                    let _ = reply.send(result);
                    return;
                }
            }
        }
    }

    /// Writes one event, preceded by an overflow marker when records were lost
    /// since the previous write.
    async fn append(&mut self, event: EventEnvelope) -> Result<(), RuntimeError> {
        self.account_for_drops(Some(event.seq)).await?;

        let seq = event.seq;
        let id = event.id.clone();
        let session = event.session.clone();
        let turn = event.turn.clone();
        let timestamp = event.timestamp;

        let rendered = self.render(JournalRecord::Event { event })?;
        if rendered.len() > self.config.max_record_bytes {
            // Truncating the JSON would produce a line no reader can parse, so
            // the whole record is replaced by a marker that still attributes
            // the gap to a turn and a sequence position.
            let marker = self.render(JournalRecord::Oversized {
                seq,
                id,
                session,
                turn,
                timestamp,
                event: discriminant_of(&rendered),
                bytes: rendered.len(),
            })?;
            self.write_line(marker).await?;
            self.stats.oversized += 1;
            return Ok(());
        }

        self.write_line(rendered).await?;
        self.stats.written += 1;
        Ok(())
    }

    async fn append_marker(&mut self, record: JournalRecord) -> Result<(), RuntimeError> {
        self.account_for_drops(None).await?;
        let rendered = self.render(record)?;
        if rendered.len() > self.config.max_record_bytes {
            return Err(RuntimeError::new(
                ErrorKind::Limit,
                format!(
                    "Smith journal marker exceeded the {} byte record bound",
                    self.config.max_record_bytes
                ),
            ));
        }
        self.write_line(rendered).await
    }

    async fn account_for_drops(&mut self, before_seq: Option<u64>) -> Result<(), RuntimeError> {
        let count = self.dropped.swap(0, Ordering::Relaxed);
        if count == 0 {
            return Ok(());
        }
        self.stats.dropped += count;
        // Draining the range here, right beside the counter reset above, is
        // best-effort rather than linearizable with it — nothing locks the
        // two together against a concurrent `observe` landing in between —
        // but that only risks a rejected seq attributed to the marker one
        // burst early, never a wrong or missing range for a burst that
        // actually happened.
        let range = self
            .dropped_seq_range
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let (lowest_dropped_seq, highest_dropped_seq) = match range {
            Some((lowest, highest)) => (Some(lowest), Some(highest)),
            None => (None, None),
        };
        let marker = self.render(JournalRecord::Dropped {
            count,
            before_seq,
            lowest_dropped_seq,
            highest_dropped_seq,
        })?;
        self.write_line(marker).await
    }

    /// Serializes a record and applies redaction, returning the exact bytes
    /// that will become one line.
    fn render(&self, record: JournalRecord) -> Result<String, RuntimeError> {
        let mut value = serde_json::to_value(JournalLine::new(record)).map_err(|err| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("a journal record could not be serialized: {err}"),
            )
        })?;
        self.redactor.redact(&mut value);
        serde_json::to_string(&value).map_err(|err| {
            RuntimeError::new(
                ErrorKind::Serialization,
                format!("a redacted journal record could not be serialized: {err}"),
            )
        })
    }

    /// Issues exactly one write of `line + "\n"`, so a reader never observes a
    /// record without its terminator except after a crash.
    async fn write_line(&mut self, mut line: String) -> Result<(), RuntimeError> {
        line.push('\n');
        self.file.write_all(line.as_bytes()).await.map_err(|err| {
            RuntimeError::new(ErrorKind::Internal, format!("journal write failed: {err}"))
        })
    }

    async fn sync(&mut self, before_seq: Option<u64>) -> Result<(), RuntimeError> {
        // Drops observed with nothing after them still belong in the file:
        // otherwise a burst at the very end of a session would vanish.
        if let Err(error) = self.account_for_drops(before_seq).await {
            self.remember_failure(&error);
        }
        if let Err(error) = self.file.flush().await.map_err(|err| {
            RuntimeError::new(ErrorKind::Internal, format!("journal flush failed: {err}"))
        }) {
            self.remember_failure(&error);
        }
        if let Err(error) = self.file.sync_data().await.map_err(|err| {
            RuntimeError::new(ErrorKind::Internal, format!("journal sync failed: {err}"))
        }) {
            self.remember_failure(&error);
        }
        self.failure
            .lock()
            .expect("journal failure state poisoned")
            .clone()
            .map_or(Ok(()), Err)
    }

    fn remember_failure(&self, error: &RuntimeError) {
        let mut failure = self.failure.lock().expect("journal failure state poisoned");
        if failure.is_none() {
            *failure = Some(error.clone());
        }
    }
}

/// Reads the event discriminant back out of a rendered event line.
///
/// The runtime tags [`agent_runtime_core::event::RuntimeEvent`] with an
/// `event` field, so the label comes from the canonical vocabulary rather than
/// from a Smith-local mapping that could drift from it.
fn discriminant_of(rendered: &str) -> String {
    serde_json::from_str::<Value>(rendered)
        .ok()
        .and_then(|value| {
            value
                .get("event")?
                .get("payload")?
                .get("event")?
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

mod recovery;
mod redaction;

pub use recovery::{
    JournalReconciliation, JournalRecovery, TruncatedTail, read_journal,
    reconcile_nonterminal_journal,
};
use recovery::{
    repair_incomplete_tail, validate_ephemeral_interruption, validate_monitor_id, validate_task_id,
};
pub use redaction::{DefaultRedactor, KeepEverything, Redactor};

#[cfg(test)]
mod tests;
