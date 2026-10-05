use super::*;

pub(super) fn push_utf8(
    pending: &mut Vec<u8>,
    chunk: &[u8],
) -> Result<Option<String>, ProviderError> {
    pending.extend_from_slice(chunk);
    match std::str::from_utf8(pending) {
        Ok(text) => {
            let text = text.to_owned();
            pending.clear();
            Ok((!text.is_empty()).then_some(text))
        }
        Err(error) if error.error_len().is_none() => {
            let valid = error.valid_up_to();
            if valid == 0 {
                return Ok(None);
            }
            let text = std::str::from_utf8(&pending[..valid])
                .expect("validated UTF-8 prefix")
                .to_owned();
            pending.drain(..valid);
            Ok(Some(text))
        }
        Err(_) => Err(ProviderError::new(
            ProviderErrorKind::MalformedStream,
            "ChatGPT Responses stream contained invalid UTF-8",
        )),
    }
}

#[derive(Default)]
pub(super) struct StreamState {
    pub(super) saw_semantic: bool,
    pub(super) saw_tool_call: bool,
    pub(super) terminal: bool,
    pub(super) tool_names: BTreeMap<String, String>,
}

pub(super) fn decode_event(
    data: &str,
    state: &mut StreamState,
) -> Result<Vec<ProviderStreamEvent>, ProviderError> {
    let value: Value = serde_json::from_str(data).map_err(|_| {
        ProviderError::new(
            ProviderErrorKind::MalformedStream,
            "invalid ChatGPT Responses stream event",
        )
    })?;
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut events = Vec::new();
    match kind {
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str)
                && !delta.is_empty()
            {
                events.push(ProviderStreamEvent::TextDelta {
                    text: delta.to_owned(),
                });
                state.saw_semantic = true;
            }
        }
        "response.reasoning_summary_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str)
                && !delta.is_empty()
            {
                events.push(ProviderStreamEvent::ReasoningDelta {
                    text: delta.to_owned(),
                    redacted: false,
                    signature: None,
                });
                state.saw_semantic = true;
            }
        }
        "response.output_item.added" => {
            let item = value.get("item").unwrap_or(&Value::Null);
            if item.get("type").and_then(Value::as_str) == Some("function_call") {
                let index = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .and_then(|index| u32::try_from(index).ok())
                    .unwrap_or(0);
                events.push(ProviderStreamEvent::ToolCallDelta {
                    index,
                    id: item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    name: item.get("name").and_then(Value::as_str).map(|wire| {
                        state
                            .tool_names
                            .get(wire)
                            .cloned()
                            .unwrap_or_else(|| wire.to_owned())
                    }),
                    arguments_fragment: String::new(),
                });
                state.saw_tool_call = true;
                state.saw_semantic = true;
            }
        }
        "response.function_call_arguments.delta" => {
            let index = value
                .get("output_index")
                .and_then(Value::as_u64)
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or(0);
            events.push(ProviderStreamEvent::ToolCallDelta {
                index,
                id: None,
                name: None,
                arguments_fragment: value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
            state.saw_tool_call = true;
            state.saw_semantic = true;
        }
        "response.output_item.done" => {
            let item = value.get("item").unwrap_or(&Value::Null);
            if item.get("type").and_then(Value::as_str) == Some("reasoning")
                && let Some(signature) = item.get("encrypted_content").and_then(Value::as_str)
            {
                events.push(ProviderStreamEvent::ReasoningDelta {
                    // Agent Runtime intentionally refuses a signature with no
                    // block to seal. A fixed redacted marker preserves the
                    // opaque continuation token without exposing it.
                    text: "[encrypted]".to_owned(),
                    redacted: true,
                    signature: Some(signature.to_owned()),
                });
                state.saw_semantic = true;
            }
        }
        "response.completed" => {
            if let Some(usage) = value.pointer("/response/usage") {
                append_usage(usage, &mut events);
            }
            events.push(ProviderStreamEvent::Finish {
                reason: if state.saw_tool_call {
                    FinishReason::ToolCalls
                } else {
                    FinishReason::Stop
                },
            });
            state.terminal = true;
        }
        "response.incomplete" => {
            if let Some(usage) = value.pointer("/response/usage") {
                append_usage(usage, &mut events);
            }
            let reason = if value
                .pointer("/response/incomplete_details/reason")
                .and_then(Value::as_str)
                == Some("max_output_tokens")
            {
                FinishReason::Length
            } else {
                FinishReason::Error
            };
            events.push(ProviderStreamEvent::Finish { reason });
            state.terminal = true;
        }
        "response.failed" | "error" => {
            state.terminal = true;
            events.push(ProviderStreamEvent::Error {
                error: ProviderError::new(
                    ProviderErrorKind::Server,
                    "the experimental ChatGPT Responses backend reported a failure",
                )
                .retryable(),
            });
        }
        _ => {}
    }
    Ok(events)
}

fn append_usage(usage: &Value, events: &mut Vec<ProviderStreamEvent>) {
    let input = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let details = usage.get("input_tokens_details");
    let cached = usage
        .get("input_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .and_then(Value::as_u64);
    let written = details
        .and_then(|details| details.get("cache_write_tokens"))
        .and_then(Value::as_u64);
    // Keep the billing counters disjoint while preserving the provider's
    // independent field presence in CacheObservation. An explicit zero is
    // evidence; an omitted field is not a zero.
    let cached_count = cached.unwrap_or_default().min(input);
    let write_count = written
        .unwrap_or_default()
        .min(input.saturating_sub(cached_count));
    let output = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let reasoning = usage
        .get("output_tokens_details")
        .and_then(|details| details.get("reasoning_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .min(output);
    let mut delta = UsageDelta::new();
    let uncached = input
        .saturating_sub(cached_count)
        .saturating_sub(write_count);
    if uncached > 0 {
        delta.add(CounterKind::InputUncached, uncached);
    }
    if cached_count > 0 {
        delta.add(CounterKind::InputCached, cached_count);
    }
    if write_count > 0 {
        delta.add(CounterKind::CacheWrite, write_count);
    }
    if cached.is_some() || written.is_some() {
        events.push(ProviderStreamEvent::CacheObservation {
            read_tokens: cached,
            write_tokens: written,
        });
    }
    if output.saturating_sub(reasoning) > 0 {
        delta.add(CounterKind::Output, output.saturating_sub(reasoning));
    }
    if reasoning > 0 {
        delta.add(CounterKind::Reasoning, reasoning);
    }
    if !delta.is_empty() {
        events.push(ProviderStreamEvent::Usage { delta });
    }
}
