//! Redaction-safe display projections for Smith's built-in tools.
//!
//! A tool's canonical arguments can contain arbitrary model-generated text.
//! This module therefore does not summarize JSON generically: every displayed
//! field is explicitly selected beside the built-in tool schema that gives it
//! meaning. Callers must credential-redact canonical arguments before passing
//! them here.

use serde_json::{Map, Value};

const MAX_VALUE_CHARS: usize = 160;

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

    /// Primary operation input, such as a path, pattern, or command.
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

/// Whether Smith owns a reviewed display schema for `name`.
pub fn has_tool_call_display_schema(name: &str) -> bool {
    matches!(name, "read" | "list" | "search" | "edit" | "shell")
}

fn project_read(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_target(arguments, "path")?;
    let offset = optional_positive_integer(arguments, "offset")?;
    let limit = optional_positive_integer(arguments, "limit")?;
    let mut qualifiers = Vec::new();
    if let Some(offset) = offset {
        qualifiers.push(format!("offset {offset}"));
    }
    if let Some(limit) = limit {
        qualifiers.push(format!("limit {limit}"));
    }
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
    let pattern = required_value(arguments, "pattern")?;
    let target = serde_json::to_string(&pattern).ok()?;
    let path = optional_target(arguments, "path", ".")?;
    let extension = optional_value(arguments, "extension")?;
    let case_sensitive = optional_boolean(arguments, "case_sensitive")?;
    let limit = optional_positive_integer(arguments, "limit")?;
    let mut qualifiers = vec![path];
    if let Some(extension) = extension {
        qualifiers.push(format!("extension {extension}"));
    }
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
    let target = required_value(arguments, "command")?;
    let cwd = optional_target(arguments, "cwd", ".")?;
    let timeout = optional_positive_integer(arguments, "timeout_ms")?;
    let mut qualifiers = vec![format!("cwd {cwd}")];
    if let Some(timeout) = timeout {
        qualifiers.push(format!("timeout {timeout}ms"));
    }
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
    required_value(arguments, key)
}

fn optional_target(arguments: &Map<String, Value>, key: &str, default: &str) -> Option<String> {
    match arguments.get(key) {
        Some(Value::String(value)) => normalize_value(value),
        Some(_) => None,
        None => normalize_value(default),
    }
}

fn required_value(arguments: &Map<String, Value>, key: &str) -> Option<String> {
    normalize_value(require_string_field(arguments, key)?)
}

fn optional_value(arguments: &Map<String, Value>, key: &str) -> Option<Option<String>> {
    match arguments.get(key) {
        Some(Value::String(value)) => normalize_value(value).map(Some),
        Some(_) => None,
        None => Some(None),
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

fn normalize_value(raw: &str) -> Option<String> {
    let mut normalized = String::with_capacity(raw.len().min(MAX_VALUE_CHARS));
    let mut chars = 0usize;
    let mut pending_space = false;
    let mut truncated = false;

    for character in raw.chars() {
        if is_unsafe_control(character) || character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            if chars == MAX_VALUE_CHARS {
                truncated = true;
                break;
            }
            normalized.push(' ');
            chars += 1;
            pending_space = false;
        }
        if chars == MAX_VALUE_CHARS {
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
        if chars == MAX_VALUE_CHARS {
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
            "Read(src/lib.rs · offset 10 · limit 5)"
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
            "Search(\"TOP_SECRET_PATTERN\" · crates · extension TOP_SECRET_EXTENSION · case sensitive · limit 8)"
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
            "Shell(printf TOP_SECRET_COMMAND · cwd crates/smith-cli · timeout 3000ms)"
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
            "Search(\"hidden\" · .)"
        );
        assert_eq!(
            invocation("shell", json!({"command": "hidden"})),
            "Shell(hidden · cwd .)"
        );
    }

    #[test]
    fn ordinary_operation_values_enter_but_bulk_and_unknown_values_do_not() {
        let search = invocation(
            "search",
            json!({
                "pattern": "NEEDLE",
                "extension": "rs",
                "result": "TOP_SECRET_RESULT",
                "unknown": "TOP_SECRET_UNKNOWN"
            }),
        );
        assert!(search.contains("NEEDLE"));
        assert!(search.contains("extension rs"));
        assert!(!search.contains("TOP_SECRET_RESULT"));
        assert!(!search.contains("TOP_SECRET_UNKNOWN"));

        let edit = invocation(
            "edit",
            json!({
                "path": "safe.rs",
                "old_string": "TOP_SECRET_OLD",
                "new_string": "TOP_SECRET_NEW",
                "result": "TOP_SECRET_RESULT",
                "unknown": "TOP_SECRET_UNKNOWN"
            }),
        );
        assert!(!edit.contains("TOP_SECRET_OLD"));
        assert!(!edit.contains("TOP_SECRET_NEW"));
        assert!(!edit.contains("TOP_SECRET_RESULT"));
        assert!(!edit.contains("TOP_SECRET_UNKNOWN"));

        let shell = invocation(
            "shell",
            json!({
                "command": "printf ordinary",
                "result": "TOP_SECRET_RESULT",
                "unknown": "TOP_SECRET_UNKNOWN"
            }),
        );
        assert!(shell.contains("printf ordinary"));
        assert!(!shell.contains("TOP_SECRET_RESULT"));
        assert!(!shell.contains("TOP_SECRET_UNKNOWN"));
    }

    #[test]
    fn credential_redaction_markers_remain_explicit() {
        assert_eq!(
            invocation("search", json!({"pattern": "[redacted]"})),
            "Search(\"[redacted]\" · .)"
        );
        assert_eq!(
            invocation("shell", json!({"command": "curl -H [redacted]"})),
            "Shell(curl -H [redacted] · cwd .)"
        );
    }

    #[test]
    fn targets_are_one_line_control_free_and_bounded() {
        let raw = format!(
            "{}\nnext\t\u{1b}[31m\u{202e}tail",
            "a".repeat(MAX_VALUE_CHARS * 2)
        );
        let display =
            project_tool_call_display("read", &json!({"path": raw})).expect("path should project");

        assert!(display.target().chars().count() <= MAX_VALUE_CHARS);
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
        assert!(has_tool_call_display_schema("read"));
        assert!(!has_tool_call_display_schema("third_party"));
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
