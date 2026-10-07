//! Redaction-safe display projections for Smith's built-in tools, and for the
//! tools an installed coding agent reports running inside a harness turn.
//!
//! A tool's canonical arguments can contain arbitrary model-generated text.
//! This module therefore does not summarize JSON generically: every displayed
//! field is explicitly selected beside the built-in tool schema that gives it
//! meaning. Callers must credential-redact canonical arguments before passing
//! them here.

use agent_runtime_core::delegation::{ToolViewScope, WorkspacePolicy};
use agent_runtime_core::security::SecurityResource;
use agent_runtime_core::tool::PreparedToolCall;
use serde_json::{Map, Value};

const MAX_VALUE_CHARS: usize = 160;

/// A reviewed, bounded description of one built-in tool invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallDisplay {
    label: &'static str,
    target: String,
    qualifiers: Vec<String>,
    edit_lines: Option<(usize, usize)>,
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

    /// A compact successful result when raw output adds no useful preview.
    /// Only reviewed tool formats are counted; arbitrary output stays text.
    pub fn result_summary(&self, output: &str) -> Option<String> {
        match self.label {
            "Read" => {
                let mut count = 0usize;
                for line in output.lines().filter(|line| !line.trim().is_empty()) {
                    if line.starts_with('[') && line.ends_with(']') {
                        continue;
                    }
                    let number = line.trim_start().split_once(char::is_whitespace)?.0;
                    number.parse::<usize>().ok()?;
                    count += 1;
                }
                (count > 0).then(|| format!("Read {count} lines"))
            }
            "Update" => {
                let (added, removed) = self.edit_lines?;
                let replacements = output
                    .trim()
                    .strip_prefix("edited `")?
                    .rsplit_once("` (")?
                    .1
                    .strip_suffix(" replacement(s))")?
                    .parse::<usize>()
                    .ok()?;
                let additions = added.checked_mul(replacements)?;
                let removals = removed.checked_mul(replacements)?;
                Some(format!(
                    "Updated {} with {additions} addition{} and {removals} removal{}",
                    self.target,
                    if additions == 1 { "" } else { "s" },
                    if removals == 1 { "" } else { "s" },
                ))
            }
            "Agent" if self.target == "spawn" => {
                let result: Value = serde_json::from_str(output).ok()?;
                let fields = result.as_object()?;
                // Only the reviewed successful spawn response has this shape.
                // Errors and future result shapes keep the raw-output fallback.
                if fields.len() != 2
                    || fields.get("note")?.as_str()?
                        != "the result will be delivered when the child completes"
                {
                    return None;
                }
                let child = normalize_value(fields.get("spawned")?.as_str()?)?;
                Some(format!(
                    "{child} started · its result arrives when it completes"
                ))
            }
            _ => None,
        }
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

/// Projects the target of Smith's workspace-bound filesystem approvals.
/// Preparation has already stored the project root as the resource's mount
/// and the canonical relative components as its segments. Other tools retain
/// their absolute/opaque security-resource display in the caller.
pub fn approval_path_display(prepared: &PreparedToolCall) -> Option<String> {
    if !matches!(prepared.tool(), "read" | "list" | "search" | "edit") {
        return None;
    }
    let SecurityResource::Filesystem { segments, .. } = prepared.resource() else {
        return None;
    };
    Some(if segments.is_empty() {
        ".".to_owned()
    } else {
        segments.join("/")
    })
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
        "generate_image" => project_generate_image(arguments),
        "registry.search" => project_registry_search(arguments),
        "registry.activate" => project_registry_activate(arguments),
        "agent" => project_agent(arguments),
        "advisor" if arguments.is_empty() => Some(display("Advisor", String::new(), Vec::new())),
        _ => None,
    }
}

