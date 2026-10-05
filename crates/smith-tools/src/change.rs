//! In-session attribution for mutating Smith tools.
//!
//! Exact `edit` calls retain bounded pre/post images in memory for conflict
//! checked undo. The persisted journal receives hashes and path metadata only;
//! arbitrary file contents and protected tool arguments are never serialized.
//! Shell mutations are marked ambiguous because observing a Git delta does not
//! prove that every concurrent byte belongs to the command. A turn that mixes
//! both stays recoverable for the edits Smith performed itself: `/undo`
//! reverses those exact images and reports the ambiguous delta beside them
//! rather than reconstructing or silently dropping it.

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

    /// The mutations Smith performed through its own editing tools, in order.
    ///
    /// Automatic recovery works from exactly these: each one carries both
    /// images, so reversing it is a checked write rather than a guess. A turn
    /// that also ran a shell command keeps those exact edits recoverable —
    /// the ambiguous delta beside them is reported, never reconstructed.
    pub fn exact_mutations(&self) -> impl Iterator<Item = &EditMutation> {
        self.mutations.iter().filter_map(|mutation| match mutation {
            ToolMutation::Exact(edit) => Some(edit),
            ToolMutation::Ambiguous { .. } => None,
        })
    }

    /// Whether automatic recovery has any exact image to work from.
    pub fn has_exact_mutations(&self) -> bool {
        self.exact_mutations().next().is_some()
    }

    /// Distinct tools whose delta could not be attributed file by file.
    pub fn ambiguous_tools(&self) -> Vec<&str> {
        let mut tools = self
            .mutations
            .iter()
            .filter_map(|mutation| match mutation {
                ToolMutation::Ambiguous { tool, .. } => Some(tool.as_str()),
                ToolMutation::Exact(_) => None,
            })
            .collect::<Vec<_>>();
        tools.sort_unstable();
        tools.dedup();
        tools
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
    project_root: Option<PathBuf>,
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
            project_root: None,
        }
    }

    /// Uses project-relative paths in recovery previews, retaining absolute
    /// paths for targets outside the project. Canonical paths still own I/O.
    pub fn with_project_root(mut self, root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        self.project_root = Some(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
        self
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
            } else if set.has_exact_mutations() {
                "mixed"
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
                unavailable("undo is not available for turns from before this session was resumed")
            } else {
                unavailable("no Smith turn has attributable changes")
            }
        })?;
        if set.undone {
            return Err(unavailable(
                "the newest attributable turn was already undone",
            ));
        }
        if !set.has_exact_mutations() {
            return Err(unavailable(
                "the newest turn changed the workspace only through shell or extension \
                 deltas Smith cannot attribute file by file; use /diff and /revert",
            ));
        }
        let output = undo_preview_text(&set, self.project_root.as_deref());
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
            .filter(|set| !set.undone && set.has_exact_mutations())
            .map(|set| {
                hash(Some(
                    undo_preview_text(&set, self.project_root.as_deref()).as_bytes(),
                ))
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
        let edits = set.exact_mutations().collect::<Vec<_>>();
        if set.undone || edits.is_empty() {
            return Err(unavailable(
                "the newest turn is not eligible for automatic undo",
            ));
        }

        for edit in &edits {
            let current = bounded_image(&edit.path)?;
            if hash(current.as_deref()) != edit.after_hash {
                self.persist(&JournalEntry::Recovery {
                    operation: "undo",
                    turn: set.turn,
                    outcome: "conflict",
                    fingerprint: None,
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
                    fingerprint: None,
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
        let fingerprint = hash(Some(
            undo_preview_text(&set, self.project_root.as_deref()).as_bytes(),
        ));
        self.persist(&JournalEntry::Recovery {
            operation: "undo",
            turn: set.turn,
            outcome: "applied",
            fingerprint: Some(&fingerprint),
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
                "no exact redo candidate exists; changes Smith cannot attribute file by file \
                 are never reapplied automatically",
            )
        })?;
        let output = redo_preview_text(&set, direction, self.project_root.as_deref());
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
            .map(|(set, direction)| {
                redo_preview_text(&set, direction, self.project_root.as_deref())
            })
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
        let edits = set.exact_mutations().collect::<Vec<_>>();
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
                    fingerprint: None,
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
                    fingerprint: None,
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
        let fingerprint = hash(Some(
            redo_preview_text(&set, direction, self.project_root.as_deref()).as_bytes(),
        ));
        self.persist(&JournalEntry::Recovery {
            operation: "redo",
            turn: set.turn,
            outcome: "applied",
            fingerprint: Some(&fingerprint),
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

/// Wraps the built-in tools with whatever session state they need.
///
/// Mutation attribution is optional — a caller may not want undo — but read
/// state never is, because `edit`'s `overwrite` and `delete` are refused
/// without it.
pub(crate) fn observe(
    recorder: Option<Arc<ChangeRecorder>>,
    reads: Arc<ReadRecorder>,
) -> Vec<Arc<dyn Tool>> {
    observe_with_background(recorder, reads, crate::background::unavailable())
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

fn bounded_capability_image(
    ctx: &InvocationContext,
    path: &Path,
) -> Result<Option<Vec<u8>>, RuntimeError> {
    crate::support::project_workspace(ctx)?
        .read_optional_bounded(path, MAX_IMAGE_BYTES as usize)
        .map(|read| read.map(|read| read.bytes))
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
        #[serde(skip_serializing_if = "Option::is_none")]
        fingerprint: Option<&'a str>,
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
mod observed_session;

#[cfg(test)]
mod tests;

mod observation;
mod recovery;

pub(crate) use observation::observe_with_background;
pub use observation::{observed_tools, observed_tools_with_background};
use recovery::{redo_direction, redo_preview_text, undo_preview_text};
