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
//!   [`JournalRecord::Dropped`] marker naming how many were lost and which
//!   sequence number resumes the record. A reader can always tell a complete
//!   journal from a lossy one.
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
use agent_runtime_core::ids::{EventId, SessionId, TurnId};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::store::Secret;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};

use crate::session::SessionPaths;

/// The schema version stamped on every journal line.
///
/// Present on markers as well as events so a reader can identify what it is
/// parsing from the first field, without inferring it from the file name.
pub const JOURNAL_SCHEMA_VERSION: u32 = 1;

/// The replacement written in place of a redacted value.
const REDACTED: &str = "[redacted]";

/// Credential words matched anywhere in a key once separators and case are
/// removed, so `apiKey`, `API_KEY`, and `x-api-key` all match.
///
/// Every entry is specific enough that it cannot occur inside a benign
/// identifier — which is why the plain word `token` is *not* here: the shared
/// event vocabulary is full of legitimate `*_tokens` counters, and redacting
/// `reserved_tokens` would corrupt canonical usage data to protect nothing.
const SENSITIVE_KEY_NEEDLES: &[&str] = &[
    "accesskey",
    "accesstoken",
    "apikey",
    "authorization",
    "authtoken",
    "bearer",
    "credential",
    "passphrase",
    "password",
    "privatekey",
    "refreshtoken",
    "secret",
    "sessionkey",
];

/// Credential words matched only as a whole word of a key, where the plural
/// counters above cannot collide with them: `token` and `key` are sensitive,
/// `reserved_tokens` and `keyframes` are not.
const SENSITIVE_KEY_WORDS: &[&str] = &["auth", "key", "token"];

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
/// Only [`JournalRecord::Event`] carries runtime history; the other two are
/// markers describing something the journal could not record faithfully. They
/// are distinguishable by the `record` tag so a reader can never mistake a
/// marker for a shared runtime event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
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
    },
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
    /// absorb a burst without ever making the runtime wait on a disk.
    pub queue_capacity: usize,
    /// The largest serialized record written verbatim.
    pub max_record_bytes: usize,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 1024,
            // Large enough for a realistic tool-call payload, small enough
            // that one pathological event cannot dominate a session's file.
            max_record_bytes: 64 * 1024,
        }
    }
}

/// The journal's redaction seam.
///
/// Runs over the fully serialized line, in the writer task, before a single
/// byte reaches the file. Placing it here rather than at each call site means
/// a new event variant cannot introduce a leak by forgetting to opt in.
pub trait Redactor: Send + Sync + std::fmt::Debug {
    /// Rewrites `line` in place, removing anything that must not be persisted.
    fn redact(&self, line: &mut Value);
}

/// A redactor that keeps everything. Useful only where the caller has already
/// proven the event stream carries no credentials.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeepEverything;

impl Redactor for KeepEverything {
    fn redact(&self, _line: &mut Value) {}
}

/// Smith's default redaction policy.
///
/// Two independent rules, because a secret can enter the journal two different
/// ways. A *structural* rule replaces the value of any credential-shaped key,
/// which covers tool arguments and provider metadata whose shape Smith does
/// not control. A *literal* rule replaces any registered secret value wherever
/// it appears, including inside free text, which covers a model echoing a key
/// back into an assistant message.
#[derive(Clone, Default)]
pub struct DefaultRedactor {
    secrets: Arc<RwLock<Vec<String>>>,
}

impl DefaultRedactor {
    /// A redactor with the structural rule only.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a known secret literal to scrub wherever it appears.
    ///
    /// Resolved credentials are the intended input: the host knows the exact
    /// string it handed the provider, so the journal can be made to never
    /// contain it. An empty value is ignored, since it would match everywhere.
    pub fn with_secret(self, secret: impl Into<String>) -> Self {
        self.register_value(secret.into());
        self
    }

    /// Registers a resolved credential without exposing its value at the call
    /// site. Clones share the same registry, allowing the factory to resolve a
    /// credential after the host has already injected persistence adapters.
    pub fn register_secret(&self, secret: &Secret) {
        self.register_value(secret.expose().to_owned());
    }

    fn register_value(&self, secret: String) {
        if !secret.is_empty() {
            let mut secrets = self
                .secrets
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !secrets.contains(&secret) {
                secrets.push(secret);
            }
        }
    }

