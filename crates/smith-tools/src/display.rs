//! Redaction-safe display projections for Smith's built-in tools.
//!
//! A tool's canonical arguments can contain arbitrary model-generated text.
//! This module therefore does not summarize JSON generically: every displayed
//! field is explicitly allowlisted beside the built-in tool schema that gives
//! it meaning.

use serde_json::{Map, Value};

const MAX_TARGET_CHARS: usize = 160;

/// A reviewed, bounded description of one built-in tool invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallDisplay {
    label: &'static str,
    target: String,
    qualifiers: Vec<String>,
}

impl ToolCallDisplay {
    /// Human-readable tool label.
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// Actionable local target, such as a path or working directory.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Reviewed numeric or boolean invocation details.
    pub fn qualifiers(&self) -> &[String] {
        &self.qualifiers
    }

    /// The compact invocation portion of a transcript row.
    pub fn invocation(&self) -> String {
        let mut details = Vec::with_capacity(self.qualifiers.len() + 1);
        details.push(self.target.as_str());
        details.extend(self.qualifiers.iter().map(String::as_str));
        format!("{}({})", self.label, details.join(" · "))
    }
}

/// Projects canonical arguments into a display-safe built-in invocation.
///
/// Unknown tools, malformed calls, and ill-typed allowlisted fields return
/// `None`, leaving the caller free to use a value-free protected fallback.
pub fn project_tool_call_display(name: &str, arguments: &Value) -> Option<ToolCallDisplay> {
    let arguments = arguments.as_object()?;
    match name {
        "read" => project_read(arguments),
        "list" => project_list(arguments),
        "search" => project_search(arguments),
        "edit" => project_edit(arguments),
        "shell" => project_shell(arguments),
        _ => None,
    }
}

fn project_read(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_target(arguments, "path")?;
    let offset = optional_positive_integer(arguments, "offset")?;
    let limit = optional_positive_integer(arguments, "limit")?;
    let qualifiers = match (offset, limit) {
        (Some(offset), Some(limit)) => vec![format!(
            "lines {offset}–{}",
            offset.saturating_add(limit.saturating_sub(1))
        )],
        (Some(offset), None) => vec![format!("from line {offset}")],
        (None, Some(limit)) => vec![format!("first {limit} lines")],
        (None, None) => Vec::new(),
    };
    Some(display("Read", target, qualifiers))
}

fn project_list(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = optional_target(arguments, "path", ".")?;
    let recursive = optional_boolean(arguments, "recursive")?;
    let all = optional_boolean(arguments, "all")?;
    let limit = optional_positive_integer(arguments, "limit")?;
    let mut qualifiers = Vec::new();
    if recursive == Some(true) {
        qualifiers.push("recursive".to_owned());
    }
    if all == Some(true) {
        qualifiers.push("all".to_owned());
    }
    if let Some(limit) = limit {
        qualifiers.push(format!("limit {limit}"));
    }
    Some(display("List", target, qualifiers))
}

fn project_search(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    require_string_field(arguments, "pattern")?;
    let target = optional_target(arguments, "path", ".")?;
    let case_sensitive = optional_boolean(arguments, "case_sensitive")?;
    let limit = optional_positive_integer(arguments, "limit")?;
    let mut qualifiers = Vec::new();
    if case_sensitive == Some(true) {
        qualifiers.push("case sensitive".to_owned());
    }
    if let Some(limit) = limit {
        qualifiers.push(format!("limit {limit}"));
    }
    Some(display("Search", target, qualifiers))
}

fn project_edit(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    require_string_field(arguments, "old_string")?;
    require_string_field(arguments, "new_string")?;
    let target = required_target(arguments, "path")?;
    let replace_all = optional_boolean(arguments, "replace_all")?;
    let qualifiers = if replace_all == Some(true) {
        vec!["replace all".to_owned()]
    } else {
        Vec::new()
    };
    Some(display("Edit", target, qualifiers))
}

fn project_shell(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    require_string_field(arguments, "command")?;
    let target = optional_target(arguments, "cwd", ".")?;
    let timeout = optional_positive_integer(arguments, "timeout_ms")?;
    let qualifiers = timeout
        .map(|timeout| vec![format!("timeout {timeout} ms")])
        .unwrap_or_default();
    Some(display("Shell", target, qualifiers))
}

fn display(label: &'static str, target: String, qualifiers: Vec<String>) -> ToolCallDisplay {
    ToolCallDisplay {
        label,
        target,
        qualifiers,
    }
}

fn required_target(arguments: &Map<String, Value>, key: &str) -> Option<String> {
    normalize_target(require_string_field(arguments, key)?)
}

fn optional_target(arguments: &Map<String, Value>, key: &str, default: &str) -> Option<String> {
    match arguments.get(key) {
        Some(Value::String(value)) => normalize_target(value),
        Some(_) => None,
        None => normalize_target(default),
    }
}

fn require_string_field<'a>(arguments: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    arguments.get(key)?.as_str()
}

fn optional_boolean(arguments: &Map<String, Value>, key: &str) -> Option<Option<bool>> {
    match arguments.get(key) {
        Some(Value::Bool(value)) => Some(Some(*value)),
        Some(_) => None,
        None => Some(None),
    }
}

fn optional_positive_integer(arguments: &Map<String, Value>, key: &str) -> Option<Option<u64>> {
    match arguments.get(key) {
        Some(Value::Number(value)) => value.as_u64().filter(|value| *value > 0).map(Some),
        Some(_) => None,
        None => Some(None),
    }
}

