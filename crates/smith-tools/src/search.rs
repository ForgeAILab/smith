//! The `search` tool.
//!
//! Substring search across project text files, reported as `path:line: text`.
//!
//! It is deliberately **not** a regular-expression engine. A literal substring
//! has no catastrophic-backtracking failure mode, no dialect ambiguity between
//! what the model writes and what the engine accepts, and no way for a mistyped
//! pattern to become an accidental `.*`. Matching a shape rather than a string
//! is what `shell` with `rg` is for, once the user has approved it.

use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::tool::{InvocationContext, Tool, ToolEffects, ToolOutcome};
use async_trait::async_trait;
use ignore::WalkBuilder;
use serde_json::{Value, json};

use crate::support::{
    check_stop, display_path, invalid, looks_binary, optional_bool, optional_str, optional_usize,
    require_str, resolve,
};

/// How many matches a search returns when the caller does not say.
const DEFAULT_LIMIT: usize = 100;

/// The longest match line reported before it is trimmed.
const MAX_LINE: usize = 300;

/// Files above this size are skipped: a minified bundle or a checked-in dump
/// produces matches nobody can act on.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Searches project files for a literal substring.
#[derive(Debug, Default, Clone, Copy)]
pub struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }

    fn description(&self) -> &str {
        "Search project text files for a literal substring. Returns \
         `path:line: text` matches. Not a regular expression: the pattern is \
         matched verbatim."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Literal text to find. Not a regular expression."
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search. Defaults to the project root."
                },
                "extension": {
                    "type": "string",
                    "description": "Restrict to files with this extension, e.g. `rs`."
                },
                "case_sensitive": {
                    "type": "boolean",
                    "description": "Defaults to false."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Maximum matches to return. Defaults to 100."
                }
            },
            "required": ["pattern"],
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
        let pattern = require_str(&arguments, "pattern")?;
        if pattern.is_empty() {
            return Err(invalid("`pattern` must not be empty"));
        }
        let root = resolve(ctx, optional_str(&arguments, "path").unwrap_or("."))?;
        let extension =
            optional_str(&arguments, "extension").map(|ext| ext.trim_start_matches('.'));
        let case_sensitive = optional_bool(&arguments, "case_sensitive").unwrap_or(false);
        let limit = optional_usize(&arguments, "limit")
            .unwrap_or(DEFAULT_LIMIT)
            .max(1);

        let needle = if case_sensitive {
            pattern.to_owned()
        } else {
            pattern.to_lowercase()
        };

        let mut matches = Vec::new();
        let mut files_searched = 0usize;
        let mut truncated = false;

        let mut walker = WalkBuilder::new(&root);
        walker.hidden(true).git_ignore(true).parents(true);
        walker.filter_entry(|entry| entry.file_name() != ".git");

        'walk: for entry in walker.build() {
            check_stop(ctx)?;
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            if let Some(wanted) = extension
                && path.extension().and_then(|ext| ext.to_str()) != Some(wanted)
            {
                continue;
            }
            if entry
                .metadata()
                .is_ok_and(|meta| meta.len() > MAX_FILE_BYTES)
            {
                continue;
            }

            let Ok(bytes) = tokio::fs::read(path).await else {
                continue;
            };
            if looks_binary(&bytes) {
                continue;
            }
            let Ok(contents) = String::from_utf8(bytes) else {
                continue;
            };
            files_searched += 1;

            for (index, line) in contents.lines().enumerate() {
                let haystack = if case_sensitive {
                    line.to_owned()
                } else {
                    line.to_lowercase()
                };
                if !haystack.contains(&needle) {
                    continue;
                }
                if matches.len() >= limit {
                    truncated = true;
                    break 'walk;
                }
                matches.push(format!(
                    "{}:{}: {}",
                    display_path(ctx, path),
                    index + 1,
                    trim_line(line)
                ));
            }
        }

        let found = matches.len();
        let mut rendered = matches.join("\n");
        if found == 0 {
            // An empty result and a failed search look identical otherwise, and
            // the model needs to tell "not present" from "I searched wrong".
            rendered = format!(
                "no matches for `{pattern}` in {files_searched} file(s) under `{}`",
                display_path(ctx, &root)
            );
        } else if truncated {
            rendered.push_str(&format!("\n\n[stopped at {limit} matches]"));
        }

        Ok(ToolOutcome {
            value: json!({
                "pattern": pattern,
                "matches": found,
                "files_searched": files_searched,
                "truncated": truncated,
            }),
            content: vec![agent_runtime_core::content::ContentPart::text(rendered)],
            is_error: false,
        })
    }
}

