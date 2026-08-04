//! In-session attribution for mutating Smith tools.
//!
//! Exact `edit` calls retain bounded pre/post images in memory for conflict
//! checked undo. The persisted journal receives hashes and path metadata only;
//! arbitrary file contents and protected tool arguments are never serialized.
//! Shell mutations are marked ambiguous because observing a Git delta does not
//! prove that every concurrent byte belongs to the command.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::read_state::ReadRecorder;
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolOutcome, ToolSpec,
};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
const CHANGE_SCHEMA_VERSION: u32 = 1;

/// One exact or ambiguous mutation observed at a tool boundary.
#[derive(Debug, Clone)]
pub enum ToolMutation {
    /// An exact edit whose complete file images are retained in memory.
    Exact(EditMutation),
    /// A mutating tool whose complete ownership cannot be proven.
    Ambiguous {
        /// Tool call id.
        call_id: String,
        /// Tool name, never its protected arguments.
        tool: String,
    },
}

/// One exact edit.
#[derive(Debug, Clone)]
pub struct EditMutation {
    /// Tool call id.
    pub call_id: String,
    /// Canonical target path.
    pub path: PathBuf,
    /// `None` when Smith created the file.
    pub before: Option<Vec<u8>>,
    /// Complete post-image, or `None` when the operation removed the file.
    pub after: Option<Vec<u8>>,
    /// Hash of the pre-image or the absence marker.
    pub before_hash: String,
    /// Hash of the post-image.
    pub after_hash: String,
    /// Session recovery copy for an untracked removal, when applicable.
    pub recovery_path: Option<PathBuf>,
}

/// A completed turn's mutation attribution.
#[derive(Debug, Clone)]
pub struct TurnChangeSet {
    /// Monotonic in-session turn number.
    pub turn: u64,
    /// Mutations observed within the turn.
    pub mutations: Vec<ToolMutation>,
    /// Whether the set has already been undone.
    pub undone: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RedoDirection {
    ReapplyUndoneTurn,
    ReapplyRevertedChange,
}

impl TurnChangeSet {
    /// Whether every mutation has an exact reversible image.
    pub fn is_fully_attributable(&self) -> bool {
        !self.mutations.is_empty()
            && self
                .mutations
                .iter()
                .all(|mutation| matches!(mutation, ToolMutation::Exact(_)))
    }
}

#[derive(Debug, Default)]
struct State {
    active: bool,
    next_turn: u64,
    historical: bool,
    pending: Vec<ToolMutation>,
    completed: Vec<TurnChangeSet>,
    timeline: Vec<String>,
}

/// Shared mutation recorder installed around Smith's built-in tools.
#[derive(Debug)]
pub struct ChangeRecorder {
    state: Mutex<State>,
    journal: Option<PathBuf>,
}

impl ChangeRecorder {
    /// Creates a recorder with an optional metadata-only JSONL journal.
    pub fn new(journal: Option<PathBuf>) -> Self {
        let mut state = State::default();
        if let Some(path) = &journal
            && let Ok(contents) = std::fs::read_to_string(path)
        {
            for line in contents.lines() {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if let Some(turn) = value.get("turn").and_then(Value::as_u64) {
                    state.next_turn = state.next_turn.max(turn);
                }
                if let Some(label) = persisted_timeline_label(&value) {
                    state.timeline.push(label);
                }
                state.historical = true;
            }
        }
        Self {
            state: Mutex::new(state),
            journal,
        }
    }

    /// Starts a root turn.
    pub fn start_turn(&self) {
        let mut state = self.state.lock().expect("change recorder poisoned");
        state.active = true;
        state.pending.clear();
    }

    /// Completes the active turn and returns its change set, if it mutated.
    pub fn finish_turn(&self) -> Option<TurnChangeSet> {
        let set = {
            let mut state = self.state.lock().expect("change recorder poisoned");
            state.active = false;
            if state.pending.is_empty() {
                return None;
            }
            state.next_turn = state.next_turn.saturating_add(1);
            let turn = state.next_turn;
            let mutations = coalesce(std::mem::take(&mut state.pending));
            let set = TurnChangeSet {
                turn,
                mutations,
                undone: false,
            };
            state.completed.push(set.clone());
            set
        };
        self.persist(&JournalEntry::TurnCompleted(PersistedTurn::from(&set)));
        self.record_timeline(format!(
            "turn {} · {} · {} mutation(s)",
            set.turn,
            if set.is_fully_attributable() {
                "exact"
            } else {
                "ambiguous"
            },
            set.mutations.len()
        ));
        Some(set)
    }

    /// The newest completed change set.
    pub fn latest(&self) -> Option<TurnChangeSet> {
        self.state
            .lock()
            .expect("change recorder poisoned")
            .completed
            .last()
            .cloned()
    }

    /// Whether this resumed session has metadata-only historical attribution
    /// that cannot be safely reconstructed into file images.
    pub fn has_historical_records(&self) -> bool {
        self.state
            .lock()
            .expect("change recorder poisoned")
            .historical
    }

    /// Bounded metadata-only change and recovery timeline.
    pub fn timeline(&self) -> Vec<String> {
        let state = self.state.lock().expect("change recorder poisoned");
        state
            .timeline
            .iter()
            .rev()
            .take(100)
            .rev()
            .cloned()
            .collect()
    }

