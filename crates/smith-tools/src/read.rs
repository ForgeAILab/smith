//! The `read` tool.
//!
//! Reads a file as text, optionally a line range. Output is line-numbered
//! because the `edit` tool and the model's own references are both easier to
//! ground when every line has a visible address, and because a model asked to
//! "fix line 40" needs to have seen line 40 labelled as such.

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::tool::{InvocationContext, Tool, ToolEffects, ToolOutcome};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::support::{
    MAX_READ_BYTES, display_path, invalid, optional_usize, read_bounded, require_str, resolve,
};

/// How many lines a read returns when the caller does not say.
const DEFAULT_LINE_LIMIT: usize = 2000;

/// Reads a text file from the project.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Read a text file from the project. Returns line-numbered content. \
         Use `offset` and `limit` to read part of a large file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file, relative to the project root."
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "First line to return, 1-based. Defaults to 1."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum lines to return. Defaults to 2000."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn effects(&self) -> ToolEffects {
        ToolEffects::read_only()
    }

    async fn invoke(
        &self,
        arguments: Value,
        ctx: &InvocationContext,
    ) -> Result<ToolOutcome, RuntimeError> {
        let raw_path = require_str(&arguments, "path")?;
        let path = resolve(ctx, raw_path)?;

        let offset = match optional_usize(&arguments, "offset") {
            Some(0) => return Err(invalid("`offset` is 1-based; the first line is 1")),
            Some(value) => value,
            None => 1,
        };
        let limit = optional_usize(&arguments, "limit").unwrap_or(DEFAULT_LINE_LIMIT);
        if limit == 0 {
            return Err(invalid("`limit` must be at least 1"));
        }

        let contents = read_bounded(&path, MAX_READ_BYTES).await?;
        let all: Vec<&str> = contents.lines().collect();
        let total = all.len();

        if offset > total {
            // An empty result would read as "the file is empty"; say what is
            // actually true so the model corrects its offset rather than its
            // conclusion.
            return Ok(ToolOutcome::error(format!(
                "`{}` has {total} lines; `offset` {offset} is past the end",
                display_path(ctx, &path)
            )));
        }

        let start = offset - 1;
        let end = start.saturating_add(limit).min(total);
        let width = end.to_string().len();

        let mut rendered = String::new();
        for (index, line) in all[start..end].iter().enumerate() {
            let number = start + index + 1;
            rendered.push_str(&format!("{number:>width$}  {line}\n"));
        }
        if end < total {
            rendered.push_str(&format!(
                "\n[{} more lines; read from offset {}]\n",
                total - end,
                end + 1
            ));
        }

        Ok(ToolOutcome {
            value: json!({
                "path": display_path(ctx, &path),
                "lines": total,
                "shown": [offset, end],
            }),
            content: vec![agent_runtime_core::content::ContentPart::text(rendered)],
            is_error: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::project;

    #[tokio::test]
    async fn a_file_is_returned_line_numbered() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.rs"), "fn main() {\n    ok();\n}\n").unwrap();

        let outcome = ReadTool
            .invoke(json!({"path": "a.rs"}), &ctx)
            .await
            .unwrap();
        let text = crate::testing::text_of(&outcome);

        assert!(!outcome.is_error);
        assert!(text.contains("1  fn main() {"), "{text}");
        assert!(text.contains("2      ok();"), "{text}");
        assert_eq!(outcome.value["lines"], 3);
    }

    #[tokio::test]
    async fn a_line_range_is_honored_and_reports_what_remains() {
        let (dir, ctx) = project();
        let body: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        std::fs::write(dir.path().join("long.txt"), body).unwrap();

        let outcome = ReadTool
            .invoke(json!({"path": "long.txt", "offset": 10, "limit": 5}), &ctx)
            .await
            .unwrap();
        let text = crate::testing::text_of(&outcome);

        assert!(text.contains("10  line 10"), "{text}");
        assert!(text.contains("14  line 14"), "{text}");
        assert!(!text.contains("line 15"), "{text}");
        assert!(
            text.contains("[86 more lines; read from offset 15]"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn an_offset_past_the_end_says_so_instead_of_looking_empty() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("short.txt"), "one\ntwo\n").unwrap();

        let outcome = ReadTool
            .invoke(json!({"path": "short.txt", "offset": 50}), &ctx)
            .await
            .unwrap();

        assert!(outcome.is_error);
        let text = crate::testing::text_of(&outcome);
        assert!(text.contains("has 2 lines"), "{text}");
    }

    #[tokio::test]
    async fn reading_outside_the_project_is_refused() {
        let (_dir, ctx) = project();
        let err = ReadTool
            .invoke(json!({"path": "../../etc/passwd"}), &ctx)
            .await
            .unwrap_err();
        assert_eq!(err.kind, agent_runtime_core::error::ErrorKind::Workspace);
    }

    #[tokio::test]
    async fn a_zero_offset_is_rejected_rather_than_silently_shifted() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();
        assert!(
            ReadTool
                .invoke(json!({"path": "a.txt", "offset": 0}), &ctx)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_missing_file_reports_the_path() {
        let (_dir, ctx) = project();
        let err = ReadTool
            .invoke(json!({"path": "absent.rs"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("absent.rs"), "{err:?}");
    }
}
