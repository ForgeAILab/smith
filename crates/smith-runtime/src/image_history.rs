//! Active-session access to canonical conversation images.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use agent_runtime::runtime::SessionHandle;
use agent_runtime_core::content::{ContentPart, Message};
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::ids::SessionId;
use smith_tools::{MAX_RECENT_IMAGE_DATA_URL_BYTES, RecentImageSource};

/// Maps active root sessions to their canonical history without copying image
/// data into a second cache. Resumed sessions register their restored handle.
#[derive(Debug, Default)]
pub struct SessionImageHistory {
    sessions: Mutex<HashMap<String, SessionHandle>>,
}

impl SessionImageHistory {
    /// Registers one live or resumed session until the returned lease drops or
    /// is explicitly unregistered during host shutdown.
    pub fn register(self: &Arc<Self>, session: SessionHandle) -> SessionImageRegistration {
        let key = session.id().as_str().to_owned();
        self.sessions
            .lock()
            .expect("image-history registry lock poisoned")
            .insert(key.clone(), session);
        SessionImageRegistration {
            registry: Arc::downgrade(self),
            key,
            active: AtomicBool::new(true),
        }
    }

    fn unregister(&self, key: &str) {
        self.sessions
            .lock()
            .expect("image-history registry lock poisoned")
            .remove(key);
    }
}

impl RecentImageSource for SessionImageHistory {
    fn recent_images(
        &self,
        session: &SessionId,
        count: usize,
    ) -> Result<Vec<String>, RuntimeError> {
        let handle = self
            .sessions
            .lock()
            .expect("image-history registry lock poisoned")
            .get(session.as_str())
            .cloned()
            .ok_or_else(|| RuntimeError::not_found("this session has no active image history"))?;
        let mut images = handle.with_history(|history| collect_recent_images(history, count))?;
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

/// Removes one session from recent-image lookup on shutdown or drop.
#[derive(Debug)]
pub struct SessionImageRegistration {
    registry: Weak<SessionImageHistory>,
    key: String,
    active: AtomicBool,
}

impl SessionImageRegistration {
    /// Removes the registered history source. Safe to call more than once.
    pub fn unregister(&self) {
        if self.active.swap(false, Ordering::AcqRel)
            && let Some(registry) = self.registry.upgrade()
        {
            registry.unregister(&self.key);
        }
    }
}

impl Drop for SessionImageRegistration {
    fn drop(&mut self) {
        self.unregister();
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