    /// Whether the latest live change set proves Smith ownership of `path`.
    pub fn latest_owns_path(&self, path: &Path) -> bool {
        self.latest().is_some_and(|set| {
            set.mutations
                .iter()
                .any(|mutation| matches!(mutation, ToolMutation::Exact(edit) if edit.path == path))
        })
    }

    /// Records an exact user-confirmed recovery operation so it can itself be
    /// restored with the same `/undo` conflict checks.
    pub fn record_recovery(
        &self,
        path: PathBuf,
        before: Option<Vec<u8>>,
        after: Option<Vec<u8>>,
        operation: &str,
        recovery_path: Option<PathBuf>,
    ) {
        self.start_turn();
        self.record(ToolMutation::Exact(EditMutation {
            call_id: format!("recovery:{operation}"),
            path,
            before_hash: hash(before.as_deref()),
            after_hash: hash(after.as_deref()),
            before,
            after,
            recovery_path,
        }));
        let _ = self.finish_turn();
    }

    /// A reverse-patch preview for the newest attributable turn.
    pub fn undo_preview(&self) -> Result<String, RuntimeError> {
        let set = self.latest().ok_or_else(|| {
            if self.has_historical_records() {
                unavailable(
                    "historical change records are visible but cannot be automatically undone after resume",
                )
            } else {
                unavailable("no Smith turn has attributable changes")
            }
        })?;
        if set.undone {
            return Err(unavailable(
                "the newest attributable turn was already undone",
            ));
        }
        if !set.is_fully_attributable() {
            return Err(unavailable(
                "the newest turn contains ambiguous shell or extension changes; use /diff and /revert",
            ));
        }
        let mut output = format!("Smith turn {}\n", set.turn);
        for mutation in &set.mutations {
            let ToolMutation::Exact(edit) = mutation else {
                unreachable!("checked above");
            };
            output.push_str(&format!(
                "\n--- current {}\n+++ restore {}\n",
                edit.path.display(),
                edit.path.display()
            ));
            output.push_str(&textual_reverse(edit));
        }
        let fingerprint = hash(Some(output.as_bytes()));
        self.persist(&JournalEntry::RecoveryRequest {
            operation: "undo",
            scope: "last-turn",
            fingerprint: &fingerprint,
            outcome: "previewed",
        });
        self.record_timeline(format!("undo previewed · turn {}", set.turn));
        Ok(output)
    }

    /// Journals cancellation of the current undo preview without file content.
    pub fn record_undo_cancelled(&self) {
        let fingerprint = self
            .latest()
            .filter(|set| !set.undone && set.is_fully_attributable())
            .map(|set| {
                let mut output = format!("Smith turn {}\n", set.turn);
                for mutation in &set.mutations {
                    if let ToolMutation::Exact(edit) = mutation {
                        output.push_str(&format!(
                            "\n--- current {}\n+++ restore {}\n",
                            edit.path.display(),
                            edit.path.display()
                        ));
                        output.push_str(&textual_reverse(edit));
                    }
                }
                hash(Some(output.as_bytes()))
            })
            .unwrap_or_else(|| "unavailable".to_owned());
        self.persist(&JournalEntry::RecoveryRequest {
            operation: "undo",
            scope: "last-turn",
            fingerprint: &fingerprint,
            outcome: "cancelled",
        });
        self.record_timeline("undo cancelled".to_owned());
    }

    /// Atomically restores every pre-image after exact post-image checks.
    pub fn undo_latest(&self) -> Result<(), RuntimeError> {
        let set = self
            .latest()
            .ok_or_else(|| unavailable("no Smith turn has attributable changes"))?;
        if set.undone || !set.is_fully_attributable() {
            return Err(unavailable(
                "the newest turn is not eligible for automatic undo",
            ));
        }
        let edits = set
            .mutations
            .iter()
            .filter_map(|mutation| match mutation {
                ToolMutation::Exact(edit) => Some(edit),
                ToolMutation::Ambiguous { .. } => None,
            })
            .collect::<Vec<_>>();

        for edit in &edits {
            let current = bounded_image(&edit.path)?;
            if hash(current.as_deref()) != edit.after_hash {
                self.persist(&JournalEntry::Recovery {
                    operation: "undo",
                    turn: set.turn,
                    outcome: "conflict",
                });
                return Err(unavailable(format!(
                    "undo refused: `{}` changed after Smith's turn; use /diff and /revert",
                    edit.path.display()
                )));
            }
        }

        let mut applied: Vec<&EditMutation> = Vec::new();
        for edit in &edits {
            let result = match &edit.before {
                Some(before) => atomic_write(&edit.path, before),
                None => std::fs::remove_file(&edit.path).map_err(io_error),
            };
            if let Err(error) = result {
                for prior in applied.into_iter().rev() {
                    match &prior.after {
                        Some(after) => {
                            let _ = atomic_write(&prior.path, after);
                        }
                        None => {
                            let _ = std::fs::remove_file(&prior.path);
                        }
                    }
                }
                self.persist(&JournalEntry::Recovery {
                    operation: "undo",
                    turn: set.turn,
                    outcome: "rolled_back",
                });
                return Err(error);
            }
            applied.push(edit);
        }

        let mut state = self.state.lock().expect("change recorder poisoned");
        if let Some(latest) = state.completed.last_mut() {
            latest.undone = true;
        }
        drop(state);
        self.persist(&JournalEntry::Recovery {
            operation: "undo",
            turn: set.turn,
            outcome: "applied",
        });
        self.record_timeline(format!("undo applied · turn {}", set.turn));
        Ok(())
    }

