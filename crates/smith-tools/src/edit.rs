//! The `edit` tool.
//!
//! Edits are exact string replacements, not diffs. A unified diff carries line
//! numbers and context that a model reconstructs from memory, and when that
//! memory is one line stale the hunk either fails to apply or — worse — applies
//! at the wrong offset. An exact `old_string` either matches the file that is
//! actually on disk or it does not.
//!
//! The safety rule follows from that: `old_string` must match **exactly once**.
//! An ambiguous match is refused rather than resolved by picking the first,
//! because "the first occurrence" is rarely the one the model meant.
//!
//! Replacements go through a temporary file and a rename, so an interrupted
//! edit leaves the original intact instead of a half-written source file.
//! Smith's authority contract treats that randomized sibling as an internal,
//! non-durable implementation detail of the logical exact-target write: it is
//! never model-selectable and must be removed on both success and failure.
//! New files instead use an atomic `create_new` open on the prepared target so
//! a concurrent creator can never be overwritten.

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolEffects,
    ToolOutcome, ToolSpec,
};
use agent_runtime_registry::Permission;
use async_trait::async_trait;
use serde_json::{Value, json};
use smith_host::workspace::{FileVersion, ProjectWorkspace};

use crate::read_state::{ReadDefect, ReadRecorder};
use crate::support::{
    MAX_READ_BYTES, display_path, invalid, optional_bool, optional_str, prepare_path_argument,
    project_workspace, read_bounded, require_str, resolve,
};

/// Internal prepared-argument field carrying the exact prior read version.
pub(crate) const EXPECTED_VERSION_FIELD: &str = "_smith_expected_file_version";

/// What one `edit` call does to its target.
///
/// One tool with four verbs rather than four tools, for the same reason
/// `codex`'s `apply_patch` carries add/update/delete: the base tool surface is
/// sent on every request, so a peer tool costs tokens forever while an extra
/// enum variant costs a line of schema. It also keeps one path-resolution and
/// one approval-display path instead of four.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EditOperation {
    /// Replace an exact string. The historical behavior, and the default.
    #[default]
    Replace,
    /// Create a file that does not exist yet.
    Create,
    /// Replace an existing file's entire contents.
    Overwrite,
    /// Remove a file.
    Delete,
}

impl EditOperation {
    fn parse(value: Option<&str>) -> Result<Self, RuntimeError> {
        match value {
            None | Some("replace") => Ok(Self::Replace),
            Some("create") => Ok(Self::Create),
            Some("overwrite") => Ok(Self::Overwrite),
            Some("delete") => Ok(Self::Delete),
            Some(other) => Err(invalid(format!(
                "`operation` must be replace, create, overwrite, or delete, not `{other}`"
            ))),
        }
    }

    /// Whether the operation destroys content the call itself does not carry.
    ///
    /// `Replace` is excluded deliberately: a stale `old_string` fails to match,
    /// so an exact replacement proves its own currency. These two do not.
    pub fn destroys_unseen_content(self) -> bool {
        matches!(self, Self::Overwrite | Self::Delete)
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Replace => "Edit",
            Self::Create => "Create",
            Self::Overwrite => "Overwrite",
            Self::Delete => "Delete",
        }
    }
}

