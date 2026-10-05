use super::*;

pub(super) fn response_items(
    message: &Message,
    tool_names: &BTreeMap<String, String>,
) -> Result<Vec<Value>, ProviderError> {
    Ok(match message.role {
        Role::System => Vec::new(),
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
            let content = message
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } => {
                        Some(json!({"type": "output_text", "text": text, "annotations": []}))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if !content.is_empty() {
                items.push(json!({"type": "message", "role": "assistant", "content": content}));
            }
            for part in &message.content {
                match part {
                    ContentPart::Reasoning {
                        signature: Some(signature),
                        ..
                    } => items.push(json!({
                        "type": "reasoning",
                        "summary": [],
                        "encrypted_content": signature,
                    })),
                    ContentPart::ToolCall(call) => {
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
            items
        }
        Role::Tool => message
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ToolResult(result) => Some(json!({
                    "type": "function_call_output",
                    "call_id": result.call_id.as_str(),
                    "output": result.content.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n"),
                })),
                _ => None,
            })
            .collect(),
    })
}

pub(super) fn response_tool_names(
    tools: &[agent_runtime_core::provider::ToolSchema],
) -> Result<BTreeMap<String, String>, ProviderError> {
    let mut names = BTreeMap::new();
    for tool in tools {
        let wire = response_wire_tool_name(&tool.name);
        if names
            .insert(wire, tool.name.clone())
            .is_some_and(|canonical| canonical != tool.name)
        {
            return Err(tool_name_collision());
        }
    }
    Ok(names)
}

pub(super) fn response_wire_tool_name(name: &str) -> String {
    if valid_response_tool_name(name) {
        return name.to_owned();
    }
    let digest = format!("{:x}", Sha256::digest(name.as_bytes()));
    format!("smith_tool_{}", &digest[..53])
}

fn tool_name_collision() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::BadRequest,
        "ChatGPT tool names collide after wire normalization",
    )
}

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
