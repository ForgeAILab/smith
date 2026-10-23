use super::*;
use futures_util::StreamExt;

/// Display-safe child history with its exclusive live-event boundary.
pub struct ChildTranscriptSnapshot {
    /// Coordinator-owned identity and lifecycle state.
    pub status: agent_runtime::delegation::ChildStatus,
    /// Committed conversation under the main transcript's visibility rules.
    pub history: Vec<Message>,
    /// Live events before this sequence are already represented in history.
    pub next_event_sequence: u64,
    /// An explicit limitation when retained history cannot be recovered.
    pub unavailable_reason: Option<String>,
    /// Provider-visible output still awaiting the attempt's commit decision.
    pub pending_events: Vec<crate::client::SmithEvent>,
    /// Retained native-tool/search observations at safe message boundaries.
    pub observations: Vec<(usize, crate::client::SmithEvent)>,
    /// Reviewed invocation displays resolved from the same history snapshot.
    pub tool_displays: Vec<(ToolCallId, ToolCallDisplay)>,
    /// Display-safe tool result text from the same history snapshot.
    pub tool_results: Vec<(ToolCallId, String)>,
}

/// A history snapshot captured after subscribing to the child's live stream.
pub struct ChildTranscriptSubscription {
    /// Committed history at the subscription boundary.
    pub snapshot: ChildTranscriptSnapshot,
    /// Future child events; a retained dormant child has no live stream.
    pub events: Option<crate::client::SmithEventStream>,
}

impl HostSession {
    /// Reads an owned direct child's conversation without executing it.
    pub async fn child_transcript(
        &self,
        child: &ChildId,
    ) -> Result<ChildTranscriptSnapshot, RuntimeError> {
        let coordinator = self
            .runtime
            .delegation()
            .and_then(|delegation| delegation.coordinator())
            .ok_or_else(|| RuntimeError::config("delegation is not available for this session"))?;
        self.project_child_transcript(coordinator.child_history(child).await?)
    }

    /// Subscribes before reading history, preserving the canonical boundary.
    pub async fn subscribe_child_transcript(
        &self,
        child: &ChildId,
    ) -> Result<ChildTranscriptSubscription, RuntimeError> {
        let coordinator = self
            .runtime
            .delegation()
            .and_then(|delegation| delegation.coordinator())
            .ok_or_else(|| RuntimeError::config("delegation is not available for this session"))?;
        let subscription = coordinator.subscribe_child_history(child).await?;
        let redactor = self.display_redactor.clone();
        Ok(ChildTranscriptSubscription {
            snapshot: self.project_child_transcript(subscription.snapshot)?,
            events: subscription.events.map(|stream| {
                Box::pin(stream.map(move |event| {
                    project_child_display_event(&event, &redactor).unwrap_or_else(|_| {
                        crate::client::SmithEvent::new(
                            event.seq,
                            event.id,
                            event.session,
                            event.turn,
                            event.timestamp,
                            crate::client::SmithEventKind::Unknown,
                        )
                    })
                })) as crate::client::SmithEventStream
            }),
        })
    }

