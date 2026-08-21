//! The `list` tool.
//!
//! Lists directory entries across the project. Hidden paths and common build
//! directories (`.git`, `target`, `node_modules`) are skipped by default,
//! because a listing dominated by artifacts costs tokens and buries the files
//! the model actually needs.

use std::collections::BTreeSet;

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::security::PermissionSet;
use agent_runtime_core::tool::{
    InvocationContext, PreparationContext, PreparedToolCall, Tool, ToolCallDisplay, ToolEffects,
    ToolOutcome, ToolSpec,
};
use agent_runtime_registry::Permission;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::support::{
    check_stop, display_path, optional_bool, optional_str, optional_usize, prepare_path_argument,
    project_workspace, resolve,
};

/// How many entries a listing returns when the caller does not say.
const DEFAULT_LIMIT: usize = 500;

/// Lists project files and directories.
#[derive(Debug, Default, Clone, Copy)]
pub struct ListTool;

#[async_trait]
impl Tool for ListTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "list",
            "List files and directories in the project. Set `recursive` to walk the \
             tree. Hidden paths and common build directories (.git, target, \
             node_modules) are skipped unless `all` is set.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to list inside the project root. Defaults to the root. Outside paths are refused."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "Walk subdirectories. Defaults to false."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "Include hidden and common build-directory entries. Defaults to false."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Maximum entries to return. Defaults to 500."
                    }
                },
                "additionalProperties": false
            }),
            ToolEffects::read_only(),
        )
        .with_permission_upper_bound(PermissionSet::single(Permission::FsRead))
    }

    async fn prepare(
        &self,
        mut arguments: Value,
        ctx: &PreparationContext,
    ) -> Result<PreparedToolCall, RuntimeError> {
        let path = prepare_path_argument(&mut arguments, "path", Some("."), ctx)?;
        Ok(PreparedToolCall::new(
            ctx.call_id.clone(),
            "list",
            arguments,
            PermissionSet::single(Permission::FsRead),
            path.resource,
            ToolEffects::read_only(),
            ToolCallDisplay::new(format!("List {}", path.display)),
        ))
    }

    async fn invoke(
        &self,
        prepared: PreparedToolCall,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let arguments = prepared.into_arguments();
        let root = resolve(ctx, optional_str(&arguments, "path").unwrap_or("."))?;
        let recursive = optional_bool(&arguments, "recursive").unwrap_or(false);
        let all = optional_bool(&arguments, "all").unwrap_or(false);
        let limit = optional_usize(&arguments, "limit")
            .unwrap_or(DEFAULT_LIMIT)
            .max(1);

        let workspace = project_workspace(ctx)?.clone();
        let root_entry = workspace.entry(&root)?;
        if !root_entry.is_dir {
            return Ok(ToolOutcome::error(format!(
                "`{}` is not a directory",
                display_path(ctx, &root)
            )));
        }
        let root_for_walk = root.clone();
        let mut walked = tokio::task::spawn_blocking(move || {
            workspace.entries(root_for_walk, recursive, all, limit.saturating_add(1))
        })
        .await
        .map_err(|_| RuntimeError::internal("workspace list task failed"))??;

        let mut directories = BTreeSet::new();
        let mut files = BTreeSet::new();
        let truncated = walked.len() > limit;
        walked.truncate(limit);

        for entry in walked {
            check_stop(ctx)?;
            let shown = entry.relative_path.display().to_string();
            if entry.is_dir {
                directories.insert(format!("{shown}/"));
            } else {
                files.insert(shown);
            }
        }

        let total = directories.len() + files.len();
        let mut rendered = String::new();
        for entry in directories.iter().chain(files.iter()) {
            rendered.push_str(entry);
            rendered.push('\n');
        }
        if truncated {
            rendered.push_str(&format!("\n[stopped at {limit} entries]\n"));
        }
        if total == 0 {
            rendered.push_str("(empty)\n");
        }

        Ok(ToolOutcome {
            value: json!({
                "path": display_path(ctx, &root),
                "entries": total,
                "truncated": truncated,
            }),
            content: vec![agent_runtime_core::content::ContentPart::text(rendered)].into(),
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{project, text_of};

    fn populate(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("target/debug/binary"), "artifact").unwrap();
        std::fs::write(dir.join(".git/HEAD"), "ref: main").unwrap();
        std::fs::write(dir.join(".gitignore"), "target\n").unwrap();
    }

    #[tokio::test]
    async fn a_shallow_listing_shows_directories_and_files() {
        let (dir, ctx) = project();
        populate(dir.path());

        let outcome = ListTool.invoke(json!({}), &ctx).await.unwrap();
        let text = text_of(&outcome);

        assert!(text.contains("src/"), "{text}");
        assert!(text.contains("Cargo.toml"), "{text}");
        // Shallow: the nested file is not reached.
        assert!(!text.contains("main.rs"), "{text}");
    }

    #[tokio::test]
    async fn build_artifacts_and_git_metadata_are_skipped_by_default() {
        let (dir, ctx) = project();
        populate(dir.path());

        let text = text_of(
            &ListTool
                .invoke(json!({"recursive": true}), &ctx)
                .await
                .unwrap(),
        );

        assert!(text.contains("src/main.rs"), "{text}");
        assert!(
            !text.contains("target"),
            "gitignored paths must be skipped:\n{text}"
        );
        assert!(!text.contains(".git/"), ".git must be skipped:\n{text}");
    }

    #[tokio::test]
    async fn all_includes_what_the_default_hides() {
        let (dir, ctx) = project();
        populate(dir.path());

        let text = text_of(
            &ListTool
                .invoke(json!({"recursive": true, "all": true}), &ctx)
                .await
                .unwrap(),
        );
        assert!(text.contains("target"), "{text}");
    }

    #[tokio::test]
    async fn a_limit_stops_the_walk_and_says_so() {
        let (dir, ctx) = project();
        for index in 0..50 {
            std::fs::write(dir.path().join(format!("f{index}.txt")), "x").unwrap();
        }

        let outcome = ListTool
            .invoke(json!({"recursive": true, "limit": 10}), &ctx)
            .await
            .unwrap();

        assert_eq!(outcome.value["truncated"], true);
        assert!(text_of(&outcome).contains("[stopped at 10 entries]"));
    }

    #[tokio::test]
    async fn an_empty_directory_says_empty_rather_than_returning_nothing() {
        let (dir, ctx) = project();
        std::fs::create_dir(dir.path().join("blank")).unwrap();

        let text = text_of(
            &ListTool
                .invoke(json!({"path": "blank"}), &ctx)
                .await
                .unwrap(),
        );
        assert!(text.contains("(empty)"), "{text}");
    }

    #[tokio::test]
    async fn listing_outside_the_project_is_refused() {
        let (_dir, ctx) = project();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("beyond.txt"), "x").unwrap();

        let err = ListTool
            .invoke(json!({"path": outside.path().to_str().unwrap()}), &ctx)
            .await
            .expect_err("ordinary listings are capability-contained");
        assert_eq!(err.kind, agent_runtime_core::error::ErrorKind::Workspace);
    }

    #[tokio::test]
    async fn a_file_path_reports_that_it_is_not_a_directory() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();

        let outcome = ListTool
            .invoke(json!({"path": "a.txt"}), &ctx)
            .await
            .unwrap();
        assert!(outcome.is_error);
        assert!(text_of(&outcome).contains("not a directory"));
    }

    #[tokio::test]
    async fn a_cancelled_invocation_stops_walking() {
        let (dir, ctx) = project();
        for index in 0..20 {
            std::fs::write(dir.path().join(format!("f{index}.txt")), "x").unwrap();
        }
        ctx.cancel
            .cancel(agent_runtime_core::cancel::CancelReason::UserRequested);

        let err = ListTool
            .invoke(json!({"recursive": true}), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.kind, agent_runtime_core::error::ErrorKind::Cancelled);
    }
}
