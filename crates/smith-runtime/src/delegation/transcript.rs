use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use agent_runtime::delegation::{ChildHistorySnapshot, DelegationCoordinator};
use agent_runtime_core::artifact::{
    ArtifactId, ArtifactProvenance, ArtifactRead, ArtifactRef, ArtifactRetention,
    ArtifactSensitivity, ArtifactStore, ArtifactWrite, MAX_ARTIFACT_READ_BYTES,
};
use agent_runtime_core::content::{ContentPart, Message, Role};
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::event::RuntimeEvent;
use agent_runtime_core::ids::{ChildId, ToolCallId};
use agent_runtime_core::tool::{InvocationContext, ToolOutcome};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::journal::DefaultRedactor;
use crate::tool_output::ToolOutputContextPolicy;

const MAX_SNAPSHOTS: usize = 16;
const DEFAULT_PAGE_SIZE: usize = 20;
const MAX_PAGE_SIZE: usize = 100;

/// One visible canonical content part. Call identities pair invocations and results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildTranscriptEntry {
    /// Message/part position within the pinned canonical history.
    pub id: String,
    /// Conversation author: user, assistant, or tool.
    pub role: String,
    /// Visible content kind, including reasoning and tool invocation/result.
    pub kind: String,
    /// Complete text, or an explicit notice when an artifact carries the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Credential-redacted call identity shared by the invocation and its result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Credential-redacted tool name for invocation/result entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Credential-redacted canonical invocation arguments when inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    /// Whether the canonical tool result reported an error.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    /// Requesting-parent-owned reference to the complete redacted entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<ArtifactRef>,
}

/// A bounded diagnostic read. Cursors pin one immutable historical snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildTranscriptPage {
    /// Direct child whose history was authorized for this read.
    pub child_id: String,
    /// Runtime session owning the canonical conversation.
    pub child_session_id: String,
    /// Observed lifecycle state at read time, independent of snapshot age.
    pub state: String,
    /// First live event sequence beyond the pinned history boundary.
    pub watermark: u64,
    /// Ordered visible entries that fit this page's context budget.
    pub entries: Vec<ChildTranscriptEntry>,
    /// Opaque continuation for this snapshot; absent after its final entry.
    pub next_cursor: Option<String>,
    /// Explicit retained-history or artifact limitations affecting this page.
    pub unavailable: Vec<String>,
}

/// Applies the ordinary display policy without mutating canonical child history.
/// Internal instruction messages and reasoning continuation secrets never leave it.
pub fn project_child_transcript_history(
    history: &[Message],
    redactor: &DefaultRedactor,
) -> Vec<Message> {
    history
        .iter()
        .filter(|message| message.role != Role::System)
        .map(|message| Message {
            role: message.role,
            content: visible_parts(&message.content, redactor),
        })
        .filter(|message| !message.content.is_empty())
        .collect()
}

fn visible_parts(parts: &[ContentPart], redactor: &DefaultRedactor) -> Vec<ContentPart> {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(ContentPart::text(redact_text(text, redactor))),
            ContentPart::Image { url, detail } => Some(ContentPart::Image {
                url: redact_text(url, redactor),
                detail: detail
                    .as_deref()
                    .map(|detail| redact_text(detail, redactor)),
            }),
            ContentPart::ToolCall(call) => {
                let mut call = call.clone();
                call.id = ToolCallId::new(redact_text(call.id.as_str(), redactor));
                call.name = redact_text(&call.name, redactor);
                call.arguments = redactor.redacted_clone(&call.arguments);
                Some(ContentPart::ToolCall(call))
            }
            ContentPart::Reasoning { redacted: true, .. } => None,
            ContentPart::Reasoning { text, .. } => Some(ContentPart::Reasoning {
                text: redact_text(text, redactor),
                redacted: false,
                signature: None,
                producer: None,
            }),
            ContentPart::ToolResult(result) => {
                let mut result = result.clone();
                result.call_id = ToolCallId::new(redact_text(result.call_id.as_str(), redactor));
                result.name = redact_text(&result.name, redactor);
                result.content = visible_parts(&result.content, redactor);
                Some(ContentPart::ToolResult(result))
            }
        })
        .collect()
}

fn redact_text(text: &str, redactor: &DefaultRedactor) -> String {
    match redactor.redacted_clone(&Value::String(text.to_owned())) {
        Value::String(text) => text,
        _ => String::new(),
    }
}

