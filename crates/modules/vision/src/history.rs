//! Digest lookup in canonical history, including nested tool results.

use agent_runtime_core::content::{ContentPart, Message};
use sha2::{Digest, Sha256};

pub(crate) struct HistoryImage {
    pub(crate) url: String,
    pub(crate) source_tool: Option<String>,
}

pub(crate) fn find(history: &[Message], id: &str) -> Option<HistoryImage> {
    history
        .iter()
        .rev()
        .find_map(|message| find_parts(&message.content, id, None))
}

fn find_parts(parts: &[ContentPart], id: &str, source: Option<&str>) -> Option<HistoryImage> {
    parts.iter().rev().find_map(|part| match part {
        ContentPart::Image { url, .. }
            if format!("{:x}", Sha256::digest(url.as_bytes())).starts_with(id) =>
        {
            Some(HistoryImage {
                url: url.clone(),
                source_tool: source.map(str::to_owned),
            })
        }
        ContentPart::ToolResult(result) => find_parts(&result.content, id, Some(&result.name)),
        _ => None,
    })
}