    /// Forward-patch preview for the newest successfully undone exact turn.
    pub fn redo_preview(&self) -> Result<String, RuntimeError> {
        let set = self
            .latest()
            .ok_or_else(|| unavailable("no exact redo candidate exists"))?;
        let direction = redo_direction(&set).ok_or_else(|| {
            unavailable(
                "no exact redo candidate exists; ambiguous shell changes are never redoable",
            )
        })?;
        let output = redo_preview_text(&set, direction);
        let fingerprint = hash(Some(output.as_bytes()));
        self.persist(&JournalEntry::RecoveryRequest {
            operation: "redo",
            scope: "last-undone-turn",
            fingerprint: &fingerprint,
            outcome: "previewed",
        });
        self.record_timeline(format!("redo previewed · turn {}", set.turn));
        Ok(output)
    }

    /// Journals cancellation of the current redo preview without file content.
    pub fn record_redo_cancelled(&self) {
        let fingerprint = self
            .latest()
            .and_then(|set| redo_direction(&set).map(|direction| (set, direction)))
            .map(|(set, direction)| redo_preview_text(&set, direction))
            .map(|preview| hash(Some(preview.as_bytes())))
            .unwrap_or_else(|| "unavailable".to_owned());
        self.persist(&JournalEntry::RecoveryRequest {
            operation: "redo",
            scope: "latest-exact-recovery",
            fingerprint: &fingerprint,
            outcome: "cancelled",
        });
        self.record_timeline("redo cancelled".to_owned());
    }

    /// Atomically reapplies every post-image after exact pre-image checks.
    pub fn redo_latest(&self) -> Result<(), RuntimeError> {
        let set = self
            .latest()
            .ok_or_else(|| unavailable("no exact redo candidate exists"))?;
        let direction = redo_direction(&set)
            .ok_or_else(|| unavailable("the newest change set is not eligible for exact redo"))?;
        let edits = set
            .mutations
            .iter()
            .filter_map(|mutation| match mutation {
                ToolMutation::Exact(edit) => Some(edit),
                ToolMutation::Ambiguous { .. } => None,
            })
            .collect::<Vec<_>>();
        for edit in &edits {
            let current = bounded_image(&edit.path)?;
            let expected_hash = match direction {
                RedoDirection::ReapplyUndoneTurn => &edit.before_hash,
                RedoDirection::ReapplyRevertedChange => &edit.after_hash,
            };
            if hash(current.as_deref()) != *expected_hash {
                self.persist(&JournalEntry::Recovery {
                    operation: "redo",
                    turn: set.turn,
                    outcome: "conflict",
                });
                self.record_timeline(format!("redo conflict · turn {}", set.turn));
                return Err(unavailable(format!(
                    "redo refused: `{}` changed after undo; use /diff and /timeline",
                    edit.path.display()
                )));
            }
        }

        let mut applied: Vec<&EditMutation> = Vec::new();
        for edit in &edits {
            let target = match direction {
                RedoDirection::ReapplyUndoneTurn => &edit.after,
                RedoDirection::ReapplyRevertedChange => &edit.before,
            };
            let result = match target {
                Some(after) => atomic_write(&edit.path, after),
                None => match std::fs::remove_file(&edit.path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(io_error(error)),
                },
            };
            if let Err(error) = result {
                for prior in applied.into_iter().rev() {
                    let prior_rollback = match direction {
                        RedoDirection::ReapplyUndoneTurn => &prior.before,
                        RedoDirection::ReapplyRevertedChange => &prior.after,
                    };
                    match prior_rollback {
                        Some(before) => {
                            let _ = atomic_write(&prior.path, before);
                        }
                        None => {
                            let _ = std::fs::remove_file(&prior.path);
                        }
                    }
                }
                self.persist(&JournalEntry::Recovery {
                    operation: "redo",
                    turn: set.turn,
                    outcome: "rolled_back",
                });
                self.record_timeline(format!("redo rolled back · turn {}", set.turn));
                return Err(error);
            }
            applied.push(edit);
        }
        let mut state = self.state.lock().expect("change recorder poisoned");
        if let Some(latest) = state.completed.last_mut() {
            latest.undone = matches!(direction, RedoDirection::ReapplyRevertedChange);
        }
        drop(state);
        self.persist(&JournalEntry::Recovery {
            operation: "redo",
            turn: set.turn,
            outcome: "applied",
        });
        self.record_timeline(format!("redo applied · turn {}", set.turn));
        Ok(())
    }

    fn record(&self, mutation: ToolMutation) {
        let persisted = PersistedMutation::from(&mutation);
        let mut state = self.state.lock().expect("change recorder poisoned");
        if state.active {
            state.pending.push(mutation);
        }
        drop(state);
        self.persist(&JournalEntry::ToolBoundary(persisted));
    }

    fn persist(&self, entry: &JournalEntry<'_>) {
        let Some(path) = &self.journal else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(mut line) = serde_json::to_vec(entry) else {
            return;
        };
        line.push(b'\n');
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(&line);
            let _ = file.sync_data();
        }
    }