fn entries(
    snapshot: &ChildHistorySnapshot,
    redactor: &DefaultRedactor,
    offset: usize,
    limit: usize,
) -> (Vec<ChildTranscriptEntry>, usize) {
    let mut entries = Vec::new();
    let mut total = 0;
    let included = |index| (offset..offset.saturating_add(limit)).contains(&index);
    let mut observations = snapshot.retained_events.iter().collect::<Vec<_>>();
    observations.sort_by_key(|observation| (observation.history_index, observation.event.seq));
    let mut observations = observations.into_iter().peekable();
    for message_index in 0..=snapshot.history.len() {
        while observations
            .peek()
            .is_some_and(|observation| observation.history_index <= message_index)
        {
            let observation = observations.next().expect("peeked observation");
            if is_retained_observation(&observation.event.payload) {
                if included(total)
                    && let Some(entry) = observed_entry(&observation.event, redactor)
                {
                    entries.push(entry);
                }
                total += 1;
            }
        }
        let Some(message) = snapshot.history.get(message_index) else {
            break;
        };
        if message.role == Role::System {
            continue;
        }
        let role = match message.role {
            Role::System => unreachable!(),
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        for (part_index, part) in message.content.iter().enumerate() {
            if matches!(part, ContentPart::Reasoning { redacted: true, .. }) {
                continue;
            }
            let index = total;
            total += 1;
            if !included(index) {
                continue;
            }
            let Some(part) = visible_parts(std::slice::from_ref(part), redactor)
                .into_iter()
                .next()
            else {
                continue;
            };
            let mut entry = ChildTranscriptEntry {
                id: format!("m{message_index}:p{part_index}"),
                role: role.into(),
                kind: String::new(),
                text: None,
                call_id: None,
                tool_name: None,
                arguments: None,
                is_error: false,
                artifact: None,
            };
            match part {
                ContentPart::Text { text } => {
                    entry.kind = "text".into();
                    entry.text = Some(text);
                }
                ContentPart::Reasoning { text, .. } => {
                    entry.kind = "reasoning".into();
                    entry.text = Some(text);
                }
                ContentPart::Image { url, .. } => {
                    entry.kind = "image".into();
                    entry.text = Some(url);
                }
                ContentPart::ToolCall(call) => {
                    entry.kind = "tool_call".into();
                    entry.call_id = Some(call.id.to_string());
                    entry.tool_name = Some(call.name);
                    entry.arguments = Some(call.arguments);
                }
                ContentPart::ToolResult(result) => {
                    entry.kind = "tool_result".into();
                    entry.call_id = Some(result.call_id.to_string());
                    entry.tool_name = Some(result.name);
                    entry.is_error = result.is_error;
                    entry.text = Some(
                        if result
                            .content
                            .iter()
                            .all(|part| matches!(part, ContentPart::Text { .. }))
                        {
                            result
                                .content
                                .iter()
                                .filter_map(ContentPart::as_text)
                                .collect::<Vec<_>>()
                                .join("\n")
                        } else {
                            serde_json::to_string(&result.content).expect("content parts serialize")
                        },
                    );
                }
            }
            entries.push(entry);
        }
    }
    for observation in observations {
        if is_retained_observation(&observation.event.payload) {
            if included(total)
                && let Some(entry) = observed_entry(&observation.event, redactor)
            {
                entries.push(entry);
            }
            total += 1;
        }
    }
    (entries, total)
}

fn is_retained_observation(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::ExternalToolInvoked { .. }
            | RuntimeEvent::ExternalToolCompleted { .. }
            | RuntimeEvent::HostedSearch { .. }
    )
}