    fn project_child_transcript(
        &self,
        snapshot: agent_runtime::delegation::ChildHistorySnapshot,
    ) -> Result<ChildTranscriptSnapshot, RuntimeError> {
        let mut history = Vec::new();
        let mut safe_anchors = vec![0];
        for message in snapshot.history.iter() {
            history.extend(crate::delegation::project_child_transcript_history(
                std::slice::from_ref(message),
                &self.display_redactor,
            ));
            safe_anchors.push(history.len());
        }
        let tool_displays = tool_call_displays_from_history(&history, &self.display_redactor);
        let tool_results = history
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|part| match part {
                ContentPart::ToolResult(result) => {
                    redacted_result_text(result, &self.display_redactor)
                        .map(|text| (result.call_id.clone(), text))
                }
                _ => None,
            })
            .collect();
        let pending_events = snapshot
            .pending_events
            .iter()
            .map(|event| project_child_display_event(event, &self.display_redactor))
            .collect::<Result<Vec<_>, _>>()?;
        let observations = snapshot
            .retained_events
            .iter()
            .map(|observation| {
                let anchor = *safe_anchors.get(observation.history_index).ok_or_else(|| {
                    RuntimeError::conflict("child activity references unavailable history")
                })?;
                Ok((
                    anchor,
                    project_child_display_event(&observation.event, &self.display_redactor)?,
                ))
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let status = serde_json::to_value(&snapshot.status)
            .map(|value| self.display_redactor.redacted_clone(&value))
            .and_then(serde_json::from_value)
            .map_err(|_| {
                RuntimeError::new(
                    ErrorKind::Serialization,
                    "child status could not be projected",
                )
            })?;
        Ok(ChildTranscriptSnapshot {
            status,
            history,
            next_event_sequence: snapshot.next_event_sequence,
            unavailable_reason: snapshot.unavailable_reason,
            pending_events,
            observations,
            tool_displays,
            tool_results,
        })
    }
}

fn project_child_display_event(
    event: &EventEnvelope,
    redactor: &DefaultRedactor,
) -> Result<crate::client::SmithEvent, RuntimeError> {
    let value = serde_json::to_value(event).map_err(|_| {
        RuntimeError::new(
            ErrorKind::Serialization,
            "child event could not be projected",
        )
    })?;
    let mut event: EventEnvelope = serde_json::from_value(redactor.redacted_clone(&value))
        .map_err(|_| {
            RuntimeError::new(
                ErrorKind::Serialization,
                "child event could not be redacted",
            )
        })?;
    if let RuntimeEvent::ReasoningDelta {
        redacted: true,
        text,
        ..
    } = &mut event.payload
    {
        *text = "[protected reasoning]".into();
    }
    Ok(crate::client::SmithEvent::project_or_unknown(&event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::clock::Timestamp;
    use agent_runtime_core::ids::{AttemptId, EventId, RequestId};

    #[test]
    fn pending_child_output_uses_display_redaction_and_preserves_event_identity() {
        let redactor = DefaultRedactor::new().with_secret("sensitive-provider-key");
        let event = EventEnvelope::new(
            42,
            EventId::new("child-output"),
            SessionId::new("child-session"),
            Some(TurnId::new("turn-1")),
            Timestamp(123),
            RuntimeEvent::TextDelta {
                request: RequestId::new("request-1"),
                attempt: AttemptId::new("attempt-1"),
                text: "The response echoed sensitive-provider-key".into(),
            },
        );
        let projected = project_child_display_event(&event, &redactor).expect("safe event");
        assert_eq!(projected.seq, 42);
        assert_eq!(projected.id, event.id);
        assert!(
            !serde_json::to_string(&projected)
                .expect("serialized event")
                .contains("sensitive-provider-key")
        );
        assert!(
            matches!(projected.payload, crate::client::SmithEventKind::TextDelta { ref text, .. } if text.contains("[redacted]"))
        );
    }

    #[test]
    fn protected_pending_reasoning_does_not_expose_its_text() {
        let event = EventEnvelope::new(
            9,
            EventId::new("protected-child-output"),
            SessionId::new("child-session"),
            Some(TurnId::new("turn-1")),
            Timestamp(123),
            RuntimeEvent::ReasoningDelta {
                request: RequestId::new("request-1"),
                attempt: AttemptId::new("attempt-1"),
                text: "protected provider payload".into(),
                redacted: true,
            },
        );
        let projected =
            project_child_display_event(&event, &DefaultRedactor::new()).expect("safe event");
        assert!(
            !serde_json::to_string(&projected)
                .expect("serialized event")
                .contains("protected provider payload")
        );
        assert!(matches!(
            projected.payload,
            crate::client::SmithEventKind::ReasoningDelta { redacted: true, .. }
        ));
    }
}
