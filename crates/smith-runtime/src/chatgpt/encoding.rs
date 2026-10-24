use super::*;

pub(super) fn response_items(
    message: &Message,
    tool_names: &BTreeMap<String, String>,
) -> Result<Vec<Value>, ProviderError> {
    Ok(match message.role {
        Role::System => {
            let text = message.joined_text();
            if text.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "type": "message",
                    "role": "developer",
                    "content": [{"type": "input_text", "text": text}],
                })]
            }
        }
        Role::User => vec![json!({
            "type": "message",
            "role": "user",
            "content": message.content.iter().filter_map(|part| match part {
                ContentPart::Text { text } => Some(json!({"type": "input_text", "text": text})),
                ContentPart::Image { url, detail } => Some(json!({
                    "type": "input_image",
                    "image_url": url,
                    "detail": detail.as_deref().unwrap_or("auto"),
                })),
                _ => None,
            }).collect::<Vec<_>>(),
        })],
        Role::Assistant => {
            let mut items = Vec::new();
            let mut content = Vec::new();
            for part in &message.content {
                match part {
                    ContentPart::Text { text } => {
                        content
                            .push(json!({"type": "output_text", "text": text, "annotations": []}));
                    }
                    ContentPart::Reasoning {
                        signature: Some(signature),
                        ..
                    } => {
                        flush_assistant_text(&mut items, &mut content);
                        items.push(json!({
                            "type": "reasoning",
                            "summary": [],
                            "encrypted_content": signature,
                        }));
                    }
                    ContentPart::ToolCall(call) => {
                        flush_assistant_text(&mut items, &mut content);
                        let wire = response_wire_tool_name(&call.name);
                        if tool_names
                            .get(&wire)
                            .is_some_and(|canonical| canonical != &call.name)
                        {
                            return Err(tool_name_collision());
                        }
                        items.push(json!({
                            "type": "function_call",
                            "call_id": call.id.as_str(),
                            "name": wire,
                            "arguments": call.arguments.to_string(),
                        }));
                    }
                    _ => {}
                }
            }
            flush_assistant_text(&mut items, &mut content);
            items
        }
        Role::Tool => message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolResult(result) => Some(json!({
                    "type": "function_call_output",
                    "call_id": result.call_id.as_str(),
                    "output": function_result_output(&result.content),
                })),
                _ => None,
            })
            .collect(),
    })
}

fn flush_assistant_text(items: &mut Vec<Value>, content: &mut Vec<Value>) {
    if !content.is_empty() {
        items.push(json!({
            "type": "message",
            "role": "assistant",
            "content": std::mem::take(content),
        }));
    }
}

fn function_result_output(parts: &[ContentPart]) -> Value {
    if !parts
        .iter()
        .any(|part| matches!(part, ContentPart::Image { .. }))
    {
        return Value::String(
            parts
                .iter()
                .filter_map(ContentPart::as_text)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    Value::Array(
        parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::Text { text } => Some(json!({"type": "input_text", "text": text})),
                ContentPart::Image { url, detail } => Some(json!({
                    "type": "input_image",
                    "image_url": url,
                    "detail": detail.as_deref().unwrap_or("auto"),
                })),
                _ => None,
            })
            .collect(),
    )
}

pub(super) fn response_tool_names(
    tools: &[agent_runtime_core::provider::ToolSchema],
) -> Result<BTreeMap<String, String>, ProviderError> {
    let mut names = BTreeMap::new();
    for tool in tools {
        let wire = response_wire_tool_name(&tool.name);
        if wire.is_empty() {
            return Err(tool_name_collision());
        }
        if names
            .insert(wire, tool.name.clone())
            .is_some_and(|canonical| canonical != tool.name)
        {
            return Err(tool_name_collision());
        }
    }
    Ok(names)
}

/// The same readable provider name the runtime's adapters send: a model has
/// to be able to repeat it, so it carries no hash.
pub(super) fn response_wire_tool_name(name: &str) -> String {
    agent_runtime_core::tool::provider_tool_name(name)
}

fn tool_name_collision() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::BadRequest,
        "ChatGPT tool names collide after wire normalization",
    )
}

#[cfg(test)]
pub(super) fn valid_response_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
}

pub(super) fn response_tool_choice(
    choice: &ToolChoice,
    tool_names: &BTreeMap<String, String>,
) -> Result<Value, ProviderError> {
    Ok(match choice {
        ToolChoice::Auto => json!("auto"),
        ToolChoice::None => json!("none"),
        ToolChoice::Required => json!("required"),
        ToolChoice::Named(name) => {
            let wire = tool_names
                .iter()
                .find_map(|(wire, canonical)| (canonical == name).then_some(wire))
                .ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::BadRequest,
                        "the named ChatGPT tool choice is not in the request",
                    )
                })?;
            json!({"type": "function", "name": wire})
        }
    })
}