    fn record_timeline(&self, entry: String) {
        let mut state = self.state.lock().expect("change recorder poisoned");
        state.timeline.push(entry);
        if state.timeline.len() > 200 {
            let remove = state.timeline.len() - 200;
            state.timeline.drain(..remove);
        }
    }

    /// Journals a selective recovery request or outcome without file content.
    pub fn record_revert_event(&self, scope: &str, fingerprint: &str, outcome: &str) {
        self.persist(&JournalEntry::RecoveryRequest {
            operation: "revert",
            scope,
            fingerprint,
            outcome,
        });
        self.record_timeline(format!("revert {outcome} · {scope}"));
    }
}

fn coalesce(mutations: Vec<ToolMutation>) -> Vec<ToolMutation> {
    let mut combined: Vec<ToolMutation> = Vec::new();
    for mutation in mutations {
        match mutation {
            ToolMutation::Exact(later) => {
                if let Some(ToolMutation::Exact(earlier)) =
                    combined.iter_mut().find(|candidate| {
                        matches!(candidate, ToolMutation::Exact(edit) if edit.path == later.path)
                    })
                {
                    earlier.call_id = later.call_id;
                    earlier.after = later.after;
                    earlier.after_hash = later.after_hash;
                } else {
                    combined.push(ToolMutation::Exact(later));
                }
            }
            ambiguous => combined.push(ambiguous),
        }
    }
    combined
}

/// Wraps built-in tools with mutation attribution.
pub fn observed_tools(recorder: Arc<ChangeRecorder>) -> Vec<Arc<dyn Tool>> {
    observe(Some(recorder), ReadRecorder::new())
}