/// Applies an exact-match edit to a project file.
#[derive(Debug, Default, Clone, Copy)]
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "edit",
            "Change a project file. `replace` (the default) swaps an exact \
             `old_string` that must appear once unless `replace_all` is set; \
             prefer it for partial changes, since it sends only the diff. \
             `create`, `overwrite`, and `delete` act on the whole file, and the \
             last two require having read it first.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to a file inside the project root. Outside paths are refused; use the explicitly approved host shell for host access."
                    },
                    "operation": {
                        "type": "string",
                        "enum": ["replace", "create", "overwrite", "delete"],
                        "description": "Defaults to `replace`."
                    },
                    "old_string": {
                        "type": "string",
                        "description": "Exact text to replace, including indentation. Required for `replace`."
                    },
                    "new_string": {
                        "type": "string",
                        "description": "Replacement text. Required except for `delete`."
                    },
                    "replace_all": {
                        "type": "boolean",
                        "description": "Replace every occurrence instead of requiring exactly one."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
            ToolEffects::read_only().with_write("project:files"),
        )
        .with_permission_upper_bound(
            [
                Permission::FsRead,
                Permission::FsWrite,
                Permission::FsCreate,
                Permission::FsDelete,
            ]
            .into_iter()
            .collect(),
        )
    }

    async fn prepare(
        &self,
        mut arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let operation = resolve_operation(&arguments)?;
        if operation != EditOperation::Delete {
            let new_string = require_str(&arguments, "new_string")?.to_owned();
            if operation == EditOperation::Replace {
                let old_string = require_str(&arguments, "old_string")?.to_owned();
                if old_string == new_string {
                    return Err(invalid(
                        "`old_string` and `new_string` are identical; the edit would do nothing",
                    ));
                }
            }
        }
        let path = prepare_path_argument(&mut arguments, "path", None, ctx)?;
        if operation == EditOperation::Create {
            let workspace =
                ProjectWorkspace::from_workspace(ctx.workspace.as_ref()).ok_or_else(|| {
                    RuntimeError::new(
                        agent_runtime_core::error::ErrorKind::Workspace,
                        "Smith built-in filesystem tools require a ProjectWorkspace capability",
                    )
                })?;
            workspace.ensure_parent_directory(&path.canonical)?;
        }
        let object = arguments
            .as_object_mut()
            .ok_or_else(|| invalid("tool arguments must be a JSON object"))?;
        object.entry("replace_all").or_insert(Value::Bool(false));
        // Normalize the operation so an observer, a journal, and a replay all
        // see the same explicit verb the permission set was derived from.
        object.insert(
            "operation".to_owned(),
            Value::String(operation_name(operation).to_owned()),
        );

        let permissions = match operation {
            EditOperation::Create => [Permission::FsCreate, Permission::FsWrite]
                .into_iter()
                .collect(),
            EditOperation::Delete => [Permission::FsRead, Permission::FsDelete]
                .into_iter()
                .collect(),
            EditOperation::Replace | EditOperation::Overwrite => {
                [Permission::FsRead, Permission::FsWrite]
                    .into_iter()
                    .collect()
            }
        };
        let effects = match operation {
            EditOperation::Create => {
                ToolEffects::new(Vec::new()).with_write(path.canonical.clone())
            }
            _ => ToolEffects::read_only().with_write(path.canonical.clone()),
        };
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "edit",
            arguments,
            permissions,
            path.resource,
            effects,
            ToolCallDisplay::new(format!("{} {}", operation.verb(), path.display)),
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let raw_path = require_str(&arguments, "path")?;
        let path = resolve(ctx, raw_path)?;
        let workspace = project_workspace(ctx)?.clone();
        let operation = resolve_operation(&arguments)?;
        let replace_all = optional_bool(&arguments, "replace_all").unwrap_or(false);
        let shown = display_path(ctx, &path);

        match operation {
            EditOperation::Create => {
                let new_string = require_str(&arguments, "new_string")?;
                let path_for_write = path.clone();
                let bytes = new_string.as_bytes().to_vec();
                let created = tokio::task::spawn_blocking(move || {
                    workspace.create_new(path_for_write, &bytes)
                })
                .await
                .map_err(|_| RuntimeError::internal("workspace create task failed"))?;
                if let Err(err) = created {
                    return Ok(ToolOutcome::error(err.message));
                }
                return Ok(ToolOutcome {
                    value: json!({"path": shown, "created": true, "replacements": 1}),
                    content: vec![agent_runtime_core::content::ContentPart::text(format!(
                        "created `{shown}` ({} lines)",
                        new_string.lines().count()
                    ))]
                    .into(),
                    is_error: false,
                });
            }
            EditOperation::Overwrite => {
                let new_string = require_str(&arguments, "new_string")?;
                if workspace.entry(&path).is_err() {
                    return Ok(ToolOutcome::error(format!(
                        "`{shown}` does not exist; use `create` to make a new file"
                    )));
                }
                let expected = match expected_version_argument(&arguments) {
                    Ok(expected) => expected,
                    Err(err) => return Ok(ToolOutcome::error(err.message)),
                };
                let path_for_write = path.clone();
                let bytes = new_string.as_bytes().to_vec();
                tokio::task::spawn_blocking(move || {
                    workspace.replace_if_version(
                        path_for_write,
                        &bytes,
                        &expected,
                        MAX_READ_BYTES as usize,
                    )
                })
                .await
                .map_err(|_| RuntimeError::internal("workspace replace task failed"))??;
                return Ok(ToolOutcome {
                    value: json!({"path": shown, "created": false, "replacements": 1}),
                    content: vec![agent_runtime_core::content::ContentPart::text(format!(
                        "overwrote `{shown}` ({} lines)",
                        new_string.lines().count()
                    ))]
                    .into(),
                    is_error: false,
                });
            }
            EditOperation::Delete => {
                let expected = expected_version_argument(&arguments)?;
                let path_for_delete = path.clone();
                tokio::task::spawn_blocking(move || {
                    workspace.delete_if_version(path_for_delete, &expected, MAX_READ_BYTES as usize)
                })
                .await
                .map_err(|_| RuntimeError::internal("workspace delete task failed"))??;
                return Ok(ToolOutcome {
                    value: json!({"path": shown, "deleted": true}),
                    content: vec![agent_runtime_core::content::ContentPart::text(format!(
                        "deleted `{shown}`"
                    ))]
                    .into(),
                    is_error: false,
                });
            }
            EditOperation::Replace => {}
        }

        let old_string = require_str(&arguments, "old_string")?;
        let new_string = require_str(&arguments, "new_string")?;
        if old_string == new_string {
            return Err(invalid(
                "`old_string` and `new_string` are identical; the edit would do nothing",
            ));
        }

        let contents = read_bounded(ctx, &path, MAX_READ_BYTES).await?;
        let occurrences = contents.text.matches(old_string).count();

        match occurrences {
            0 => {
                return Ok(ToolOutcome::error(format!(
                    "`old_string` does not appear in `{shown}`. Read the file and \
                     match its exact text, including indentation."
                )));
            }
            // The dangerous case: silently editing one of several identical
            // sites would look like success and corrupt the wrong one.
            count if count > 1 && !replace_all => {
                return Ok(ToolOutcome::error(format!(
                    "`old_string` appears {count} times in `{shown}`. Include \
                     enough surrounding context to make it unique, or set \
                     `replace_all`."
                )));
            }
            _ => {}
        }

        let updated = if replace_all {
            contents.text.replace(old_string, new_string)
        } else {
            contents.text.replacen(old_string, new_string, 1)
        };
        let path_for_write = path.clone();
        let expected = contents.version;
        tokio::task::spawn_blocking(move || {
            workspace.replace_if_version(
                path_for_write,
                updated.as_bytes(),
                &expected,
                MAX_READ_BYTES as usize,
            )
        })
        .await
        .map_err(|_| RuntimeError::internal("workspace replace task failed"))??;

        Ok(ToolOutcome {
            value: json!({
                "path": shown,
                "created": false,
                "replacements": if replace_all { occurrences } else { 1 },
            }),
            content: vec![agent_runtime_core::content::ContentPart::text(format!(
                "edited `{shown}` ({} replacement(s))",
                if replace_all { occurrences } else { 1 }
            ))]
            .into(),
            is_error: false,
        })
    }
}

