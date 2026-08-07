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

    /// Appends one more qualifier to an already-projected row.
    ///
    /// This exists for enrichment after the fact: a delegation spawn row is
    /// projected from the call's own arguments before the runtime confirms
    /// the child, so the projector cannot yet know the child's id, its
    /// resolved workspace posture, or its turn ceiling. Once the runtime
    /// reports those facts, the caller correlates them back to this row by
    /// call id and enriches it in place rather than rendering a second row
    /// for the same spawn. The qualifier is normalized and bounded exactly
    /// like every projector's own qualifiers, so a caller enriching a row
    /// from event data — not from a reviewed schema — cannot smuggle
    /// unbounded text or line, terminal, and bidi control characters onto
    /// the transcript. A qualifier that normalizes to nothing (empty, or
    /// only control/whitespace) is dropped rather than appended.
    pub fn with_qualifier(mut self, qualifier: impl Into<String>) -> Self {
        let qualifier = qualifier.into();
        if let Some(normalized) = normalize_value(&qualifier) {
            self.qualifiers.push(normalized);
        }
        self
    }

    /// Appends several qualifiers in order; see [`Self::with_qualifier`].
    pub fn with_qualifiers<I, S>(self, qualifiers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        qualifiers
            .into_iter()
            .fold(self, |built, qualifier| built.with_qualifier(qualifier))
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
        "task_output" => project_task_output(arguments),
        "task_stop" => project_task_stop(arguments),
        "registry.search" => project_registry_search(arguments),
        "agent" => project_agent(arguments),
        _ => None,
    }
}