fn normalize_target(raw: &str) -> Option<String> {
    let mut normalized = String::with_capacity(raw.len().min(MAX_TARGET_CHARS));
    let mut chars = 0usize;
    let mut pending_space = false;
    let mut truncated = false;

    for character in raw.chars() {
        if is_unsafe_control(character) || character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            if chars == MAX_TARGET_CHARS {
                truncated = true;
                break;
            }
            normalized.push(' ');
            chars += 1;
            pending_space = false;
        }
        if chars == MAX_TARGET_CHARS {
            truncated = true;
            break;
        }
        normalized.push(character);
        chars += 1;
    }

    if normalized.is_empty() {
        return None;
    }
    if truncated {
        if normalized.ends_with(' ') {
            normalized.pop();
            chars = chars.saturating_sub(1);
        }
        if chars == MAX_TARGET_CHARS {
            normalized.pop();
        }
        normalized.push('…');
    }
    Some(normalized)
}

fn is_unsafe_control(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn invocation(name: &str, arguments: Value) -> String {
        project_tool_call_display(name, &arguments)
            .expect("the call should have a reviewed projection")
            .invocation()
    }

    #[test]
    fn every_built_in_projects_only_its_reviewed_target_and_qualifiers() {
        assert_eq!(
            invocation(
                "read",
                json!({"path": "src/lib.rs", "offset": 10, "limit": 5})
            ),
            "Read(src/lib.rs · lines 10–14)"
        );
        assert_eq!(
            invocation(
                "list",
                json!({"path": "src", "recursive": true, "all": true, "limit": 25})
            ),
            "List(src · recursive · all · limit 25)"
        );
        assert_eq!(
            invocation(
                "search",
                json!({
                    "pattern": "TOP_SECRET_PATTERN",
                    "path": "crates",
                    "extension": "TOP_SECRET_EXTENSION",
                    "case_sensitive": true,
                    "limit": 8
                })
            ),
            "Search(crates · case sensitive · limit 8)"
        );
        assert_eq!(
            invocation(
                "edit",
                json!({
                    "path": "src/config.rs",
                    "old_string": "TOP_SECRET_OLD",
                    "new_string": "TOP_SECRET_NEW",
                    "replace_all": true
                })
            ),
            "Edit(src/config.rs · replace all)"
        );
        assert_eq!(
            invocation(
                "shell",
                json!({
                    "command": "printf TOP_SECRET_COMMAND",
                    "cwd": "crates/smith-cli",
                    "timeout_ms": 3000
                })
            ),
            "Shell(crates/smith-cli · timeout 3000 ms)"
        );
    }

    #[test]
    fn optional_root_targets_have_a_stable_default() {
        assert_eq!(
            invocation("list", json!({"recursive": true})),
            "List(. · recursive)"
        );
        assert_eq!(
            invocation("search", json!({"pattern": "hidden"})),
            "Search(.)"
        );
        assert_eq!(
            invocation("shell", json!({"command": "hidden"})),
            "Shell(.)"
        );
    }

    #[test]
    fn arbitrary_values_never_enter_the_projection() {
        let secrets = [
            "TOP_SECRET_PATTERN",
            "TOP_SECRET_EXTENSION",
            "TOP_SECRET_OLD",
            "TOP_SECRET_NEW",
            "TOP_SECRET_COMMAND",
            "TOP_SECRET_RESULT",
            "TOP_SECRET_UNKNOWN",
        ];
        let calls = [
            invocation(
                "search",
                json!({
                    "pattern": secrets[0],
                    "extension": secrets[1],
                    "result": secrets[5],
                    "unknown": secrets[6]
                }),
            ),
            invocation(
                "edit",
                json!({
                    "path": "safe.rs",
                    "old_string": secrets[2],
                    "new_string": secrets[3],
                    "result": secrets[5],
                    "unknown": secrets[6]
                }),
            ),
            invocation(
                "shell",
                json!({
                    "command": secrets[4],
                    "result": secrets[5],
                    "unknown": secrets[6]
                }),
            ),
        ];

        for call in calls {
            for secret in secrets {
                assert!(!call.contains(secret), "{secret} leaked through `{call}`");
            }
        }
    }

    #[test]
    fn targets_are_one_line_control_free_and_bounded() {
        let raw = format!(
            "{}\nnext\t\u{1b}[31m\u{202e}tail",
            "a".repeat(MAX_TARGET_CHARS * 2)
        );
        let display =
            project_tool_call_display("read", &json!({"path": raw})).expect("path should project");

        assert!(display.target().chars().count() <= MAX_TARGET_CHARS);
        assert!(display.target().ends_with('…'));
        assert!(
            !display
                .target()
                .chars()
                .any(|character| character.is_control())
        );
        assert!(!display.target().contains('\u{202e}'));
        assert!(!display.invocation().contains('\n'));
    }

    #[test]
    fn controls_inside_a_short_target_are_collapsed_to_spaces() {
        assert_eq!(
            invocation(
                "read",
                json!({"path": "src/\nsecret\t\u{1b}[31m.rs\u{202e}"})
            ),
            "Read(src/ secret [31m.rs)"
        );
    }

    #[test]
    fn malformed_or_unknown_calls_keep_the_caller_on_its_fallback_path() {
        assert!(project_tool_call_display("third_party", &json!({"path": "safe"})).is_none());
        assert!(project_tool_call_display("read", &json!({"path": 42})).is_none());
        assert!(project_tool_call_display("list", &json!({"recursive": "yes"})).is_none());
        assert!(
            project_tool_call_display("shell", &json!({"command": "ok", "timeout_ms": 0}))
                .is_none()
        );
        assert!(project_tool_call_display("search", &json!({"path": "."})).is_none());
    }
}