fn observed_entry(
    event: &agent_runtime_core::event::EventEnvelope,
    redactor: &DefaultRedactor,
) -> Option<ChildTranscriptEntry> {
    let mut entry = ChildTranscriptEntry {
        id: format!("e{}", event.seq),
        role: "assistant".into(),
        kind: String::new(),
        text: None,
        call_id: None,
        tool_name: None,
        arguments: None,
        is_error: false,
        artifact: None,
    };
    match &event.payload {
        RuntimeEvent::ExternalToolInvoked { id, name, detail } => {
            entry.kind = "external_tool_call".into();
            entry.call_id = Some(redact_text(id, redactor));
            entry.tool_name = Some(redact_text(name, redactor));
            entry.arguments = Some(redactor.redacted_clone(detail));
        }
        RuntimeEvent::ExternalToolCompleted { id, ok, detail } => {
            entry.kind = "external_tool_result".into();
            entry.role = "tool".into();
            entry.call_id = Some(redact_text(id, redactor));
            entry.is_error = !ok;
            entry.text = Some(redactor.redacted_clone(detail).to_string());
        }
        RuntimeEvent::HostedSearch {
            action,
            query,
            sources,
        } => {
            entry.kind = "hosted_search".into();
            entry.arguments = Some(redactor.redacted_clone(&serde_json::json!({
                "action": action,
                "query": query,
                "sources": sources,
            })));
        }
        _ => return None,
    }
    Some(entry)
}

/// Read-only projection and parent-owned artifact access for child conversations.
/// The small snapshot cache bounds retained reads; expired cursors fail explicitly.
#[derive(Debug)]
pub struct ChildTranscriptReader {
    store: Option<Arc<dyn ArtifactStore>>,
    redactor: DefaultRedactor,
    policy: ToolOutputContextPolicy,
    snapshots: Mutex<VecDeque<(String, Arc<ChildHistorySnapshot>)>>,
    next_snapshot: AtomicU64,
}

impl ChildTranscriptReader {
    /// Shares the root's artifact storage, credential registry, and output budget.
    pub fn new(
        store: Option<Arc<dyn ArtifactStore>>,
        redactor: DefaultRedactor,
        policy: ToolOutputContextPolicy,
    ) -> Self {
        Self {
            store,
            redactor,
            policy,
            snapshots: Mutex::new(VecDeque::new()),
            next_snapshot: AtomicU64::new(1),
        }
    }

