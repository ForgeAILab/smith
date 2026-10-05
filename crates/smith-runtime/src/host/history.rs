use super::*;

pub(super) fn append_shell_shortcut(
    path: &Path,
    record: &SavedShellShortcut,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(record).map_err(std::io::Error::other)?;
    line.push(b'\n');
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(&line)?;
    file.sync_data()
}

pub(super) fn tool_call_display_from_history(
    history: &[Message],
    call_id: &ToolCallId,
    redactor: &DefaultRedactor,
) -> Option<ToolCallDisplay> {
    history.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|part| {
            let ContentPart::ToolCall(call) = part else {
                return None;
            };
            (call.id == *call_id)
                .then(|| {
                    let arguments = redactor.redacted_clone(&call.arguments);
                    project_tool_call_display(&call.name, &arguments)
                })
                .flatten()
        })
    })
}

pub(super) fn tool_result_text_from_history(
    history: &[Message],
    call_id: &ToolCallId,
    redactor: &DefaultRedactor,
) -> Option<String> {
    history.iter().rev().find_map(|message| {
        message.content.iter().rev().find_map(|part| {
            let ContentPart::ToolResult(result) = part else {
                return None;
            };
            (result.call_id == *call_id)
                .then(|| redacted_result_text(result, redactor))
                .flatten()
        })
    })
}

pub(super) fn redacted_result_text(
    result: &agent_runtime_core::content::ToolResultBlock,
    redactor: &DefaultRedactor,
) -> Option<String> {
    let text = result
        .content
        .iter()
        .filter_map(ContentPart::as_text)
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return None;
    }
    match redactor.redacted_clone(&serde_json::Value::String(text)) {
        serde_json::Value::String(redacted) => Some(redacted),
        _ => None,
    }
}

pub(super) fn tool_call_displays_from_history(
    history: &[Message],
    redactor: &DefaultRedactor,
) -> Vec<(ToolCallId, ToolCallDisplay)> {
    history
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| {
            let ContentPart::ToolCall(call) = part else {
                return None;
            };
            let arguments = redactor.redacted_clone(&call.arguments);
            project_tool_call_display(&call.name, &arguments)
                .map(|display| (call.id.clone(), display))
        })
        .collect()
}