/// The reviewed label also used when the invocation's values are protected.
pub fn tool_display_label(name: &str) -> Option<&'static str> {
    match name {
        "shell" => Some("Bash"),
        "read" => Some("Read"),
        "edit" => Some("Update"),
        "search" => Some("Search"),
        "list" => Some("List"),
        "agent" => Some("Agent"),
        "advisor" => Some("Advisor"),
        "task_output" => Some("Task Output"),
        "task_stop" => Some("Task Stop"),
        "generate_image" => Some("Generate Image"),
        "registry.search" => Some("Registry Search"),
        "registry.activate" => Some("Activate"),
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
            | "generate_image"
            | "registry.search"
            | "registry.activate"
            | "agent"
            | "advisor"
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
    let old = require_string_field(arguments, "old_string")?;
    let new = require_string_field(arguments, "new_string")?;
    let target = required_target(arguments, "path")?;
    let replace_all = optional_boolean(arguments, "replace_all")?;
    let qualifiers = if replace_all == Some(true) {
        vec!["replace all".to_owned()]
    } else {
        Vec::new()
    };
    let mut projected = display("Update", target, qualifiers);
    projected.edit_lines = edit_line_changes(old, new);
    Some(projected)
}

fn edit_line_changes(old: &str, new: &str) -> Option<(usize, usize)> {
    // Redaction can make distinct changed lines look identical. Large edits
    // also keep their existing replacement summary instead of an estimate.
    if old.contains("[redacted]") || new.contains("[redacted]") {
        return None;
    }
    let old = old.lines().collect::<Vec<_>>();
    let new = new.lines().collect::<Vec<_>>();
    if old.len().checked_mul(new.len())? > 1_000_000 {
        return None;
    }
    let mut previous = vec![0usize; new.len() + 1];
    let mut current = previous.clone();
    for before in &old {
        for (index, after) in new.iter().enumerate() {
            current[index + 1] = if before == after {
                previous[index] + 1
            } else {
                current[index].max(previous[index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let unchanged = previous[new.len()];
    Some((new.len() - unchanged, old.len() - unchanged))
}

/// The runtime's capability-discovery bootstrap (`registry.search`) is a
/// first-party tool with a reviewed schema: an optional `query` (absent means
/// "list what exists"), plus optional `domain`, `offset`, and `max_results`.
fn project_registry_search(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = match optional_value(arguments, "query")? {
        Some(query) => serde_json::to_string(&query).ok()?,
        None => "all".to_owned(),
    };
    let domain = optional_value(arguments, "domain")?;
    let offset = optional_non_negative_integer(arguments, "offset")?;
    let max_results = optional_positive_integer(arguments, "max_results")?;
    let mut qualifiers = Vec::new();
    if let Some(domain) = domain {
        qualifiers.push(domain);
    }
    if let Some(offset) = offset.filter(|offset| *offset > 0) {
        qualifiers.push(format!("from {offset}"));
    }
    if let Some(max_results) = max_results {
        qualifiers.push(format!("max {max_results}"));
    }
    Some(display("Registry Search", target, qualifiers))
}

/// `registry.activate` names the capabilities the agent chose to turn on.
fn project_registry_activate(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let ids = arguments
        .get("ids")?
        .as_array()?
        .iter()
        .map(|id| id.as_str().and_then(normalize_value))
        .collect::<Option<Vec<_>>>()?;
    if ids.is_empty() {
        return None;
    }
    Some(display("Activate", ids.join(", "), Vec::new()))
}

fn project_shell(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_value(arguments, "command")?;
    let cwd = optional_target(arguments, "cwd", ".")?;
    let timeout = optional_positive_integer(arguments, "timeout_ms")?;
    let mut qualifiers = Vec::new();
    if cwd != "." {
        qualifiers.push(format!("cwd {cwd}"));
    }
    if let Some(timeout) = timeout {
        qualifiers.push(format!("timeout {timeout}ms"));
    }
    Some(display("Bash", target, qualifiers))
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

fn project_generate_image(arguments: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let prompt = required_value(arguments, "prompt")?;
    let target = serde_json::to_string(&prompt).ok()?;
    let mut qualifiers = Vec::new();
    if let Some(paths) = arguments.get("reference_paths") {
        let paths = paths.as_array()?;
        if paths.len() > 5 || paths.iter().any(|path| !path.is_string()) {
            return None;
        }
        if !paths.is_empty() {
            qualifiers.push(format!("{} reference image(s)", paths.len()));
        }
    }
    if let Some(count) = optional_positive_integer(arguments, "recent_images")? {
        if count > 5 {
            return None;
        }
        qualifiers.push(format!("{count} recent image(s)"));
    }
    Some(display("Generate Image", target, qualifiers))
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
    let scope = match arguments.get("tools") {
        None => ToolViewScope::ReadOnly,
        Some(Value::String(value)) if value == "read_only" => ToolViewScope::ReadOnly,
        Some(Value::String(value)) if value == "all" => ToolViewScope::All,
        _ => return None,
    };
    agent_tool_scope_display(&scope)
}

/// Tool-scope words shared by spawn rows and prepared child approvals.
fn agent_tool_scope_display(scope: &ToolViewScope) -> Option<String> {
    match scope {
        ToolViewScope::All => Some("all".to_owned()),
        ToolViewScope::ReadOnly => Some("read only".to_owned()),
        ToolViewScope::Named { names } => {
            let names = names
                .iter()
                .map(|name| normalize_value(name))
                .collect::<Option<Vec<_>>>()?;
            normalize_value(&names.join(", "))
        }
    }
}

/// `workspace` is either one of two fixed labels or a `{"directory": {"path":
/// …}}` object; a value outside that shape is ill-typed for this field.
fn agent_workspace(arguments: &Map<String, Value>) -> Option<String> {
    let workspace = match arguments.get("workspace") {
        None => WorkspacePolicy::ReadOnlyView,
        Some(Value::String(value)) if value == "shared" => WorkspacePolicy::SharedProject,
        Some(Value::String(value)) if value == "read_only" => WorkspacePolicy::ReadOnlyView,
        Some(Value::Object(object)) => {
            let path = object
                .get("directory")?
                .as_object()?
                .get("path")?
                .as_str()?;
            WorkspacePolicy::ExplicitDirectory {
                path: path.to_owned(),
            }
        }
        _ => return None,
    };
    agent_workspace_display(&workspace)
}

/// Workspace words shared by reviewed spawn rows and child details.
/// Directory paths use the same bounds and control normalization as tool inputs.
pub fn agent_workspace_display(workspace: &WorkspacePolicy) -> Option<String> {
    match workspace {
        WorkspacePolicy::SharedProject => Some("shared".to_owned()),
        WorkspacePolicy::ExplicitDirectory { path } => normalize_value(path),
        WorkspacePolicy::IsolatedWorktree => Some("isolated worktree".to_owned()),
        WorkspacePolicy::ReadOnlyView => Some("read only".to_owned()),
    }
}

fn display(label: &'static str, target: String, qualifiers: Vec<String>) -> ToolCallDisplay {
    ToolCallDisplay {
        label,
        target,
        qualifiers,
        edit_lines: None,
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
    normalize_value_with_limit(raw, MAX_VALUE_CHARS)
}

fn normalize_value_with_limit(raw: &str, limit: usize) -> Option<String> {
    let mut normalized = String::with_capacity(raw.len().min(limit));
    let mut chars = 0usize;
    let mut pending_space = false;
    let mut truncated = false;

    for character in raw.chars() {
        if is_unsafe_control(character) || character.is_whitespace() {
            pending_space = !normalized.is_empty();
            continue;
        }
        if pending_space {
            if chars == limit {
                truncated = true;
                break;
            }
            normalized.push(' ');
            chars += 1;
            pending_space = false;
        }
        if chars == limit {
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
        if chars == limit {
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
mod tests;

#[cfg(test)]
mod result_tests;

mod delegation;
mod external;

pub use delegation::{DelegationApprovalDisplay, project_delegation_approval_display};
pub use external::{external_tool_result_text, project_external_tool_call_display};