    /// Reads an authorized immutable history page without executing child work
    /// or acknowledging any protected completion outcome.
    pub async fn page(
        &self,
        coordinator: &DelegationCoordinator,
        child: &ChildId,
        cursor: Option<&str>,
        page_size: Option<usize>,
        ctx: &InvocationContext,
    ) -> Result<ChildTranscriptPage, RuntimeError> {
        // Resolve authority before touching cached content or protected storage.
        let status = coordinator.status(child)?;
        if status.parent != ctx.session {
            return Err(RuntimeError::new(
                ErrorKind::Approval,
                "only the owning root session may read a child transcript",
            ));
        }
        let page_size = page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
            return Err(RuntimeError::tool("transcript page_size must be 1..=100"));
        }
        let (token, offset, snapshot) = if let Some(cursor) = cursor {
            let (token, offset) = cursor
                .rsplit_once(':')
                .and_then(|(token, offset)| offset.parse::<usize>().ok().map(|n| (token, n)))
                .ok_or_else(|| RuntimeError::tool("invalid transcript cursor"))?;
            let snapshot = self
                .snapshots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .find(|(candidate, _)| candidate == token)
                .map(|(_, snapshot)| snapshot.clone())
                .ok_or_else(|| RuntimeError::tool("transcript cursor expired; start a new read"))?;
            if snapshot.status.child != *child || snapshot.status.parent != ctx.session {
                return Err(RuntimeError::tool(
                    "transcript cursor belongs to another child",
                ));
            }
            (token.to_owned(), offset, snapshot)
        } else {
            let snapshot = Arc::new(coordinator.child_history(child).await?);
            let ordinal = self.next_snapshot.fetch_add(1, Ordering::Relaxed);
            let token = format!(
                "{:x}",
                Sha256::digest(format!(
                    "{}:{}:{ordinal}:{}",
                    ctx.session, child, snapshot.next_event_sequence
                ))
            );
            let mut cache = self
                .snapshots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.push_back((token.clone(), snapshot.clone()));
            while cache.len() > MAX_SNAPSHOTS {
                cache.pop_front();
            }
            (token, 0, snapshot)
        };
        let (selected_entries, total_entries) =
            entries(&snapshot, &self.redactor, offset, page_size);
        if offset > total_entries {
            return Err(RuntimeError::tool(
                "transcript cursor offset is out of range",
            ));
        }
        let mut page = ChildTranscriptPage {
            child_id: child.to_string(),
            child_session_id: snapshot.status.session.to_string(),
            state: super::tool::status_json(&status)["state"]
                .as_str()
                .unwrap_or("unknown")
                .to_owned(),
            watermark: snapshot.next_event_sequence,
            entries: Vec::new(),
            next_cursor: None,
            unavailable: snapshot
                .unavailable_reason
                .iter()
                .map(|reason| {
                    redact_text(reason, &self.redactor)
                        .chars()
                        .take(512)
                        .collect()
                })
                .collect(),
        };
        let budget = (self.policy.inline_bytes as usize)
            .min(ctx.output_limit)
            .saturating_sub(64);
        for (next, mut entry) in (offset..).zip(selected_entries) {
            self.expand_archived_result(&mut entry, &snapshot, &mut page.unavailable)
                .await;
            if serde_json::to_vec(&entry).map_err(serialization)?.len() > budget / 2 {
                self.offload_entry(&mut entry, &snapshot, ctx).await?;
            }
            page.entries.push(entry);
            page.next_cursor = (next + 1 < total_entries).then(|| format!("{token}:{}", next + 1));
            if serde_json::to_vec(&page).map_err(serialization)?.len() > budget {
                page.entries.pop();
                if page.entries.is_empty() {
                    return Err(RuntimeError::tool(
                        "transcript output budget is too small for a readable entry",
                    ));
                }
                page.next_cursor = Some(format!("{token}:{next}"));
                break;
            }
        }
        if serde_json::to_vec(&page).map_err(serialization)?.len() > budget {
            return Err(RuntimeError::tool(
                "transcript output budget is too small for history metadata",
            ));
        }
        Ok(page)
    }

    async fn offload_entry(
        &self,
        entry: &mut ChildTranscriptEntry,
        snapshot: &ChildHistorySnapshot,
        ctx: &InvocationContext,
    ) -> Result<(), RuntimeError> {
        let store = self.store.as_ref().ok_or_else(|| {
            RuntimeError::tool("complete transcript entry requires unavailable artifact storage")
        })?;
        let bytes = serde_json::to_vec(entry).map_err(serialization)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let provenance = ArtifactProvenance::new(ctx.session.clone(), "child-transcript");
        let reference = store
            .put(ArtifactWrite {
                bytes,
                media_type: "application/json".into(),
                sensitivity: ArtifactSensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                provenance,
                idempotency_key: format!(
                    "child-transcript:{}:{}:{digest}",
                    snapshot.status.session, entry.id
                ),
            })
            .await
            .map_err(|error| {
                RuntimeError::tool(format!("transcript artifact unavailable: {error}"))
            })?;
        entry.text = Some("Complete redacted entry: read artifact.id with artifact.read.".into());
        entry.arguments = None;
        entry.artifact = Some(reference);
        Ok(())
    }

    async fn expand_archived_result(
        &self,
        entry: &mut ChildTranscriptEntry,
        snapshot: &ChildHistorySnapshot,
        unavailable: &mut Vec<String>,
    ) {
        if entry.kind != "tool_result" {
            return;
        }
        let Some(id) = entry
            .text
            .as_deref()
            .and_then(|text| text.strip_prefix("[artifact id="))
            .and_then(|text| text.split_once(' '))
            .and_then(|(id, _)| ArtifactId::new(id).ok())
        else {
            return;
        };
        match self.read_archived_result(&id, snapshot).await {
            Ok(text) => entry.text = Some(text),
            Err(error) => {
                // A child's raw reference grants the parent no access. Never
                // present an inaccessible source id as a successful retrieval.
                let preview = entry
                    .text
                    .as_deref()
                    .and_then(|text| text.split_once('\n'))
                    .map(|(_, preview)| preview)
                    .unwrap_or_default();
                entry.text = Some(format!("Archived tool output is unavailable.\n{preview}"));
                unavailable.push(format!(
                    "{}: {}",
                    entry.id,
                    redact_text(&error.message, &self.redactor)
                ));
            }
        }
    }

    async fn read_archived_result(
        &self,
        id: &ArtifactId,
        snapshot: &ChildHistorySnapshot,
    ) -> Result<String, RuntimeError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| RuntimeError::tool("child artifact storage is unavailable"))?;
        let mut bytes = Vec::new();
        let mut offset = 0;
        let mut reference = None;
        loop {
            let request = ArtifactRead {
                session: snapshot.status.session.clone(),
                id: id.clone(),
                offset,
                limit: MAX_ARTIFACT_READ_BYTES,
            };
            let chunk = store.read(request.clone()).await.map_err(|error| {
                RuntimeError::tool(format!("child artifact unavailable: {error}"))
            })?;
            chunk
                .validate_for(&request)
                .map_err(|error| RuntimeError::tool(error.to_string()))?;
            if chunk.reference.provenance.session != snapshot.status.session
                || reference
                    .as_ref()
                    .is_some_and(|reference| reference != &chunk.reference)
            {
                return Err(RuntimeError::tool(
                    "child artifact ownership or content changed",
                ));
            }
            reference = Some(chunk.reference.clone());
            if chunk.reference.sensitivity == ArtifactSensitivity::Secret {
                return Err(RuntimeError::tool("child artifact is protected"));
            }
            if bytes.len().saturating_add(chunk.bytes.len())
                > crate::artifact::MAX_SMITH_ARTIFACT_BYTES
            {
                return Err(RuntimeError::tool(
                    "child artifact exceeds the storage bound",
                ));
            }
            bytes.extend_from_slice(&chunk.bytes);
            match chunk.next_offset {
                Some(next) if next > offset => offset = next,
                Some(_) => return Err(RuntimeError::tool("child artifact cursor did not advance")),
                None => break,
            }
        }
        let outcome: ToolOutcome = serde_json::from_slice(&bytes).map_err(serialization)?;
        if let Some(parts) = outcome.content.as_inline() {
            let safe = visible_parts(parts, &self.redactor);
            return serde_json::to_string(&serde_json::json!({
                "value": self.redactor.redacted_clone(&outcome.value),
                "content": safe,
                "is_error": outcome.is_error,
            }))
            .map_err(serialization);
        }
        Err(RuntimeError::tool(
            "nested archived tool output is unavailable",
        ))
    }
}