/// Whether Smith owns a reviewed display schema for `name`.
pub fn has_tool_call_display_schema(name: &str) -> bool {
    matches!(
        name,
        "read"
            | "list"
            | "search"
            | "edit"
            | "shell"
            | "task_output"
            | "task_stop"
            | "registry.search"
            | "agent"
    )
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

/// The runtime's capability-discovery bootstrap (`registry.search`) is a
/// first-party tool with a reviewed schema: `query` plus an optional
/// `max_results`.
fn project_registry_search(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let query = required_value(arguments, "query")?;
    let target = serde_json::to_string(&query).ok()?;
    let max_results = optional_positive_integer(arguments, "max_results")?;
    let mut qualifiers = Vec::new();
    if let Some(max_results) = max_results {
        qualifiers.push(format!("max {max_results}"));
    }
    Some(display("Registry Search", target, qualifiers))
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

/// `task_output`'s `offset` is 0-based and 0 is its (common) default, unlike
/// `read`'s 1-based `offset` where 0 is nonsensical — so a bare `0` here is a
/// legitimate value, not a signal to fall back like
/// [`optional_positive_integer`] treats it.
fn project_task_output(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_target(arguments, "task_id")?;
    let offset = optional_non_negative_integer(arguments, "offset")?;
    let limit = optional_positive_integer(arguments, "limit")?;
    let mut qualifiers = Vec::new();
    if let Some(offset) = offset.filter(|offset| *offset > 0) {
        qualifiers.push(format!("offset {offset}"));
    }
    if let Some(limit) = limit {
        qualifiers.push(format!("limit {limit}"));
    }
    Some(display("Task Output", target, qualifiers))
}

fn project_task_stop(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_target(arguments, "task_id")?;
    Some(display("Task Stop", target, Vec::new()))
}

/// The delegation tool (`agent`) is dispatched on its own tagged `action`
/// rather than a fixed operation per tool name, so it gets one projector per
/// action instead of one projector per tool. `task` is model-authored free
/// text like `shell`'s command or `search`'s pattern, so it is bounded,
/// control-normalized, and quoted the same way. `action`, `tools`, and the
/// two labelled `workspace` variants come from the small fixed vocabulary
/// the tool schema itself declares, so once validated against that
/// vocabulary they are safe to display verbatim. A `workspace` naming a
/// directory displays its (already-canonicalized) path bounded the same way
/// `project_read` displays a path — this crate does not invent a new
/// convention for showing a location the user already has filesystem-level
/// visibility into. An `action` outside the schema's enum has no reviewed
/// meaning here and falls back like an unknown tool would.
fn project_agent(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    match require_string_field(arguments, "action")? {
        "spawn" => project_agent_spawn(arguments),
        "list" => Some(display("Agent", "list".to_owned(), Vec::new())),
        "wait" => project_agent_child_action(arguments, "wait"),
        "result" => project_agent_child_action(arguments, "result"),
        "follow_up" => project_agent_follow_up(arguments),
        "resume" => project_agent_child_action(arguments, "resume"),
        "stop" => project_agent_child_action(arguments, "stop"),
        _ => None,
    }
}

/// A spawn names its task, its child's tool scope, and its child's
/// workspace posture, in that order, matching the order the lifecycle
/// notice used to carry them. `profile` names a registered child-enabled
/// agent profile the runtime validates on its own; this projector does not
/// re-validate it, and instead treats it as reviewed free text exactly like
/// `task`, so a stale or third-party call cannot smuggle unbounded or
/// control text through an unvalidated `profile` value. An absent profile
/// means the call selected none and contributes no qualifier — the caller
/// (the interactive transcript) is the one that labels an inherited profile
/// as inherited, because this projector cannot see what the parent's
/// profile actually is.
///
/// The scope and workspace qualifiers are labelled rather than bare. Both
/// vocabularies contain `read only`, and the common spawn declares it for
/// both, so unlabelled they render as `… · read only · read only` — two
/// adjacent identical tokens a reader cannot tell apart, let alone match
/// back to the argument each came from.
fn project_agent_spawn(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let task = required_value(arguments, "task")?;
    let excerpt = serde_json::to_string(&task).ok()?;
    let tool_scope = agent_tool_scope(arguments)?;
    let workspace = agent_workspace(arguments)?;
    let mut qualifiers = vec![
        excerpt,
        format!("tools {tool_scope}"),
        format!("workspace {workspace}"),
    ];
    if let Some(profile) = optional_value(arguments, "profile")? {
        qualifiers.push(format!("profile {profile}"));
    }
    Some(display("Agent", "spawn".to_owned(), qualifiers))
}

/// `follow_up` is the one addressed action that also carries free-form task
/// text, so it names its child and then excerpts the task the same way a
/// spawn does.
fn project_agent_follow_up(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let child_id = required_target(arguments, "child_id")?;
    let task = required_value(arguments, "task")?;
    let excerpt = serde_json::to_string(&task).ok()?;
    Some(display(
        "Agent",
        "follow_up".to_owned(),
        vec![child_id, excerpt],
    ))
}

/// `wait`, `result`, `resume`, and `stop` each address exactly one child and
/// carry no other reviewed argument.
fn project_agent_child_action(
    arguments: &Map<String, Value>,
    action: &'static str,
) -> Option<ToolCallDisplay> {
    let child_id = required_target(arguments, "child_id")?;
    Some(display("Agent", action.to_owned(), vec![child_id]))
}

/// `tools` selects a fixed vocabulary (`read_only` defaulting, or `all`), so
/// it is matched rather than normalized: a value outside that vocabulary is
/// ill-typed for this field, not free text to pass through.
fn agent_tool_scope(arguments: &Map<String, Value>) -> Option<String> {
    match arguments.get("tools") {
        None => Some("read only".to_owned()),
        Some(Value::String(value)) if value == "read_only" => Some("read only".to_owned()),
        Some(Value::String(value)) if value == "all" => Some("all".to_owned()),
        _ => None,
    }
}

/// `workspace` is either one of two fixed labels or a `{"directory": {"path":
/// …}}` object; a value outside that shape is ill-typed for this field.
fn agent_workspace(arguments: &Map<String, Value>) -> Option<String> {
    match arguments.get("workspace") {
        None => Some("read only".to_owned()),
        Some(Value::String(value)) if value == "shared" => Some("shared".to_owned()),
        Some(Value::String(value)) if value == "read_only" => Some("read only".to_owned()),
        Some(Value::Object(object)) => {
            let path = object
                .get("directory")?
                .as_object()?
                .get("path")?
                .as_str()?;
            normalize_value(path)
        }
        _ => None,
    }
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

fn optional_non_negative_integer(arguments: &Map<String, Value>, key: &str) -> Option<Option<u64>> {
    match arguments.get(key) {
        Some(Value::Number(value)) => value.as_u64().map(Some),
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
    fn registry_search_projects_its_reviewed_query_and_bound() {
        assert_eq!(
            invocation(
                "registry.search",
                json!({"query": "browser automation", "max_results": 5})
            ),
            "Registry Search(\"browser automation\" · max 5)"
        );
        assert_eq!(
            invocation("registry.search", json!({"query": "sql"})),
            "Registry Search(\"sql\")"
        );
        assert!(has_tool_call_display_schema("registry.search"));
        assert!(project_tool_call_display("registry.search", &json!({"max_results": 3})).is_none());
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

    #[test]
    fn task_output_and_task_stop_project_only_the_task_id_and_reviewed_numbers() {
        assert_eq!(
            invocation(
                "task_output",
                json!({"task_id": "task:1", "offset": 128, "limit": 4096})
            ),
            "Task Output(task:1 · offset 128 · limit 4096)"
        );
        // Offset 0 is the ordinary default, not a signal to fall back — unlike
        // `read`'s 1-based offset, it must still project.
        assert_eq!(
            invocation("task_output", json!({"task_id": "task:1", "offset": 0})),
            "Task Output(task:1)"
        );
        assert_eq!(
            invocation("task_output", json!({"task_id": "task:1"})),
            "Task Output(task:1)"
        );
        assert_eq!(
            invocation(
                "task_stop",
                json!({"task_id": "task:2", "result": "TOP_SECRET_RESULT"})
            ),
            "Task Stop(task:2)"
        );
        assert!(has_tool_call_display_schema("task_output"));
        assert!(has_tool_call_display_schema("task_stop"));
        assert!(project_tool_call_display("task_output", &json!({"offset": 1})).is_none());
        assert!(project_tool_call_display("task_stop", &json!({})).is_none());
    }

    #[test]
    fn agent_spawn_renders_every_reviewed_field() {
        assert_eq!(
            invocation(
                "agent",
                json!({
                    "action": "spawn",
                    "task": "explore the autoloads and data layer",
                    "tools": "all",
                    "workspace": "shared",
                    "profile": "explore"
                })
            ),
            "Agent(spawn · \"explore the autoloads and data layer\" · tools all · workspace shared · profile explore)"
        );
        assert!(has_tool_call_display_schema("agent"));
    }

    #[test]
    fn agent_spawn_defaults_to_read_only_scope_and_workspace() {
        assert_eq!(
            invocation("agent", json!({"action": "spawn", "task": "look around"})),
            "Agent(spawn · \"look around\" · tools read only · workspace read only)"
        );
    }

    #[test]
    fn agent_spawn_directory_workspace_shows_a_bounded_path() {
        assert_eq!(
            invocation(
                "agent",
                json!({
                    "action": "spawn",
                    "task": "build the feature",
                    "workspace": {"directory": {"path": "/repo/crates/smith-tools"}}
                })
            ),
            "Agent(spawn · \"build the feature\" · tools read only · workspace /repo/crates/smith-tools)"
        );
        assert!(
            project_tool_call_display(
                "agent",
                &json!({
                    "action": "spawn",
                    "task": "build the feature",
                    "workspace": {"directory": {}}
                })
            )
            .is_none()
        );
    }

    #[test]
    fn agent_spawn_excerpt_normalizes_control_terminal_and_bidi_characters() {
        let display = project_tool_call_display(
            "agent",
            &json!({
                "action": "spawn",
                "task": "line one\nline two\rcarriage \u{1b}[31mred\u{202e}reversed"
            }),
        )
        .expect("spawn should project");
        let rendered = display.invocation();
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains('\r'));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\u{202e}'));
        assert!(rendered.contains("line one"));
        assert!(rendered.contains("reversed"));
    }

    #[test]
    fn agent_spawn_excerpt_is_bounded_to_one_line() {
        let long_task = "word ".repeat(MAX_VALUE_CHARS);
        let display =
            project_tool_call_display("agent", &json!({"action": "spawn", "task": long_task}))
                .expect("spawn should project");
        let rendered = display.invocation();
        assert!(!rendered.contains('\n'));
        assert!(rendered.contains('…'));
        // The excerpt itself (inside the quotes) must not exceed the shared
        // bound; the surrounding quotes and label are not part of that bound.
        let excerpt = &display.qualifiers()[0];
        assert!(excerpt.chars().count() <= MAX_VALUE_CHARS + 2);
    }

    #[test]
    fn agent_addressed_actions_name_their_child() {
        assert_eq!(
            invocation(
                "agent",
                json!({"action": "follow_up", "child_id": "child-1", "task": "keep going"})
            ),
            "Agent(follow_up · child-1 · \"keep going\")"
        );
        assert_eq!(
            invocation("agent", json!({"action": "stop", "child_id": "child-1"})),
            "Agent(stop · child-1)"
        );
        assert_eq!(
            invocation("agent", json!({"action": "wait", "child_id": "child-2"})),
            "Agent(wait · child-2)"
        );
        assert_eq!(
            invocation("agent", json!({"action": "result", "child_id": "child-2"})),
            "Agent(result · child-2)"
        );
        assert_eq!(
            invocation("agent", json!({"action": "resume", "child_id": "child-3"})),
            "Agent(resume · child-3)"
        );
    }

    #[test]
    fn agent_list_names_no_child_and_no_task() {
        assert_eq!(
            invocation("agent", json!({"action": "list"})),
            "Agent(list)"
        );
    }

    #[test]
    fn agent_rejects_ill_typed_arguments_and_unknown_actions() {
        assert!(project_tool_call_display("agent", &json!({})).is_none());
        assert!(project_tool_call_display("agent", &json!({"action": 1})).is_none());
        assert!(project_tool_call_display("agent", &json!({"action": "teleport"})).is_none());
        assert!(project_tool_call_display("agent", &json!({"action": "spawn"})).is_none());
        assert!(
            project_tool_call_display(
                "agent",
                &json!({"action": "spawn", "task": "ok", "tools": "sudo"})
            )
            .is_none()
        );
        assert!(
            project_tool_call_display(
                "agent",
                &json!({"action": "spawn", "task": "ok", "workspace": "everywhere"})
            )
            .is_none()
        );
        assert!(
            project_tool_call_display("agent", &json!({"action": "stop", "child_id": 5})).is_none()
        );
        assert!(
            project_tool_call_display(
                "agent",
                &json!({"action": "follow_up", "child_id": "child-1"})
            )
            .is_none()
        );
        assert!(project_tool_call_display("agent", &json!({"action": "wait"})).is_none());
    }

    #[test]
    fn with_qualifier_appends_and_normalizes() {
        let display = display("Agent", "spawn".to_owned(), Vec::new())
            .with_qualifier("child-1")
            .with_qualifier("turns 12\nnext line")
            .with_qualifier("");

        assert_eq!(
            display.qualifiers(),
            &["child-1".to_owned(), "turns 12 next line".to_owned()]
        );
        assert_eq!(
            display.invocation(),
            "Agent(spawn · child-1 · turns 12 next line)"
        );
    }

    #[test]
    fn with_qualifiers_appends_several_in_order() {
        let display = display("Agent", "spawn".to_owned(), Vec::new())
            .with_qualifiers(["child-1", "shared", "turns 12"]);

        assert_eq!(
            display.qualifiers(),
            &[
                "child-1".to_owned(),
                "shared".to_owned(),
                "turns 12".to_owned()
            ]
        );
    }
}