/// Wraps the built-in tools with whatever session state they need.
///
/// Mutation attribution is optional — a caller may not want undo — but read
/// state never is, because `edit`'s `overwrite` and `delete` are refused
/// without it.
pub(crate) fn observe(
    recorder: Option<Arc<ChangeRecorder>>,
    reads: Arc<ReadRecorder>,
) -> Vec<Arc<dyn Tool>> {
    crate::built_in()
        .into_iter()
        .map(|inner| {
            Arc::new(ObservedTool {
                inner,
                recorder: recorder.clone(),
                reads: reads.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

#[derive(Debug)]
struct ObservedTool {
    inner: Arc<dyn Tool>,
    recorder: Option<Arc<ChangeRecorder>>,
    reads: Arc<ReadRecorder>,
}

impl ObservedTool {
    fn record(&self, mutation: ToolMutation) {
        if let Some(recorder) = &self.recorder {
            recorder.record(mutation);
        }
    }
}

#[async_trait]
impl Tool for ObservedTool {
    fn spec(&self) -> ToolSpec {
        self.inner.spec()
    }

    async fn prepare(
        &self,
        arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let prepared = self.inner.prepare(arguments, ctx).await?;
        // Enforced after the inner prepare so the operation has been validated
        // and normalized, and so a doomed destructive call never reaches the
        // approval prompt. `edit` owns the rule; this supplies the state.
        if prepared.tool() == "edit"
            && let Some(defect) = crate::edit::read_state_defect(prepared.arguments(), &self.reads)
        {
            let display = prepared
                .arguments()
                .get("path")
                .and_then(Value::as_str)
                .map_or_else(
                    || "the target".to_owned(),
                    |path| display_relative(ctx, path),
                );
            return Err(RuntimeError::new(ErrorKind::Tool, defect.message(&display)));
        }
        Ok(prepared)
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let tool = prepared.tool().to_owned();
        let call_id = prepared.call_id().as_str().to_owned();
        let mutates = prepared.effects().mutates();
        let target = |wanted: &str| {
            (tool == wanted)
                .then(|| {
                    prepared
                        .arguments()
                        .get("path")
                        .and_then(Value::as_str)
                        .and_then(|path| crate::support::resolve(ctx, path).ok())
                })
                .flatten()
        };
        let edit_path = target("edit");
        let read_path = target("read");
        let before = match &edit_path {
            Some(path) => bounded_image(path),
            None => Ok(None),
        };
        let outcome = self.inner.invoke(prepared, ctx).await;
        if tool == "read" {
            self.observe_read(&outcome, &read_path);
        }
        match (&outcome, edit_path, before) {
            (Ok(outcome), Some(path), Ok(before)) if !outcome.is_error => {
                // `bounded_image` returns `None` for a path that is not there,
                // which after a successful call is exactly what a completed
                // delete looks like. Both images absent is the ambiguous case:
                // nothing existed before and nothing exists now.
                match (bounded_image(&path), before.is_some()) {
                    (Ok(Some(after)), _) => self.record(ToolMutation::Exact(EditMutation {
                        call_id: call_id.clone(),
                        path,
                        before_hash: hash(before.as_deref()),
                        after_hash: hash(Some(&after)),
                        before,
                        after: Some(after),
                        recovery_path: None,
                    })),
                    (Ok(None), true) => self.record(ToolMutation::Exact(EditMutation {
                        call_id: call_id.clone(),
                        path,
                        before_hash: hash(before.as_deref()),
                        after_hash: hash(None),
                        before,
                        after: None,
                        recovery_path: None,
                    })),
                    _ => self.record(ToolMutation::Ambiguous {
                        call_id: call_id.clone(),
                        tool: tool.clone(),
                    }),
                }
            }
            (_, Some(_), Err(_)) if tool == "edit" => {
                self.record(ToolMutation::Ambiguous {
                    call_id: call_id.clone(),
                    tool: tool.clone(),
                });
            }
            (_, _, _) if mutates && tool != "edit" => {
                self.record(ToolMutation::Ambiguous { call_id, tool });
            }
            _ => {}
        }
        outcome
    }
}

impl ObservedTool {
    /// Records a completed read, and whether it showed the whole file.
    ///
    /// "Whole file" is derived from the outcome the read actually produced
    /// rather than from the requested arguments: a `limit` larger than the file
    /// is a full view, and a caller that omits both is only a full view because
    /// the default happened to cover it.
    fn observe_read(&self, outcome: &Result<ToolOutcome, RuntimeError>, path: &Option<PathBuf>) {
        let (Ok(outcome), Some(path)) = (outcome, path) else {
            return;
        };
        if outcome.is_error {
            return;
        }
        let total = outcome.value.get("lines").and_then(Value::as_u64);
        let shown = outcome
            .value
            .get("shown")
            .and_then(Value::as_array)
            .and_then(|range| match range.as_slice() {
                [first, last] => Some((first.as_u64()?, last.as_u64()?)),
                _ => None,
            });
        let full = matches!((total, shown), (Some(total), Some((1, last))) if last == total);
        self.reads.record(path.clone(), full);
    }
}

/// Renders a prepared canonical path the way the tools themselves would.
fn display_relative(ctx: &PreparationContext, canonical: &str) -> String {
    std::path::Path::new(canonical)
        .strip_prefix(ctx.workspace.root())
        .map_or_else(|_| canonical.to_owned(), |path| path.display().to_string())
}

fn bounded_image(path: &Path) -> Result<Option<Vec<u8>>, RuntimeError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_IMAGE_BYTES => Err(unavailable(
            "edited file exceeds the attribution image limit",
        )),
        Ok(_) => std::fs::read(path).map(Some).map_err(io_error),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

fn hash(bytes: Option<&[u8]>) -> String {
    let mut digest = Sha256::new();
    match bytes {
        Some(bytes) => {
            digest.update(b"present\0");
            digest.update(bytes);
        }
        None => digest.update(b"absent\0"),
    }
    format!("{:x}", digest.finalize())
}

fn textual_reverse(edit: &EditMutation) -> String {
    let before = edit
        .before
        .as_deref()
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    let after = edit
        .after
        .as_deref()
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    let mut output = String::new();
    for line in after.lines() {
        output.push_str(&format!("-{line}\n"));
    }
    for line in before.lines() {
        output.push_str(&format!("+{line}\n"));
    }
    output
}

fn textual_forward(edit: &EditMutation) -> String {
    let before = edit
        .before
        .as_deref()
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    let after = edit
        .after
        .as_deref()
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    let mut output = String::new();
    for line in before.lines() {
        output.push_str(&format!("-{line}\n"));
    }
    for line in after.lines() {
        output.push_str(&format!("+{line}\n"));
    }
    output
}

fn redo_direction(set: &TurnChangeSet) -> Option<RedoDirection> {
    if !set.is_fully_attributable() {
        return None;
    }
    let is_revert = set.mutations.iter().all(|mutation| {
        matches!(
            mutation,
            ToolMutation::Exact(edit) if edit.call_id == "recovery:revert"
        )
    });
    if is_revert {
        (!set.undone).then_some(RedoDirection::ReapplyRevertedChange)
    } else {
        set.undone.then_some(RedoDirection::ReapplyUndoneTurn)
    }
}

fn redo_preview_text(set: &TurnChangeSet, direction: RedoDirection) -> String {
    let mut output = format!("Smith turn {} redo\n", set.turn);
    for mutation in &set.mutations {
        let ToolMutation::Exact(edit) = mutation else {
            unreachable!("redo direction requires exact mutations");
        };
        output.push_str(&format!(
            "\n--- current {}\n+++ reapply {}\n",
            edit.path.display(),
            edit.path.display()
        ));
        match direction {
            RedoDirection::ReapplyUndoneTurn => output.push_str(&textual_forward(edit)),
            RedoDirection::ReapplyRevertedChange => output.push_str(&textual_reverse(edit)),
        }
    }
    output
}

fn persisted_timeline_label(value: &Value) -> Option<String> {
    let record = value.get("record")?.as_str()?;
    match record {
        "turn_completed" => Some(format!(
            "turn {} · persisted attribution",
            value
                .get("turn")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        )),
        "recovery" => Some(format!(
            "{} {} · turn {}",
            value
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or("recovery"),
            value
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("recorded"),
            value
                .get("turn")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        )),
        "recovery_request" => Some(format!(
            "{} {} · {}",
            value
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or("recovery"),
            value
                .get("outcome")
                .and_then(Value::as_str)
                .unwrap_or("recorded"),
            value
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("unknown scope")
        )),
        _ => None,
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    let parent = path
        .parent()
        .ok_or_else(|| unavailable("recovery target has no parent directory"))?;
    let temporary = parent.join(format!(".smith-recovery-{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, contents).map_err(io_error)?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        io_error(error)
    })
}

#[derive(Debug, Serialize)]
#[serde(tag = "record", rename_all = "snake_case")]
enum JournalEntry<'a> {
    ToolBoundary(PersistedMutation),
    TurnCompleted(PersistedTurn),
    Recovery {
        operation: &'a str,
        turn: u64,
        outcome: &'a str,
    },
    RecoveryRequest {
        operation: &'a str,
        scope: &'a str,
        fingerprint: &'a str,
        outcome: &'a str,
    },
}

#[derive(Debug, Serialize)]
struct PersistedTurn {
    schema_version: u32,
    turn: u64,
    fully_attributable: bool,
    mutations: Vec<PersistedMutation>,
}

impl From<&TurnChangeSet> for PersistedTurn {
    fn from(set: &TurnChangeSet) -> Self {
        Self {
            schema_version: CHANGE_SCHEMA_VERSION,
            turn: set.turn,
            fully_attributable: set.is_fully_attributable(),
            mutations: set.mutations.iter().map(PersistedMutation::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "attribution", rename_all = "snake_case")]
enum PersistedMutation {
    ExactEdit {
        schema_version: u32,
        call_id: String,
        path: String,
        before_hash: String,
        after_hash: String,
        recovery_path: Option<String>,
    },
    Ambiguous {
        schema_version: u32,
        call_id: String,
        tool: String,
    },
}

impl From<&ToolMutation> for PersistedMutation {
    fn from(mutation: &ToolMutation) -> Self {
        match mutation {
            ToolMutation::Exact(edit) => Self::ExactEdit {
                schema_version: CHANGE_SCHEMA_VERSION,
                call_id: edit.call_id.clone(),
                path: edit.path.to_string_lossy().into_owned(),
                before_hash: edit.before_hash.clone(),
                after_hash: edit.after_hash.clone(),
                recovery_path: edit
                    .recovery_path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
            },
            ToolMutation::Ambiguous { call_id, tool } => Self::Ambiguous {
                schema_version: CHANGE_SCHEMA_VERSION,
                call_id: call_id.clone(),
                tool: tool.clone(),
            },
        }
    }
}

fn io_error(error: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::new(ErrorKind::Workspace, error.to_string())
}

fn unavailable(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(ErrorKind::Workspace, message)
}

#[cfg(test)]
mod observed_session {
    use super::*;
    use crate::testing::{call, context, text_of};
    use serde_json::json;

    /// A composed session: the real wrapper, the real recorders.
    fn session() -> (tempfile::TempDir, Vec<Arc<dyn Tool>>, InvocationContext) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let ctx = context(dir.path());
        let tools = observe(
            Some(Arc::new(ChangeRecorder::new(None))),
            ReadRecorder::new(),
        );
        (dir, tools, ctx)
    }

    async fn read_fully(tools: &[Arc<dyn Tool>], ctx: &InvocationContext, path: &str) {
        call(tools, "read", json!({ "path": path }), ctx)
            .await
            .expect("the read succeeds");
    }

    #[tokio::test]
    async fn overwriting_an_unread_file_is_refused() {
        let (dir, tools, ctx) = session();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");

        let error = call(
            &tools,
            "edit",
            json!({"path": "a.rs", "operation": "overwrite", "new_string": "gone\n"}),
            &ctx,
        )
        .await
        .expect_err("an unread file cannot be overwritten");

        assert!(error.to_string().contains("has not been read"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).expect("still there"),
            "fn a() {}\n"
        );
    }

    #[tokio::test]
    async fn a_partial_read_does_not_authorize_an_overwrite() {
        let (dir, tools, ctx) = session();
        let body: String = (1..=50).map(|n| format!("line {n}\n")).collect();
        std::fs::write(dir.path().join("long.txt"), &body).expect("seed");

        call(
            &tools,
            "read",
            json!({"path": "long.txt", "offset": 1, "limit": 5}),
            &ctx,
        )
        .await
        .expect("the partial read succeeds");

        let error = call(
            &tools,
            "edit",
            json!({"path": "long.txt", "operation": "overwrite", "new_string": "short\n"}),
            &ctx,
        )
        .await
        .expect_err("a window does not authorize replacing the whole file");

        assert!(error.to_string().contains("only read in part"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("long.txt")).expect("still there"),
            body
        );
    }

    #[tokio::test]
    async fn an_external_change_invalidates_the_read() {
        let (dir, tools, ctx) = session();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "fn a() {}\n").expect("seed");
        read_fully(&tools, &ctx, "a.rs").await;

        // The user edits the file in their own editor.
        std::fs::write(&path, "fn a() { work_in_progress(); }\n").expect("user edit");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| {
                file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(1))
            })
            .expect("retouch");

        let error = call(
            &tools,
            "edit",
            json!({"path": "a.rs", "operation": "overwrite", "new_string": "clobbered\n"}),
            &ctx,
        )
        .await
        .expect_err("a file changed since the read cannot be overwritten");

        assert!(
            error.to_string().contains("changed since it was read"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            "fn a() { work_in_progress(); }\n",
            "the user's work must survive"
        );
    }

    #[tokio::test]
    async fn a_full_read_authorizes_overwrite_and_delete() {
        let (dir, tools, ctx) = session();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");
        read_fully(&tools, &ctx, "a.rs").await;

        let outcome = call(
            &tools,
            "edit",
            json!({"path": "a.rs", "operation": "overwrite", "new_string": "fn b() {}\n"}),
            &ctx,
        )
        .await
        .expect("the overwrite runs");
        assert!(!outcome.is_error, "{}", text_of(&outcome));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).expect("rewritten"),
            "fn b() {}\n"
        );

        // The overwrite changed the file, so the earlier read no longer proves
        // anything about it — exactly the staleness rule, applied to ourselves.
        read_fully(&tools, &ctx, "a.rs").await;
        let outcome = call(
            &tools,
            "edit",
            json!({"path": "a.rs", "operation": "delete"}),
            &ctx,
        )
        .await
        .expect("the delete runs");
        assert!(!outcome.is_error, "{}", text_of(&outcome));
        assert!(!dir.path().join("a.rs").exists());
    }

    #[tokio::test]
    async fn exact_replacement_needs_no_prior_read() {
        let (dir, tools, ctx) = session();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");

        let outcome = call(
            &tools,
            "edit",
            json!({"path": "a.rs", "old_string": "fn a", "new_string": "fn b"}),
            &ctx,
        )
        .await
        .expect("an exact replacement proves its own currency");
        assert!(!outcome.is_error, "{}", text_of(&outcome));
    }

    #[tokio::test]
    async fn a_deleted_file_is_recorded_with_its_pre_image() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let ctx = context(dir.path());
        let recorder = Arc::new(ChangeRecorder::new(None));
        let tools = observe(Some(recorder.clone()), ReadRecorder::new());
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("seed");

        recorder.start_turn();
        read_fully(&tools, &ctx, "a.rs").await;
        call(
            &tools,
            "edit",
            json!({"path": "a.rs", "operation": "delete"}),
            &ctx,
        )
        .await
        .expect("the delete runs");
        let set = recorder.finish_turn().expect("a change set");

        assert!(
            set.is_fully_attributable(),
            "a delete must be exactly attributed so it can be undone: {set:?}"
        );
        let [ToolMutation::Exact(edit)] = set.mutations.as_slice() else {
            panic!("expected one exact mutation: {set:?}");
        };
        assert_eq!(edit.before.as_deref(), Some(b"fn a() {}\n".as_slice()));
        assert_eq!(edit.after, None, "a removed file has no post-image");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(path: &Path, before: Option<&[u8]>, after: &[u8]) -> ToolMutation {
        ToolMutation::Exact(EditMutation {
            call_id: "call-1".to_owned(),
            path: path.to_path_buf(),
            before: before.map(<[u8]>::to_vec),
            after: Some(after.to_vec()),
            before_hash: hash(before),
            after_hash: hash(Some(after)),
            recovery_path: None,
        })
    }

    #[test]
    fn exact_turn_previews_and_undoes_after_postimage_validation() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("file.txt");
        std::fs::write(&path, b"after\n").expect("after");
        let recorder = ChangeRecorder::new(None);
        recorder.start_turn();
        recorder.record(exact(&path, Some(b"before\n"), b"after\n"));
        let set = recorder.finish_turn().expect("set");
        assert!(set.is_fully_attributable());
        assert!(recorder.undo_preview().expect("preview").contains("-after"));
        recorder.undo_latest().expect("undo");
        assert_eq!(std::fs::read(&path).expect("read"), b"before\n");
        assert!(recorder.undo_latest().is_err(), "repeated undo must refuse");
    }

    #[test]
    fn concurrent_edit_refuses_without_touching_any_path() {
        let dir = tempfile::tempdir().expect("temp");
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        std::fs::write(&first, b"after one\n").expect("first");
        std::fs::write(&second, b"after two\n").expect("second");
        let recorder = ChangeRecorder::new(None);
        recorder.start_turn();
        recorder.record(exact(&first, Some(b"before one\n"), b"after one\n"));
        recorder.record(exact(&second, Some(b"before two\n"), b"after two\n"));
        recorder.finish_turn();

        std::fs::write(&second, b"user edit\n").expect("edit");
        assert!(recorder.undo_latest().is_err());
        assert_eq!(std::fs::read(&first).expect("first"), b"after one\n");
        assert_eq!(std::fs::read(&second).expect("second"), b"user edit\n");
    }

    #[test]
    fn an_ambiguous_mutation_blocks_automatic_undo() {
        let recorder = ChangeRecorder::new(None);
        recorder.start_turn();
        recorder.record(ToolMutation::Ambiguous {
            call_id: "shell-1".to_owned(),
            tool: "shell".to_owned(),
        });
        let set = recorder.finish_turn().expect("set");
        assert!(!set.is_fully_attributable());
        assert!(
            recorder
                .undo_preview()
                .unwrap_err()
                .message
                .contains("ambiguous")
        );
    }

    #[test]
    fn persisted_metadata_contains_hashes_but_not_file_contents() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("file.txt");
        let journal = dir.path().join("changes.jsonl");
        let recorder = ChangeRecorder::new(Some(journal.clone()));
        recorder.start_turn();
        recorder.record(exact(
            &path,
            Some(b"secret-before-value"),
            b"secret-after-value",
        ));
        recorder.finish_turn();
        let stored = std::fs::read_to_string(journal).expect("journal");
        assert!(stored.contains("before_hash"));
        assert!(!stored.contains("secret-before-value"));
        assert!(!stored.contains("secret-after-value"));
    }

    #[test]
    fn cancelled_undo_is_audited_without_file_content() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("file.txt");
        let journal = dir.path().join("changes.jsonl");
        let recorder = ChangeRecorder::new(Some(journal.clone()));
        recorder.start_turn();
        recorder.record(exact(&path, Some(b"before"), b"after"));
        recorder.finish_turn();
        recorder.undo_preview().expect("preview");
        recorder.record_undo_cancelled();

        let stored = std::fs::read_to_string(journal).expect("journal");
        assert!(stored.contains(r#""outcome":"cancelled""#));
        assert!(!stored.contains(r#""before""#));
        assert!(!stored.contains(r#""after""#));
    }

    #[test]
    fn repeated_edits_to_one_path_restore_the_original_image() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("file.txt");
        std::fs::write(&path, b"third\n").expect("write");
        let recorder = ChangeRecorder::new(None);
        recorder.start_turn();
        recorder.record(exact(&path, Some(b"first\n"), b"second\n"));
        recorder.record(exact(&path, Some(b"second\n"), b"third\n"));
        let set = recorder.finish_turn().expect("set");
        assert_eq!(set.mutations.len(), 1);
        recorder.undo_latest().expect("undo");
        assert_eq!(std::fs::read(path).expect("read"), b"first\n");
    }

    #[test]
    fn exact_undo_can_be_previewed_and_redone_once() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("file.txt");
        std::fs::write(&path, b"after\n").expect("write");
        let recorder = ChangeRecorder::new(None);
        recorder.start_turn();
        recorder.record(exact(&path, Some(b"before\n"), b"after\n"));
        recorder.finish_turn().expect("set");
        recorder.undo_latest().expect("undo");

        let preview = recorder.redo_preview().expect("redo preview");
        assert!(preview.contains("-before"));
        assert!(preview.contains("+after"));
        recorder.redo_latest().expect("redo");

        assert_eq!(std::fs::read(&path).expect("read"), b"after\n");
        assert!(recorder.redo_latest().is_err(), "redo is single-use");
    }

    #[test]
    fn concurrent_edit_refuses_redo_without_touching_any_path() {
        let dir = tempfile::tempdir().expect("temp");
        let first = dir.path().join("first.txt");
        let second = dir.path().join("second.txt");
        std::fs::write(&first, b"after one\n").expect("first");
        std::fs::write(&second, b"after two\n").expect("second");
        let recorder = ChangeRecorder::new(None);
        recorder.start_turn();
        recorder.record(exact(&first, Some(b"before one\n"), b"after one\n"));
        recorder.record(exact(&second, Some(b"before two\n"), b"after two\n"));
        recorder.finish_turn().expect("set");
        recorder.undo_latest().expect("undo");

        std::fs::write(&second, b"user edit\n").expect("edit");
        let error = recorder.redo_latest().expect_err("conflict");
        assert!(error.message.contains("/diff"));
        assert!(error.message.contains("/timeline"));
        assert_eq!(std::fs::read(&first).expect("first"), b"before one\n");
        assert_eq!(std::fs::read(&second).expect("second"), b"user edit\n");
    }

    #[test]
    fn exact_selective_revert_is_a_redo_candidate() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("file.txt");
        std::fs::write(&path, b"restored\n").expect("reverted state");
        let recorder = ChangeRecorder::new(None);
        recorder.record_recovery(
            path.clone(),
            Some(b"changed\n".to_vec()),
            Some(b"restored\n".to_vec()),
            "revert",
            None,
        );

        let preview = recorder.redo_preview().expect("redo preview");
        assert!(preview.contains("-restored"));
        assert!(preview.contains("+changed"));
        recorder.redo_latest().expect("redo revert");

        assert_eq!(std::fs::read(path).expect("read"), b"changed\n");
        assert!(recorder.redo_preview().is_err(), "redo is single-use");
    }

    #[test]
    fn ambiguous_shell_delta_is_never_redoable() {
        let recorder = ChangeRecorder::new(None);
        recorder.start_turn();
        recorder.record(ToolMutation::Ambiguous {
            call_id: "shell-1".to_owned(),
            tool: "shell".to_owned(),
        });
        recorder.finish_turn().expect("set");
        assert!(recorder.redo_preview().is_err());
        assert!(recorder.redo_latest().is_err());
    }

    #[test]
    fn resumed_metadata_stays_visible_but_is_not_synthesized_into_undo_images() {
        let dir = tempfile::tempdir().expect("temp");
        let journal = dir.path().join("changes.jsonl");
        std::fs::write(
            &journal,
            r#"{"record":"turn_completed","schema_version":1,"turn":7,"fully_attributable":true,"mutations":[]}
"#,
        )
        .expect("journal");
        let recorder = ChangeRecorder::new(Some(journal));
        assert!(recorder.has_historical_records());
        assert!(
            recorder
                .undo_preview()
                .unwrap_err()
                .message
                .contains("historical")
        );

        recorder.start_turn();
        recorder.record(ToolMutation::Ambiguous {
            call_id: "shell".to_owned(),
            tool: "shell".to_owned(),
        });
        assert_eq!(recorder.finish_turn().expect("set").turn, 8);
    }
}