fn serialization(error: serde_json::Error) -> RuntimeError {
    RuntimeError::tool(format!(
        "transcript content could not be serialized: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::event::EventEnvelope;
    use agent_runtime_core::ids::{EventId, SessionId};

    fn event(seq: u64, payload: RuntimeEvent) -> EventEnvelope {
        EventEnvelope::new(
            seq,
            EventId::new(format!("e{seq}")),
            SessionId::new("child-session"),
            None,
            Timestamp::ZERO,
            payload,
        )
    }

    #[test]
    fn native_tool_observations_keep_their_reported_origin_and_redact_details() {
        let redactor = DefaultRedactor::new().with_secret("TOP_SECRET");
        let call = observed_entry(
            &event(2, RuntimeEvent::ExternalToolInvoked {
                id:"native-call".into(),
                name:"Read".into(),
                detail:serde_json::json!({"path":"TOP_SECRET/file", "api_key":"STRUCTURAL_SECRET"}),
            }),
            &redactor,
        ).unwrap();
        let result = observed_entry(
            &event(
                3,
                RuntimeEvent::ExternalToolCompleted {
                    id: "native-call".into(),
                    ok: false,
                    detail: serde_json::json!({"output":"TOP_SECRET evidence"}),
                },
            ),
            &redactor,
        )
        .unwrap();
        assert_eq!(call.kind, "external_tool_call");
        assert_eq!(result.kind, "external_tool_result");
        assert_eq!(call.call_id, result.call_id);
        assert!(result.is_error);
        assert_eq!(call.arguments.as_ref().unwrap()["api_key"], "[redacted]");
        assert!(
            result
                .text
                .as_deref()
                .unwrap()
                .contains("[redacted] evidence")
        );
        let wire = serde_json::to_string(&[call, result]).unwrap();
        assert!(!wire.contains("TOP_SECRET") && !wire.contains("STRUCTURAL_SECRET"));
    }

    #[test]
    fn hosted_search_keeps_observed_metadata_and_ignores_unrelated_events() {
        let redactor = DefaultRedactor::new().with_secret("TOP_SECRET");
        let entry = observed_entry(
            &event(
                4,
                RuntimeEvent::HostedSearch {
                    action: None,
                    query: Some("TOP_SECRET query".into()),
                    sources: Vec::new(),
                },
            ),
            &redactor,
        )
        .unwrap();
        assert_eq!(entry.kind, "hosted_search");
        assert_eq!(entry.id, "e4");
        assert_eq!(entry.arguments.unwrap()["query"], "[redacted] query");
        assert!(observed_entry(&event(5, RuntimeEvent::TurnStarted), &redactor).is_none());
    }
}
