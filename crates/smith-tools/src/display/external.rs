use super::*;
/// Projects one tool an installed coding agent reported running itself.
///
/// This is the same explicit-selection rule the projectors above follow, over
/// a different vocabulary: the schema giving each field meaning is the CLI
/// vendor's, and the values are reported by a program Smith did not dispatch
/// and cannot vouch for. Every displayed field is therefore still named here
/// beside the tool that defines it, normalized, and bounded — nothing is
/// summarized generically. A tool or shape this build does not know returns
/// `None`, which leaves the caller its value-free fallback.
///
/// `name` and `detail` are the pair the agent reported: for Claude Code the
/// tool name and its tool-use input, for Codex the item type and the item.
pub fn project_external_tool_call_display(name: &str, detail: &Value) -> Option<ToolCallDisplay> {
    let detail = detail.as_object()?;
    match name {
        // Claude Code.
        "Read" => project_claude_read(detail),
        "Write" => project_claude_path("Write", detail, "file_path"),
        "Edit" | "MultiEdit" => project_claude_edit(detail),
        "NotebookEdit" => project_claude_path("Notebook Edit", detail, "notebook_path"),
        "Bash" => project_claude_bash(detail),
        "BashOutput" => project_claude_path("Bash Output", detail, "bash_id"),
        "KillShell" => project_claude_path("Kill Shell", detail, "shell_id"),
        "Glob" => project_claude_glob(detail),
        "Grep" => project_claude_grep(detail),
        "WebFetch" => project_claude_path("Web Fetch", detail, "url"),
        "WebSearch" => project_claude_path("Web Search", detail, "query"),
        "Task" => project_claude_task(detail),
        "TodoWrite" => project_claude_todo_write(detail),
        "SlashCommand" => project_claude_path("Slash Command", detail, "command"),
        // Codex.
        "command_execution" => project_codex_command(detail),
        "file_change" => project_codex_file_change(detail),
        "mcp_tool_call" => project_codex_mcp(detail),
        _ => None,
    }
}

/// The text an installed agent reported as one tool's outcome.
///
/// Claude Code reports either a plain string or a list of content blocks;
/// Codex reports its aggregated output as a string. Anything else — an image
/// block, a shape this build does not know — yields no preview rather than a
/// generic rendering of JSON. The caller bounds and sanitizes what comes back
/// exactly as it does a built-in tool's result.
pub fn external_tool_result_text(detail: &Value) -> Option<String> {
    match detail {
        Value::String(text) => non_empty(text.clone()),
        Value::Array(blocks) => {
            let text = blocks
                .iter()
                .filter_map(|block| block.as_object()?.get("text")?.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            non_empty(text)
        }
        Value::Object(object) => non_empty(object.get("text")?.as_str()?.to_owned()),
        _ => None,
    }
}

fn non_empty(text: String) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    // A tab is horizontal whitespace, not a control signal, and agent output
    // leans on it: Claude Code's file reads are `<line>\t<text>`, which a
    // control-stripping preview would render as `1fn main()`. Turning it into
    // a space keeps the columns apart without keeping the control character.
    Some(text.replace('\t', " "))
}

fn project_claude_read(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_target(detail, "file_path")?;
    let offset = optional_positive_integer(detail, "offset")?;
    let limit = optional_positive_integer(detail, "limit")?;
    let mut qualifiers = Vec::new();
    if let Some(offset) = offset {
        qualifiers.push(format!("offset {offset}"));
    }
    if let Some(limit) = limit {
        qualifiers.push(format!("limit {limit}"));
    }
    Some(display("Read", target, qualifiers))
}

/// The agent tools whose whole reviewed shape is one named string: a path, an
/// identifier, a URL, a query.
fn project_claude_path(
    label: &'static str,
    detail: &Map<String, Value>,
    key: &str,
) -> Option<ToolCallDisplay> {
    let target = required_target(detail, key)?;
    Some(display(label, target, Vec::new()))
}

fn project_claude_edit(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_target(detail, "file_path")?;
    let replace_all = optional_boolean(detail, "replace_all")?;
    let mut qualifiers = Vec::new();
    // `MultiEdit` carries a list rather than one replacement; its length is
    // the reviewed fact, not the edits themselves.
    if let Some(edits) = detail.get("edits") {
        qualifiers.push(format!("{} edits", edits.as_array()?.len()));
    }
    if replace_all == Some(true) {
        qualifiers.push("replace all".to_owned());
    }
    Some(display("Update", target, qualifiers))
}

fn project_claude_bash(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_value(detail, "command")?;
    let background = optional_boolean(detail, "run_in_background")?;
    let timeout = optional_positive_integer(detail, "timeout")?;
    let mut qualifiers = Vec::new();
    if background == Some(true) {
        qualifiers.push("background".to_owned());
    }
    if let Some(timeout) = timeout {
        qualifiers.push(format!("timeout {timeout}ms"));
    }
    Some(display("Bash", target, qualifiers))
}

fn project_claude_glob(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_value(detail, "pattern")?;
    let path = optional_value(detail, "path")?;
    Some(display("Glob", target, path.into_iter().collect()))
}

fn project_claude_grep(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_value(detail, "pattern")?;
    let mut qualifiers = Vec::new();
    if let Some(path) = optional_value(detail, "path")? {
        qualifiers.push(path);
    }
    if let Some(glob) = optional_value(detail, "glob")? {
        qualifiers.push(format!("glob {glob}"));
    }
    if let Some(mode) = optional_value(detail, "output_mode")? {
        qualifiers.push(mode);
    }
    Some(display("Grep", target, qualifiers))
}

/// A sub-agent the CLI spawned inside its own turn. The description is the
/// reviewed field; the prompt it was given is not displayed, exactly as
/// Smith's own spawn shows a bounded excerpt rather than the whole task.
fn project_claude_task(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_value(detail, "description")?;
    let subagent = optional_value(detail, "subagent_type")?;
    Some(display("Task", target, subagent.into_iter().collect()))
}

fn project_claude_todo_write(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let todos = detail.get("todos")?.as_array()?.len();
    Some(display(
        "Todo Write",
        format!("{todos} item{}", if todos == 1 { "" } else { "s" }),
        Vec::new(),
    ))
}

fn project_codex_command(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let target = required_value(detail, "command")?;
    // The exit code is not a qualifier: the row's own status already carries
    // whether the command succeeded, from the same field.
    Some(display("Command", target, Vec::new()))
}

fn project_codex_file_change(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let changes = detail.get("changes")?.as_array()?;
    let first = changes.first()?.as_object()?;
    let target = required_target(first, "path")?;
    let mut qualifiers = Vec::new();
    if let Some(kind) = optional_value(first, "kind")? {
        qualifiers.push(kind);
    }
    if changes.len() > 1 {
        qualifiers.push(format!("+{} more", changes.len() - 1));
    }
    Some(display("File Change", target, qualifiers))
}

fn project_codex_mcp(detail: &Map<String, Value>) -> Option<ToolCallDisplay> {
    let server = required_value(detail, "server")?;
    let tool = required_value(detail, "tool")?;
    Some(display("MCP", format!("{server}/{tool}"), Vec::new()))
}
