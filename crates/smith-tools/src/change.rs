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

use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::tool::{InvocationContext, Tool, ToolEffects, ToolOutcome};
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

    /// Journals a selective recovery request or outcome without file content.
    pub fn record_revert_event(&self, scope: &str, fingerprint: &str, outcome: &str) {
        self.persist(&JournalEntry::RecoveryRequest {
            operation: "revert",
            scope,
            fingerprint,
            outcome,
        });
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
    crate::all()
        .into_iter()
        .map(|inner| {
            Arc::new(ObservedTool {
                inner,
                recorder: recorder.clone(),
            }) as Arc<dyn Tool>
        })
        .collect()
}

#[derive(Debug)]
struct ObservedTool {
    inner: Arc<dyn Tool>,
    recorder: Arc<ChangeRecorder>,
}

#[async_trait]
impl Tool for ObservedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn input_schema(&self) -> Value {
        self.inner.input_schema()
    }

    fn effects(&self) -> ToolEffects {
        self.inner.effects()
    }

    async fn invoke(
        &self,
        arguments: Value,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let edit_path = if self.inner.name() == "edit" {
            arguments
                .get("path")
                .and_then(Value::as_str)
                .and_then(|path| ctx.workspace.resolve(path).ok())
                .map(PathBuf::from)
        } else {
            None
        };
        let before = match &edit_path {
            Some(path) => bounded_image(path),
            None => Ok(None),
        };
        let outcome = self.inner.invoke(arguments, ctx).await;
        match (&outcome, edit_path, before) {
            (Ok(outcome), Some(path), Ok(before)) if !outcome.is_error => {
                match bounded_image(&path) {
                    Ok(Some(after)) => self.recorder.record(ToolMutation::Exact(EditMutation {
                        call_id: ctx.call_id.as_str().to_owned(),
                        path,
                        before_hash: hash(before.as_deref()),
                        after_hash: hash(Some(&after)),
                        before,
                        after: Some(after),
                        recovery_path: None,
                    })),
                    _ => self.recorder.record(ToolMutation::Ambiguous {
                        call_id: ctx.call_id.as_str().to_owned(),
                        tool: self.inner.name().to_owned(),
                    }),
                }
            }
            (_, Some(_), Err(_)) if self.inner.name() == "edit" => {
                self.recorder.record(ToolMutation::Ambiguous {
                    call_id: ctx.call_id.as_str().to_owned(),
                    tool: self.inner.name().to_owned(),
                });
            }
            (_, _, _) if self.inner.effects().mutates() && self.inner.name() != "edit" => {
                self.recorder.record(ToolMutation::Ambiguous {
                    call_id: ctx.call_id.as_str().to_owned(),
                    tool: self.inner.name().to_owned(),
                });
            }
            _ => {}
        }
        outcome
    }
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