/// Trims a long line so one minified match cannot flood the result.
fn trim_line(line: &str) -> String {
    let trimmed = line.trim_end();
    if trimmed.chars().count() <= MAX_LINE {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(MAX_LINE).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{project, text_of};

    fn populate(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/retry.rs"),
            "pub fn retry() {\n    // retry the request\n    backoff();\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n    retry();\n}\n").unwrap();
        std::fs::write(dir.join("notes.md"), "The RETRY policy is documented.\n").unwrap();
    }

    #[tokio::test]
    async fn matches_are_reported_with_path_and_line() {
        let (dir, ctx) = project();
        populate(dir.path());

        let outcome = SearchTool
            .invoke(json!({"pattern": "backoff"}), &ctx)
            .await
            .unwrap();
        let text = text_of(&outcome);

        assert_eq!(outcome.value["matches"], 1);
        assert!(text.contains("src/retry.rs:3: "), "{text}");
        assert!(text.contains("backoff();"), "{text}");
    }

    #[tokio::test]
    async fn search_is_case_insensitive_unless_asked_otherwise() {
        let (dir, ctx) = project();
        populate(dir.path());

        let loose = SearchTool
            .invoke(json!({"pattern": "RETRY"}), &ctx)
            .await
            .unwrap();
        assert!(loose.value["matches"].as_u64().unwrap() >= 3);

        let strict = SearchTool
            .invoke(json!({"pattern": "RETRY", "case_sensitive": true}), &ctx)
            .await
            .unwrap();
        assert_eq!(strict.value["matches"], 1, "only notes.md has uppercase");
    }

    #[tokio::test]
    async fn the_pattern_is_literal_not_a_regular_expression() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.txt"), "a.b\naxb\n").unwrap();

        let outcome = SearchTool
            .invoke(json!({"pattern": "a.b"}), &ctx)
            .await
            .unwrap();

        // A regex engine would match both lines; a literal search matches one.
        assert_eq!(outcome.value["matches"], 1);
        assert!(!text_of(&outcome).contains("axb"));
    }

    #[tokio::test]
    async fn an_extension_filter_narrows_the_walk() {
        let (dir, ctx) = project();
        populate(dir.path());

        let outcome = SearchTool
            .invoke(json!({"pattern": "retry", "extension": "md"}), &ctx)
            .await
            .unwrap();
        let text = text_of(&outcome);
        assert!(text.contains("notes.md"), "{text}");
        assert!(!text.contains(".rs"), "{text}");
    }

    #[tokio::test]
    async fn no_matches_says_where_it_looked() {
        let (dir, ctx) = project();
        populate(dir.path());

        let outcome = SearchTool
            .invoke(json!({"pattern": "quantum"}), &ctx)
            .await
            .unwrap();
        let text = text_of(&outcome);

        assert_eq!(outcome.value["matches"], 0);
        assert!(text.contains("no matches"), "{text}");
        assert!(
            text.contains("file(s)"),
            "an empty result must say how much was searched: {text}"
        );
    }

    #[tokio::test]
    async fn binary_files_are_skipped_rather_than_matched_as_garbage() {
        let (dir, ctx) = project();
        std::fs::write(dir.path().join("a.bin"), b"needle\x00\x01\x02needle").unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle\n").unwrap();

        let outcome = SearchTool
            .invoke(json!({"pattern": "needle"}), &ctx)
            .await
            .unwrap();
        assert_eq!(outcome.value["matches"], 1);
        assert!(!text_of(&outcome).contains("a.bin"));
    }

    #[tokio::test]
    async fn a_limit_stops_the_search_and_says_so() {
        let (dir, ctx) = project();
        let body: String = (0..200).map(|_| "needle\n").collect();
        std::fs::write(dir.path().join("many.txt"), body).unwrap();

        let outcome = SearchTool
            .invoke(json!({"pattern": "needle", "limit": 5}), &ctx)
            .await
            .unwrap();

        assert_eq!(outcome.value["matches"], 5);
        assert_eq!(outcome.value["truncated"], true);
        assert!(text_of(&outcome).contains("[stopped at 5 matches]"));
    }

    #[tokio::test]
    async fn a_very_long_match_line_is_trimmed() {
        let (dir, ctx) = project();
        let long = format!("{}needle{}", "x".repeat(500), "y".repeat(500));
        std::fs::write(dir.path().join("min.js"), long).unwrap();

        let text = text_of(
            &SearchTool
                .invoke(json!({"pattern": "needle"}), &ctx)
                .await
                .unwrap(),
        );
        assert!(text.contains('…'), "{text}");
        assert!(text.chars().count() < 500, "the line must be trimmed");
    }

    #[tokio::test]
    async fn an_empty_pattern_is_rejected() {
        let (_dir, ctx) = project();
        assert!(
            SearchTool
                .invoke(json!({"pattern": ""}), &ctx)
                .await
                .is_err()
        );
    }
}
