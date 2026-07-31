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
//! Writes go through a temporary file and a rename, so an interrupted edit
//! leaves the original intact instead of a half-written source file.

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::tool::{InvocationContext, Tool, ToolEffects, ToolOutcome};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::support::{
    MAX_READ_BYTES, display_path, invalid, optional_bool, read_bounded, require_str, resolve,
};

/// Applies an exact-match edit to a project file.
#[derive(Debug, Default, Clone, Copy)]
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Edit a project file by replacing an exact string. `old_string` must \
         appear exactly once unless `replace_all` is set. To create a new file, \
         pass an empty `old_string`."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file, relative to the project root."
                },
                "old_string": {
                    "type": "string",
                    "description": "Exact text to replace, including indentation. Empty creates a new file."
                },
                "new_string": {
                    "type": "string",
                    "description": "Replacement text."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every occurrence instead of requiring exactly one."
                }
            },
            "required": ["path", "old_string", "new_string"],
            "additionalProperties": false
        })
    }

    fn effects(&self) -> ToolEffects {
        // The scope is the tool, not the path: effects are declared statically,
        // before arguments exist. The workspace enforces the actual boundary.
        ToolEffects::read_only().with_write("project:files")
    }

    async fn invoke(
        &self,
        arguments: Value,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let raw_path = require_str(&arguments, "path")?;
        let path = resolve(ctx, raw_path)?;
        let old_string = require_str(&arguments, "old_string")?;
        let new_string = require_str(&arguments, "new_string")?;
        let replace_all = optional_bool(&arguments, "replace_all").unwrap_or(false);
        let shown = display_path(ctx, &path);

        if old_string == new_string {
            return Err(invalid(
                "`old_string` and `new_string` are identical; the edit would do nothing",
            ));
        }

        // Creating a file.
        if old_string.is_empty() {
            if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                return Ok(ToolOutcome::error(format!(
                    "`{shown}` already exists; pass the text to replace in `old_string`"
                )));
            }
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|err| invalid(format!("cannot create `{shown}`: {err}")))?;
            }
            write_atomically(&path, new_string).await?;
            return Ok(ToolOutcome {
                value: json!({"path": shown, "created": true, "replacements": 1}),
                content: vec![agent_runtime_core::content::ContentPart::text(format!(
                    "created `{shown}` ({} lines)",
                    new_string.lines().count()
                ))],
                is_error: false,
            });
        }

        let contents = read_bounded(&path, MAX_READ_BYTES).await?;
        let occurrences = contents.matches(old_string).count();

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
            contents.replace(old_string, new_string)
        } else {
            contents.replacen(old_string, new_string, 1)
        };
        write_atomically(&path, &updated).await?;

        Ok(ToolOutcome {
            value: json!({
                "path": shown,
                "created": false,
                "replacements": if replace_all { occurrences } else { 1 },
            }),
            content: vec![agent_runtime_core::content::ContentPart::text(format!(
                "edited `{shown}` ({} replacement(s))",
                if replace_all { occurrences } else { 1 }
            ))],
            is_error: false,
        })
    }
}

/// Writes via a sibling temporary file and a rename.
///
/// The rename is atomic within a filesystem, so a reader either sees the old
/// file or the new one — never a truncated one. The temporary lives beside the
/// target rather than in `/tmp` so the rename cannot cross a filesystem.
async fn write_atomically(path: &std::path::Path, contents: &str) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(|| {
        invalid(format!(
            "cannot write `{}`: it has no parent directory",
            path.display()
        ))
    })?;
    let temporary = parent.join(format!(".smith-edit-{}.tmp", Uuid::new_v4()));

    let write = async {
        // `create_new` is atomic and refuses an existing path, including a
        // symlink. Keeping this handle open for the entire write avoids
        // re-opening a path an untrusted repository could swap underneath us.
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await?;
        file.write_all(contents.as_bytes()).await?;
        file.sync_all().await
    };
    if let Err(err) = write.await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(invalid(format!("cannot write `{}`: {err}", path.display())));
    }

    if let Err(err) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(invalid(format!(
            "cannot replace `{}`: {err}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{project, text_of};

    const SOURCE: &str = "fn main() {\n    println!(\"hello\");\n}\n";

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
    async fn an_empty_old_string_creates_a_file_and_its_parents() {
        let (dir, ctx) = project();

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

        assert!(outcome.is_error);
        assert!(text_of(&outcome).contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            SOURCE
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
        let err = EditTool
            .invoke(
                json!({"path": "../escape.rs", "old_string": "a", "new_string": "b"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind, agent_runtime_core::error::ErrorKind::Workspace);
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
        assert!(EditTool.effects().mutates());
        assert!(!EditTool.effects().is_read_only());
    }
}