/// Whether a prepared `edit` call fails its read precondition.
///
/// Lives here, with the operations it constrains, rather than in the observing
/// wrapper that happens to hold the state: the rule is part of what `edit`
/// means, and a reader asking "when can this delete a file" should find the
/// answer in this module.
pub(crate) fn expected_read_version(
    arguments: &Value,
    reads: &ReadRecorder,
) -> Result<Option<FileVersion>, ReadDefect> {
    let Ok(operation) = resolve_operation(arguments) else {
        return Ok(None);
    };
    if !operation.destroys_unseen_content() {
        return Ok(None);
    }
    let path = optional_str(arguments, "path").ok_or(ReadDefect::Unread)?;
    reads.expected_version(std::path::Path::new(path)).map(Some)
}

fn expected_version_argument(arguments: &Value) -> Result<FileVersion, RuntimeError> {
    let value = arguments.get(EXPECTED_VERSION_FIELD).ok_or_else(|| {
        invalid("destructive edit is missing its prepared file-version precondition")
    })?;
    serde_json::from_value(value.clone())
        .map_err(|_| invalid("destructive edit carries an invalid file-version precondition"))
}

/// Resolves the operation from arguments, honoring the historical shorthand.
///
/// An empty `old_string` with no explicit `operation` has always meant "create",
/// and recorded transcripts replay through this path, so the shorthand outlives
/// the argument that made it necessary. An explicit `operation` always wins.
fn resolve_operation(arguments: &Value) -> Result<EditOperation, RuntimeError> {
    let explicit = optional_str(arguments, "operation");
    let operation = EditOperation::parse(explicit)?;
    if explicit.is_none()
        && operation == EditOperation::Replace
        && optional_str(arguments, "old_string").is_some_and(str::is_empty)
    {
        return Ok(EditOperation::Create);
    }
    Ok(operation)
}

