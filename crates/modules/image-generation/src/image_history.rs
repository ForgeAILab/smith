//! Recent images selected from the host's live canonical history.

use std::sync::Arc;

use agent_runtime_core::content::{ContentPart, Message};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::SessionId;
use smith_module::SessionHistory;
use smith_tools::{MAX_RECENT_IMAGE_DATA_URL_BYTES, RecentImageSource};

#[derive(Debug)]
pub(crate) struct ConversationImages(pub(crate) Arc<dyn SessionHistory>);

impl RecentImageSource for ConversationImages {
    fn recent_images(
        &self,
        session: &SessionId,
        count: usize,
    ) -> Result<Vec<String>, RuntimeError> {
        let mut result = Ok(Vec::new());
        self.0.with_history(session, &mut |history| {
            result = collect_recent_images(history, count);
        })?;
        let mut images = result?;
        if images.len() < count {
            return Err(RuntimeError::tool(format!(
                "requested {count} recent image(s), but this session has only {}",
                images.len()
            )));
        }
        images.reverse();
        Ok(images)
    }
}

fn collect_recent_images(history: &[Message], count: usize) -> Result<Vec<String>, RuntimeError> {
    let mut images = Vec::with_capacity(count);
    'messages: for message in history.iter().rev() {
        for part in message.content.iter().rev() {
            match part {
                ContentPart::Image { url, .. } => {
                    if url.starts_with("data:image/") {
                        if url.len() > MAX_RECENT_IMAGE_DATA_URL_BYTES {
                            return Err(RuntimeError::tool(
                                "recent conversation image exceeds the 20 MiB reference limit",
                            ));
                        }
                        images.push(url.clone());
                    }
                }
                ContentPart::ToolResult(result) => {
                    for part in result.content.iter().rev() {
                        if let ContentPart::Image { url, .. } = part
                            && url.starts_with("data:image/")
                        {
                            if url.len() > MAX_RECENT_IMAGE_DATA_URL_BYTES {
                                return Err(RuntimeError::tool(
                                    "recent conversation image exceeds the 20 MiB reference limit",
                                ));
                            }
                            images.push(url.clone());
                            if images.len() == count {
                                break 'messages;
                            }
                        }
                    }
                }
                ContentPart::Text { .. }
                | ContentPart::Reasoning { .. }
                | ContentPart::ToolCall(_) => {}
            }
            if images.len() == count {
                break 'messages;
            }
        }
    }
    images.reverse();
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::content::{Role, ToolResultBlock};
    use agent_runtime_core::ids::ToolCallId;

    #[test]
    fn scans_newest_direct_and_tool_result_images_in_chronological_order() {
        let history = vec![
            Message::user("first"),
            Message::assistant(vec![ContentPart::Image {
                url: "data:image/png;base64,one".into(),
                detail: None,
            }]),
            Message::tool_result(ToolResultBlock {
                call_id: ToolCallId::new("call"),
                name: "generate_image".into(),
                content: vec![ContentPart::Image {
                    url: "data:image/png;base64,two".into(),
                    detail: None,
                }],
                is_error: false,
            }),
            Message::text(Role::User, "last"),
        ];
        assert_eq!(
            collect_recent_images(&history, 2).unwrap(),
            [
                "data:image/png;base64,one".to_owned(),
                "data:image/png;base64,two".to_owned()
            ]
        );
    }
}