    fn scrub(&self, value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map.iter_mut() {
                    if is_sensitive_key(key) {
                        *child = Value::String(REDACTED.to_owned());
                    } else {
                        self.scrub(child);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.scrub(item);
                }
            }
            Value::String(text) => {
                let secrets = self
                    .secrets
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for secret in secrets.iter() {
                    if text.contains(secret.as_str()) {
                        *text = text.replace(secret.as_str(), REDACTED);
                    }
                }
            }
            _ => {}
        }
    }
}

impl std::fmt::Debug for DefaultRedactor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let registered = self
            .secrets
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        f.debug_struct("DefaultRedactor")
            .field("registered_secrets", &registered)
            .finish()
    }
}

impl Redactor for DefaultRedactor {
    fn redact(&self, line: &mut Value) {
        self.scrub(line);
    }
}

/// Whether a key's *value* must never be persisted.
///
/// Deliberately biased towards over-redaction: a journal that hides one field
/// it did not have to is recoverable, a journal that persists one credential
/// is not. The one place that bias is held back is the `*_tokens` family, where
/// over-redaction would destroy the usage accounting the journal exists to
/// preserve.
fn is_sensitive_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    let compact: String = lowered
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if SENSITIVE_KEY_NEEDLES
        .iter()
        .any(|needle| compact.contains(needle))
    {
        return true;
    }
    lowered
        .split(|character: char| !character.is_ascii_alphanumeric())
        .any(|word| SENSITIVE_KEY_WORDS.contains(&word))
}

/// What the writer task is asked to do. Records and control messages share one
/// channel so a flush can never be reordered ahead of the records it is
/// supposed to wait for.
enum JournalCommand {
    /// Append one event. Boxed to keep the queued item small.
    Record(Box<EventEnvelope>),
    /// Sync everything written so far, then acknowledge.
    Flush(oneshot::Sender<Result<(), RuntimeError>>),
    /// Sync, stop, and report.
    Shutdown(oneshot::Sender<Result<JournalStats, RuntimeError>>),
}

/// An [`EventObserver`] that appends canonical events to a JSON Lines file.
#[derive(Debug)]
pub struct EventJournal {
    commands: mpsc::Sender<JournalCommand>,
    dropped: Arc<AtomicU64>,
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
        let writer = Writer {
            file,
            config,
            redactor,
            dropped: Arc::clone(&dropped),
            stats: JournalStats::default(),
        };
        tokio::spawn(writer.run(receiver));

        Ok(Self {
            commands,
            dropped,
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

    /// Waits until every record queued before this call has been written and
    /// synced.
    pub async fn flush(&self) -> Result<(), RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(JournalCommand::Flush(reply))
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
    stats: JournalStats,
}

impl Writer {
    async fn run(mut self, mut receiver: mpsc::Receiver<JournalCommand>) {
        while let Some(command) = receiver.recv().await {
            match command {
                JournalCommand::Record(event) => {
                    if let Err(err) = self.append(*event).await {
                        // A failing disk must not wedge the runtime: keep
                        // draining so `observe` never blocks, and report the
                        // failure where an operator will see it.
                        tracing::error!(%err, "journal record was not written");
                    }
                }
                JournalCommand::Flush(reply) => {
                    let _ = reply.send(self.sync().await);
                }
                JournalCommand::Shutdown(reply) => {
                    let result = self.sync().await.map(|()| self.stats);
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

    async fn account_for_drops(&mut self, before_seq: Option<u64>) -> Result<(), RuntimeError> {
        let count = self.dropped.swap(0, Ordering::Relaxed);
        if count == 0 {
            return Ok(());
        }
        self.stats.dropped += count;
        let marker = self.render(JournalRecord::Dropped { count, before_seq })?;
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

    async fn sync(&mut self) -> Result<(), RuntimeError> {
        // Drops observed with nothing after them still belong in the file:
        // otherwise a burst at the very end of a session would vanish.
        self.account_for_drops(None).await?;
        self.file.flush().await.map_err(|err| {
            RuntimeError::new(ErrorKind::Internal, format!("journal flush failed: {err}"))
        })?;
        self.file.sync_data().await.map_err(|err| {
            RuntimeError::new(ErrorKind::Internal, format!("journal sync failed: {err}"))
        })
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
async fn repair_incomplete_tail(path: &Path) -> Result<Option<TruncatedTail>, RuntimeError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        });
        let json = serde_json::to_value(&line).expect("serializable");
        assert_eq!(json["schema_version"], JOURNAL_SCHEMA_VERSION);
        assert_eq!(json["record"], "dropped");
        assert_eq!(json["count"], 3);
    }
}