fn operation_name(operation: EditOperation) -> &'static str {
    match operation {
        EditOperation::Replace => "replace",
        EditOperation::Create => "create",
        EditOperation::Overwrite => "overwrite",
        EditOperation::Delete => "delete",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{project, text_of};

    const SOURCE: &str = "fn main() {\n    println!(\"hello\");\n}\n";

    async fn prepared(
        arguments: Value,
        ctx: &agent_runtime_core::tool::InvocationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let preparation = PreparationContext {
            session: ctx.session.clone(),
            turn: ctx.turn.clone(),
            call_id: ctx.call_id.clone(),
            request: ctx.request.clone(),
            workspace: ctx.workspace.clone(),
            clock: ctx.clock.clone(),
            cancel: ctx.cancel.clone(),
            deadline: ctx.deadline,
        };
        EditTool.prepare(arguments, &preparation).await
    }

    #[tokio::test]
    async fn each_operation_requests_only_the_authority_it_needs() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        for (arguments, expected) in [
            (
                json!({"path": "a.rs", "old_string": "hello", "new_string": "world"}),
                vec![Permission::FsRead, Permission::FsWrite],
            ),
            (
                json!({"path": "new.rs", "operation": "create", "new_string": "fn n() {}\n"}),
                vec![Permission::FsWrite, Permission::FsCreate],
            ),
            (
                json!({"path": "a.rs", "operation": "overwrite", "new_string": "fn n() {}\n"}),
                vec![Permission::FsRead, Permission::FsWrite],
            ),
            (
                json!({"path": "a.rs", "operation": "delete"}),
                vec![Permission::FsRead, Permission::FsDelete],
            ),
        ] {
            let call = prepared(arguments.clone(), &ctx)
                .await
                .unwrap_or_else(|error| panic!("prepare {arguments}: {error}"));
            let mut requested = call
                .required_permissions()
                .iter()
                .map(|permission| permission.as_str().to_owned())
                .collect::<Vec<_>>();
            let mut expected = expected
                .iter()
                .map(|permission: &Permission| permission.as_str().to_owned())
                .collect::<Vec<_>>();
            requested.sort();
            expected.sort();
            assert_eq!(requested, expected, "for {arguments}");
        }
    }

    #[tokio::test]
    async fn delete_never_requests_spawn_or_network() {
        // The whole point of a narrow delete: `shell rm` would carry both, and
        // Smith's own tool-use policy forbids reaching for the broader tool.
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        let call = prepared(json!({"path": "a.rs", "operation": "delete"}), &ctx)
            .await
            .expect("a prepared delete");
        assert!(!call.effects().spawns_process());
        assert!(!call.effects().has_network());
    }

    #[tokio::test]
    async fn create_still_refuses_an_existing_target() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "a.rs", "operation": "create", "new_string": "replaced\n"}),
                &ctx,
            )
            .await
            .expect("create refusal is a tool outcome");

        assert!(text_of(&outcome).contains("already exists"), "{outcome:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            SOURCE
        );
    }

    #[tokio::test]
    async fn an_empty_old_string_still_creates() {
        // Recorded transcripts predate `operation` and replay through here.
        let (dir, ctx) = project();

        let outcome = EditTool
            .invoke(
                json!({"path": "made.rs", "old_string": "", "new_string": "fn made() {}\n"}),
                &ctx,
            )
            .await
            .expect("the legacy shorthand still creates");

        assert!(!outcome.is_error, "{}", text_of(&outcome));
        assert_eq!(outcome.value["created"], true);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("made.rs")).unwrap(),
            "fn made() {}\n"
        );
    }

    #[tokio::test]
    async fn an_unknown_operation_is_rejected_by_name() {
        let (_dir, ctx) = project();
        let error = prepared(
            json!({"path": "a.rs", "operation": "truncate", "new_string": "x"}),
            &ctx,
        )
        .await
        .expect_err("an unknown verb is refused");
        assert!(error.to_string().contains("truncate"), "{error}");
    }

    #[tokio::test]
    async fn overwrite_refuses_a_missing_file() {
        let (_dir, ctx) = project();
        let outcome = EditTool
            .invoke(
                json!({"path": "absent.rs", "operation": "overwrite", "new_string": "x\n"}),
                &ctx,
            )
            .await
            .expect("a missing target is a tool error, not a runtime failure");
        assert!(outcome.is_error);
        assert!(
            text_of(&outcome).contains("use `create`"),
            "{}",
            text_of(&outcome)
        );
    }

    #[tokio::test]
    async fn a_unique_match_is_replaced() {
        let (dir, ctx) = project();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, SOURCE).unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "\"hello\"", "new_string": "\"world\""}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!outcome.is_error);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "fn main() {\n    println!(\"world\");\n}\n"
        );
    }

    #[tokio::test]
    async fn an_ambiguous_match_is_refused_rather_than_guessed() {
        let (dir, ctx) = project();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "let x = 1;\nlet y = 1;\n").unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "= 1;", "new_string": "= 2;"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(outcome.is_error);
        assert!(text_of(&outcome).contains("appears 2 times"), "{outcome:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "let x = 1;\nlet y = 1;\n",
            "a refused edit must not touch the file"
        );
    }

    #[tokio::test]
    async fn replace_all_edits_every_occurrence() {
        let (dir, ctx) = project();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "let x = 1;\nlet y = 1;\n").unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "= 1;", "new_string": "= 2;", "replace_all": true}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(outcome.value["replacements"], 2);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "let x = 2;\nlet y = 2;\n"
        );
    }

    #[tokio::test]
    async fn a_missing_match_explains_how_to_fix_it() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "absent", "new_string": "x"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(outcome.is_error);
        assert!(text_of(&outcome).contains("does not appear"));
    }

    #[tokio::test]
    async fn an_empty_old_string_creates_a_file_in_an_existing_directory() {
        let (dir, ctx) = project();
        std::fs::create_dir_all(dir.path().join("src/new")).unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "src/new/mod.rs", "old_string": "", "new_string": "pub mod a;\n"}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(outcome.value["created"], true);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/new/mod.rs")).unwrap(),
            "pub mod a;\n"
        );
    }

    #[tokio::test]
    async fn creating_a_file_never_silently_creates_unprepared_parent_directories() {
        let (dir, ctx) = project();

        let err = EditTool
            .invoke(
                json!({"path": "missing/nested/new.rs", "old_string": "", "new_string": "x\n"}),
                &ctx,
            )
            .await
            .expect_err("a missing parent is refused during preparation");

        assert!(err.message.contains("parent directory"), "{err:?}");
        assert!(
            !dir.path().join("missing").exists(),
            "an exact-file invocation created an unprepared ancestor"
        );
    }

    #[tokio::test]
    async fn creating_over_an_existing_file_is_refused() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "", "new_string": "replaced"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(text_of(&outcome).contains("already exists"), "{outcome:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            SOURCE
        );
    }

    #[tokio::test]
    async fn a_concurrent_creator_is_never_overwritten_after_preparation() {
        let (dir, ctx) = project();
        let preparation = PreparationContext {
            session: ctx.session.clone(),
            turn: ctx.turn.clone(),
            call_id: ctx.call_id.clone(),
            request: ctx.request.clone(),
            workspace: ctx.workspace.clone(),
            clock: ctx.clock.clone(),
            cancel: ctx.cancel.clone(),
            deadline: ctx.deadline,
        };
        let prepared = Tool::prepare(
            &EditTool,
            json!({"path": "raced.rs", "old_string": "", "new_string": "smith\n"}),
            &preparation,
        )
        .await
        .expect("the absent target prepares");

        std::fs::write(dir.path().join("raced.rs"), "other process\n")
            .expect("a competing creator wins the race");
        let outcome = Tool::invoke(&EditTool, prepared, &ctx)
            .await
            .expect("create-only refusal is a tool outcome");

        assert!(text_of(&outcome).contains("already exists"), "{outcome:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("raced.rs")).unwrap(),
            "other process\n",
            "the create-only invocation overwrote a concurrent creator"
        );
    }

    #[tokio::test]
    async fn a_no_op_edit_is_rejected() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        let err = EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "x", "new_string": "x"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("identical"), "{err:?}");
    }

    #[tokio::test]
    async fn editing_outside_the_project_is_refused() {
        let (_dir, ctx) = project();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("escape.rs");
        std::fs::write(&target, "a\n").unwrap();

        let err = EditTool
            .invoke(
                json!({"path": target.to_str().unwrap(), "old_string": "a", "new_string": "b"}),
                &ctx,
            )
            .await
            .expect_err("ordinary edit paths are capability-contained");
        assert_eq!(err.kind, agent_runtime_core::error::ErrorKind::Workspace);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "a\n");
    }

    #[tokio::test]
    async fn no_temporary_file_survives_a_successful_edit() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "hello", "new_string": "world"}),
                &ctx,
            )
            .await
            .unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("smith-edit-"))
            .collect();
        assert!(leftovers.is_empty(), "a temporary file was left behind");
    }

    #[tokio::test]
    async fn no_temporary_file_survives_a_failed_atomic_replace() {
        let (dir, ctx) = project();
        let destination = dir.path().join("occupied");
        std::fs::write(&destination, "original").unwrap();
        let workspace = project_workspace(&ctx).unwrap();
        let expected = workspace.read_bounded(&destination, 1024).unwrap().version;
        std::fs::remove_file(&destination).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("keep"), "unchanged").unwrap();

        let err = workspace
            .replace_if_version(&destination, b"replacement", &expected, 1024)
            .expect_err("renaming a file over a non-empty directory must fail");
        assert!(
            err.message.contains("changed") || err.message.contains("regular file"),
            "{err:?}"
        );
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("smith-edit-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "a failed atomic replacement left a sibling temporary behind"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("keep")).unwrap(),
            "unchanged"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_preexisting_temp_symlink_cannot_escape_the_workspace() {
        use std::os::unix::fs::symlink;

        let (dir, ctx) = project();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim.txt");
        std::fs::write(&victim, "outside must stay unchanged").unwrap();
        std::fs::write(dir.path().join("a.rs"), SOURCE).unwrap();

        // This was the edit tool's old, predictable temp path. Opening it with
        // truncate semantics followed the symlink and modified `victim`
        // before the final rename touched the in-workspace target.
        symlink(&victim, dir.path().join("a.rs.smith-tmp")).unwrap();

        let outcome = EditTool
            .invoke(
                json!({"path": "a.rs", "old_string": "hello", "new_string": "world"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!outcome.is_error);
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "outside must stay unchanged"
        );
        assert!(
            std::fs::read_to_string(dir.path().join("a.rs"))
                .unwrap()
                .contains("world")
        );
    }

    #[tokio::test]
    async fn the_tool_declares_a_write_effect_so_approval_applies() {
        assert!(EditTool.spec().effects.mutates());
        assert!(!EditTool.spec().effects.is_read_only());
    }
}
