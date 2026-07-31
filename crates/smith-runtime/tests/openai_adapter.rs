//! The adapter contract for the shared OpenAI-compatible provider, offline.
//!
//! `tests/transport.rs` pins what Smith owns below the adapter: sockets,
//! statuses, deadlines, and redaction at the wire. This file pins the half
//! above it — what a recorded SSE body becomes once Agent Runtime's
//! `OpenAiProvider` has normalized it — by driving the real adapter over a
//! replay transport. No listener is bound, no byte leaves the process, and no
//! request is billable.
//!
//! Smith does not own this normalization; Agent Runtime does (design.md
//! Decision 2). These fixtures exist anyway because they are Smith's half of
//! the coordinated runtime compatibility gate (`runtime-integration/spec.md`):
//! a future runtime revision that changes event order, counter mapping, error
//! classification, or redaction must fail here before it is adopted. Two tests
//! are therefore marked UPSTREAM: they record behavior that disagrees with this
//! change's specs. Smith may not modify Agent Runtime under this approval, so
//! they pin what the adapter really does today and say so.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_runtime::agent::assembler::ToolCallAssembler;
use agent_runtime::ids::IdMinter;
use agent_runtime::provider::openai::{OpenAiConfig, OpenAiProvider};
use agent_runtime::provider::retry::{RetryPolicy, is_retryable};
use agent_runtime::provider::transport::{ByteStream, HttpRequest, HttpTransport};
use agent_runtime_core::cancel::{CancelReason, Cancellation};
use agent_runtime_core::clock::Deadline;
use agent_runtime_core::content::{Message, ToolCall};
use agent_runtime_core::ids::{AttemptId, RequestId};
use agent_runtime_core::provider::{
    FinishReason, ModelId, Provider, ProviderCallContext, ProviderError, ProviderErrorKind,
    ProviderRequest, ProviderStream, ProviderStreamEvent,
};
use agent_runtime_core::store::Secret;
use agent_runtime_core::usage::{CounterKind, UsageDelta};
use agent_runtime_testkit::ReplayTransport;
use futures_util::{StreamExt, stream};
use serde_json::json;

/// A key shaped like a real one, so a substring assertion means something.
const API_KEY: &str = "sk-live-4tVq8HmZ0pLcXw2RnB6yDe1F";

/// Prompt text that must never be echoed back out of an error surface.
const PROMPT: &str = "the private contents of a user prompt";

/// The longest any single step may take before it is a failure, not a hang.
const PATIENCE: Duration = Duration::from_secs(5);

// -- fixtures ----------------------------------------------------------------

/// One SSE frame: a `data:` line plus the blank line that terminates it.
fn frame(data: &str) -> String {
    format!("data: {data}\n\n")
}

/// The terminal frame every OpenAI-compatible stream ends with.
fn done() -> String {
    frame("[DONE]")
}

/// A config carrying a credential, because most of these fixtures also have to
/// show where that credential does *not* end up.
fn config() -> OpenAiConfig {
    OpenAiConfig {
        api_key: Some(Secret::new(API_KEY)),
        ..OpenAiConfig::new("http://provider.invalid/v1", "test-model")
    }
}

fn request() -> ProviderRequest {
    ProviderRequest::new(ModelId::new("test-model"), vec![Message::user(PROMPT)])
}

fn call_context(cancel: Cancellation) -> ProviderCallContext {
    ProviderCallContext {
        request_id: RequestId::new("req-1"),
        attempt_id: AttemptId::new("att-1"),
        cancel,
        deadline: Deadline::never(),
    }
}

/// An adapter that will be handed `chunks` as consecutive transport reads.
///
/// The testkit's replay transport covers every case where the bytes are all the
/// fixture needs to say; `ScriptedTransport` below covers the ones where it is
/// not.
fn replaying(chunks: Vec<String>) -> OpenAiProvider<ReplayTransport> {
    OpenAiProvider::new(ReplayTransport::new(chunks), config())
}

/// Runs one attempt over `chunks` and returns every normalized event.
async fn events_from(chunks: Vec<String>) -> Vec<ProviderStreamEvent> {
    let provider = replaying(chunks);
    let stream = within(
        "the request",
        provider.stream(request(), call_context(Cancellation::new())),
    )
    .await
    .expect("a normalized event stream");
    within("the stream", drain(stream)).await
}

// -- a scripted transport ----------------------------------------------------

/// What the transport does for one `post_stream` call.
///
/// The testkit's `ReplayTransport` replays one fixed byte sequence for every
/// request, always succeeds, and never stops yielding — so it cannot express a
/// first attempt that fails, a body that dies halfway, or a connection that
/// simply never speaks. It also owns its recorded requests, which become
/// unreachable once `OpenAiProvider` takes the transport by value. Those four
/// gaps are the whole reason this type exists.
#[derive(Debug)]
enum Reply {
    /// The request never becomes a stream.
    Refuse(ProviderError),
    /// These chunks are delivered and then the body ends cleanly.
    Body(Vec<String>),
    /// These chunks are delivered and then the body fails.
    Dies(Vec<String>, ProviderError),
    /// The request is accepted and not one byte ever arrives.
    Silent,
}

/// A transport that answers each request with the next scripted reply and
/// records what it was asked to send.
#[derive(Debug)]
struct ScriptedTransport {
    replies: Mutex<VecDeque<Reply>>,
    seen: Arc<Mutex<Vec<HttpRequest>>>,
}

#[async_trait::async_trait]
impl HttpTransport for ScriptedTransport {
    async fn post_stream(&self, request: HttpRequest) -> Result<ByteStream, ProviderError> {
        self.seen.lock().expect("the request log").push(request);
        let reply = self
            .replies
            .lock()
            .expect("the script")
            .pop_front()
            .expect("the adapter asked for more attempts than the script allows");
        match reply {
            Reply::Refuse(error) => Err(error),
            Reply::Body(chunks) => Ok(Box::pin(stream::iter(
                chunks.into_iter().map(|chunk| Ok(chunk.into_bytes())),
            ))),
            Reply::Dies(chunks, error) => {
                let items: Vec<Result<Vec<u8>, ProviderError>> = chunks
                    .into_iter()
                    .map(|chunk| Ok(chunk.into_bytes()))
                    .chain(std::iter::once(Err(error)))
                    .collect();
                Ok(Box::pin(stream::iter(items)))
            }
            Reply::Silent => Ok(Box::pin(stream::pending())),
        }
    }
}

/// An adapter over `replies`, plus the log of requests it actually sends.
fn scripted(
    replies: Vec<Reply>,
) -> (
    Arc<Mutex<Vec<HttpRequest>>>,
    OpenAiProvider<ScriptedTransport>,
) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let transport = ScriptedTransport {
        replies: Mutex::new(replies.into()),
        seen: Arc::clone(&seen),
    };
    (seen, OpenAiProvider::new(transport, config()))
}

fn sent(seen: &Arc<Mutex<Vec<HttpRequest>>>) -> Vec<HttpRequest> {
    seen.lock().expect("the request log").clone()
}

// -- reading the event stream ------------------------------------------------

/// Fails the test rather than hanging forever.
async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(PATIENCE, future)
        .await
        .unwrap_or_else(|_| panic!("{what} did not finish within {PATIENCE:?}"))
}

async fn drain(mut stream: ProviderStream) -> Vec<ProviderStreamEvent> {
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

/// Every text fragment, in arrival order.
fn text_deltas(events: &[ProviderStreamEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            ProviderStreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn finish(events: &[ProviderStreamEvent]) -> Option<FinishReason> {
    events.iter().find_map(|event| match event {
        ProviderStreamEvent::Finish { reason } => Some(*reason),
        _ => None,
    })
}

fn usage(events: &[ProviderStreamEvent]) -> Vec<UsageDelta> {
    events
        .iter()
        .filter_map(|event| match event {
            ProviderStreamEvent::Usage { delta } => Some(delta.clone()),
            _ => None,
        })
        .collect()
}

fn error(events: &[ProviderStreamEvent]) -> Option<&ProviderError> {
    events.iter().find_map(|event| match event {
        ProviderStreamEvent::Error { error } => Some(error),
        _ => None,
    })
}

/// Feeds the streamed fragments through the shared assembler, which is the only
/// thing that turns `ToolCallDelta`s into calls. Reassembly is what the fixture
/// is actually about, so the fixture must use the real assembler rather than
/// its own reconstruction of one.
fn assemble(events: &[ProviderStreamEvent]) -> Result<Vec<ToolCall>, ProviderError> {
    let mut assembler = ToolCallAssembler::default();
    for event in events {
        if let ProviderStreamEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments_fragment,
        } = event
        {
            assembler.push(*index, id.clone(), name.clone(), arguments_fragment);
        }
    }
    assembler.finish(&IdMinter::new())
}

/// Every surface a secret could plausibly escape through: the events, their
/// serialized form, and any error rendered three different ways.
fn surfaces(events: &[ProviderStreamEvent]) -> String {
    let mut out = format!("{events:?}");
    out.push_str(&serde_json::to_string(events).expect("serializable events"));
    if let Some(error) = error(events) {
        out.push_str(&format!(" {error} {error:?} {}", error.message));
    }
    out
}

// -- text --------------------------------------------------------------------

#[tokio::test]
async fn text_fragments_arrive_in_order_and_the_finish_reason_outlives_the_done_frame() {
    let second = frame(r#"{"choices":[{"delta":{"content":"lo, world"}}]}"#);
    // The second frame is cut in half so a chunk boundary lands inside a JSON
    // string: an adapter that parsed per transport read rather than per SSE
    // frame would fail here rather than concatenating.
    let (head, tail) = second.split_at(second.len() / 2);
    let events = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"Hel"}}]}"#),
        head.to_owned(),
        format!(
            "{tail}{}",
            frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#)
        ),
        done(),
    ])
    .await;

    assert_eq!(text_deltas(&events), vec!["Hel", "lo, world"]);
    assert_eq!(finish(&events), Some(FinishReason::Stop));
    // Finish is terminal: nothing is normalized after it.
    assert!(matches!(
        events.last(),
        Some(ProviderStreamEvent::Finish { .. })
    ));
}

#[tokio::test]
async fn a_length_finish_reason_survives_instead_of_flattening_to_stop() {
    // `[DONE]` carries no reason of its own, so the reason seen earlier in the
    // stream has to be remembered. Defaulting to `Stop` would hide a truncated
    // answer from the user and from the loop that decides whether to continue.
    let events = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"as far as I got"}}]}"#),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#),
        done(),
    ])
    .await;

    assert_eq!(finish(&events), Some(FinishReason::Length));
}

#[tokio::test]
async fn keepalive_comments_and_empty_deltas_do_not_become_text_events() {
    // Compatible endpoints send `:` comment lines to hold the connection open
    // and `content: ""` deltas around role announcements. Surfacing either as
    // text would put empty fragments into the transcript.
    let events = events_from(vec![
        ": keepalive\n\n".to_owned(),
        frame(r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":"answer"}}]}"#),
        done(),
    ])
    .await;

    assert_eq!(text_deltas(&events), vec!["answer"]);
    assert_eq!(finish(&events), Some(FinishReason::Stop));
}

// -- fragmented tool calls ---------------------------------------------------

#[tokio::test]
async fn a_tool_call_split_across_frames_and_interleaved_with_text_becomes_one_call() {
    // The name arrives alone, the arguments arrive in three pieces, and text is
    // interleaved between them — the shape a real endpoint produces when the
    // model narrates before calling a tool.
    let events = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"Reading it now. "}}]}"#),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"read"}}]}}]}"#,
        ),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#,
        ),
        frame(r#"{"choices":[{"delta":{"content":"One moment."}}]}"#),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"src/lib"}}]}}]}"#,
        ),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":".rs\"}"}}]}}]}"#,
        ),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#),
        done(),
    ])
    .await;

    let calls = assemble(&events).expect("one well-formed call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].arguments, json!({"path": "src/lib.rs"}));
    // The wire id is kept rather than replaced by a minted one, because the
    // tool result has to be correlated back to what the provider asked for.
    assert_eq!(calls[0].id.as_str(), "call_abc");
    // The narration is untouched by the tool traffic woven through it.
    assert_eq!(text_deltas(&events).concat(), "Reading it now. One moment.");
    assert_eq!(finish(&events), Some(FinishReason::ToolCalls));
}

#[tokio::test]
async fn two_tool_calls_interleaved_by_index_become_two_calls_in_index_order() {
    // Both slots advance in the same frame and the arguments of the second are
    // completed before the first, so a fragment routed by arrival order rather
    // than by `index` would cross-contaminate the two calls.
    let events = events_from(vec![
        frame(concat!(
            r#"{"choices":[{"delta":{"tool_calls":["#,
            r#"{"index":0,"id":"call_1","function":{"name":"read","arguments":"{\"path\":\"a"}},"#,
            r#"{"index":1,"id":"call_2","function":{"name":"list","arguments":"{\"dir\":"}}"#,
            r#"]}}]}"#,
        )),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"src\"}"}}]}}]}"#,
        ),
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":".rs\"}"}}]}}]}"#,
        ),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#),
        done(),
    ])
    .await;

    let calls = assemble(&events).expect("two well-formed calls");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "read");
    assert_eq!(calls[0].arguments, json!({"path": "a.rs"}));
    assert_eq!(calls[1].name, "list");
    assert_eq!(calls[1].arguments, json!({"dir": "src"}));
}

#[tokio::test]
async fn a_tool_call_whose_arguments_never_complete_is_a_classified_error_not_a_guess() {
    // The stream ends after a valid-looking prefix. Nothing may execute on a
    // half-parsed argument object (`provider-runtime/spec.md`: Smith executes
    // nothing until shared validation completes).
    let events = events_from(vec![
        frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_9","function":{"name":"shell","arguments":"{\"command\":\"rm -"}}]}}]}"#,
        ),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#),
        done(),
    ])
    .await;

    let failure = assemble(&events).expect_err("a truncated argument object is refused");
    assert_eq!(failure.kind, ProviderErrorKind::MalformedStream);
    assert!(failure.message.contains("shell"), "{}", failure.message);
}

// -- usage -------------------------------------------------------------------

#[tokio::test]
async fn reported_usage_lands_in_disjoint_counters_without_counting_cached_input_twice() {
    // The exact figures from `usage-accounting/spec.md`: 8,000 cache-read,
    // 500 uncached, 300 output. The wire reports `prompt_tokens` inclusive of
    // the cached part, so the adapter must subtract rather than add.
    let events = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"hi"}}]}"#),
        frame(concat!(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"#,
            r#""usage":{"prompt_tokens":8500,"completion_tokens":300,"#,
            r#""prompt_tokens_details":{"cached_tokens":8000}}}"#,
        )),
        done(),
    ])
    .await;

    let deltas = usage(&events);
    assert_eq!(deltas.len(), 1);
    let delta = &deltas[0];
    assert_eq!(delta.get(CounterKind::InputUncached), 500);
    assert_eq!(delta.get(CounterKind::InputCached), 8_000);
    assert_eq!(delta.get(CounterKind::Output), 300);
    // Disjoint: the rollup is the plain sum, with the 8,000 counted once.
    assert_eq!(delta.total(), 8_800);

    assert!(events.iter().any(|event| matches!(
        event,
        ProviderStreamEvent::CacheObservation {
            read_tokens: 8_000,
            ..
        }
    )));

    // Usage precedes the finish, so a consumer that stops reading at `Finish`
    // still accounts for the turn.
    let usage_at = events
        .iter()
        .position(|event| matches!(event, ProviderStreamEvent::Usage { .. }))
        .expect("a usage event");
    let finish_at = events
        .iter()
        .position(|event| matches!(event, ProviderStreamEvent::Finish { .. }))
        .expect("a finish event");
    assert!(usage_at < finish_at);
}

#[tokio::test]
async fn a_counter_the_provider_omits_is_absent_from_the_delta_rather_than_serialized_as_zero() {
    let omitted = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"hi"}}],"usage":{"prompt_tokens":120}}"#),
        done(),
    ])
    .await;
    let delta = usage(&omitted).pop().expect("a usage event");

    assert_eq!(delta.get(CounterKind::InputUncached), 120);
    // `iter` and the serialized form both omit the counter entirely, which is
    // how a consumer distinguishes it from a reported value.
    assert!(
        !delta.iter().any(|(kind, _)| kind == CounterKind::Output),
        "an unreported counter must not materialize: {delta:?}"
    );
    let encoded = serde_json::to_string(&delta).expect("a serializable delta");
    assert!(encoded.contains("input_uncached"), "{encoded}");
    assert!(!encoded.contains("output"), "{encoded}");
}

#[tokio::test]
async fn an_attempt_that_reports_no_usage_at_all_emits_no_usage_event() {
    // Absence of the event is the only representation of "unknown" the shared
    // vocabulary has; a zero-valued event would be a claim the provider never
    // made (design.md Decision 5).
    let events = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"hi"}}]}"#),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
        done(),
    ])
    .await;

    assert!(usage(&events).is_empty());
    assert_eq!(finish(&events), Some(FinishReason::Stop));
}

#[tokio::test]
async fn upstream_an_unreported_counter_is_indistinguishable_from_a_reported_zero() {
    // UPSTREAM FINDING. `usage-accounting/spec.md` requires every counter to
    // keep a `provider_reported` / `unknown` (and friends) accuracy label, and
    // design.md Decision 5 forbids treating unknown as zero. Agent Runtime's
    // `UsageDelta` has no such label: `Provenance` records which request and
    // attempt produced a record, not how accurate each counter is. `with` drops
    // zero values and `get` answers 0 for a missing key, so a provider that
    // reported zero output and a provider that reported nothing produce byte-
    // identical deltas. Recorded, not endorsed — Smith may not change the
    // shared runtime under this approval. This test flips the moment a real
    // per-counter provenance lands upstream.
    let omitted = events_from(vec![
        frame(r#"{"choices":[],"usage":{"prompt_tokens":120}}"#),
        done(),
    ])
    .await;
    let explicit_zero = events_from(vec![
        frame(r#"{"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":0}}"#),
        done(),
    ])
    .await;

    assert_eq!(usage(&omitted), usage(&explicit_zero));
    let delta = usage(&omitted).pop().expect("a usage event");
    assert_eq!(delta.get(CounterKind::Output), 0);
}

#[tokio::test]
async fn upstream_cache_writes_are_reported_as_zero_although_this_adapter_cannot_observe_them() {
    // UPSTREAM FINDING, same root cause as above. Chat Completions exposes
    // `cached_tokens` but nothing about tokens written *into* the cache, so the
    // write count for this adapter is genuinely unknown. It is emitted as a
    // hard `0` in `CacheObservation` and the `CacheWrite` counter never
    // appears, so a consumer cannot tell "nothing was cached" from "we cannot
    // see". Smith must therefore label cache-write unknown for this adapter
    // rather than trusting the zero it is handed.
    let events = events_from(vec![
        frame(concat!(
            r#"{"choices":[],"usage":{"prompt_tokens":900,"completion_tokens":10,"#,
            r#""prompt_tokens_details":{"cached_tokens":800}}}"#,
        )),
        done(),
    ])
    .await;

    let observation = events
        .iter()
        .find_map(|event| match event {
            ProviderStreamEvent::CacheObservation {
                read_tokens,
                write_tokens,
            } => Some((*read_tokens, *write_tokens)),
            _ => None,
        })
        .expect("a cache observation");
    assert_eq!(observation, (800, 0));

    let delta = usage(&events).pop().expect("a usage event");
    assert!(
        !delta
            .iter()
            .any(|(kind, _)| kind == CounterKind::CacheWrite),
        "a cache-write counter appeared from nowhere: {delta:?}"
    );
}

// -- retry -------------------------------------------------------------------

/// Drives `provider` the way the shared loop does — one entry per attempt, kept
/// whether it succeeded or failed — consulting the shared classifier and the
/// shared policy for the decision to try again.
async fn attempts_under(
    provider: &OpenAiProvider<ScriptedTransport>,
    policy: RetryPolicy,
) -> Vec<Result<Vec<ProviderStreamEvent>, ProviderError>> {
    let mut attempts: Vec<Result<Vec<ProviderStreamEvent>, ProviderError>> = Vec::new();
    let mut index = 0u32;
    loop {
        let outcome = match provider
            .stream(request(), call_context(Cancellation::new()))
            .await
        {
            Ok(stream) => Ok(within("the attempt", drain(stream)).await),
            Err(failure) => Err(failure),
        };
        let again = match &outcome {
            Err(failure) => {
                // An immediate policy computes a zero backoff, so no fixture
                // here ever sleeps.
                assert_eq!(policy.backoff_ms(index, failure), 0);
                is_retryable(failure) && policy.allows_retry(index)
            }
            Ok(_) => false,
        };
        attempts.push(outcome);
        if !again {
            return attempts;
        }
        index += 1;
    }
}

#[tokio::test]
async fn a_retryable_refusal_is_followed_by_a_second_attempt_that_both_remain_visible() {
    let (seen, provider) = scripted(vec![
        Reply::Refuse(
            ProviderError::new(ProviderErrorKind::Server, "upstream is unwell").retryable(),
        ),
        Reply::Body(vec![
            frame(r#"{"choices":[{"delta":{"content":"second time lucky"}}]}"#),
            frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
            done(),
        ]),
    ]);

    let attempts = attempts_under(&provider, RetryPolicy::immediate(2)).await;

    assert_eq!(attempts.len(), 2);
    let first = attempts[0].as_ref().expect_err("the first attempt failed");
    assert_eq!(first.kind, ProviderErrorKind::Server);
    assert!(is_retryable(first));
    let second = attempts[1].as_ref().expect("the second attempt succeeded");
    assert_eq!(text_deltas(second), vec!["second time lucky"]);
    assert_eq!(finish(second), Some(FinishReason::Stop));

    // Two attempts really reached the transport, and the retry re-sent exactly
    // the same request rather than a differently built one.
    let requests = sent(&seen);
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(requests[0].url, requests[1].url);
}

#[tokio::test]
async fn a_terminal_refusal_stops_the_loop_before_a_second_attempt_is_made() {
    // The script holds a reply the loop must never reach: if the classifier
    // called a `BadRequest` retryable, the fixture would consume it and the
    // attempt count would be wrong.
    let (seen, provider) = scripted(vec![
        Reply::Refuse(ProviderError::new(
            ProviderErrorKind::BadRequest,
            "the model rejected the request",
        )),
        Reply::Body(vec![done()]),
    ]);

    let attempts = attempts_under(&provider, RetryPolicy::immediate(3)).await;

    assert_eq!(attempts.len(), 1);
    let only = attempts[0].as_ref().expect_err("the attempt failed");
    assert!(!is_retryable(only));
    assert_eq!(sent(&seen).len(), 1);
}

#[tokio::test]
async fn a_retry_after_hint_survives_the_adapter_and_reaches_the_shared_backoff() {
    // Smith's transport is what reads `retry-after` off the response; the
    // adapter must pass the parsed hint through untouched or the shared policy
    // silently falls back to its own exponential curve.
    let (_seen, provider) = scripted(vec![Reply::Refuse(
        ProviderError::new(ProviderErrorKind::RateLimited, "slow down").retry_after(4_000),
    )]);

    let failure = provider
        .stream(request(), call_context(Cancellation::new()))
        .await
        .err()
        .expect("the refusal reaches the caller");

    assert_eq!(failure.kind, ProviderErrorKind::RateLimited);
    assert_eq!(failure.retry_after_ms, Some(4_000));
    assert!(is_retryable(&failure));
    // The default policy's own first backoff is 200 ms; the provider's hint wins.
    assert_eq!(RetryPolicy::default().backoff_ms(0, &failure), 4_000);
}

#[tokio::test]
async fn a_body_that_dies_midway_reports_in_stream_and_keeps_the_text_it_had() {
    // A failure after the response head cannot be a refused request: the
    // adapter already returned a stream. It has to arrive as a terminal event,
    // with the partial text preserved and no finish claiming success.
    let (_seen, provider) = scripted(vec![Reply::Dies(
        vec![frame(
            r#"{"choices":[{"delta":{"content":"half an ans"}}]}"#,
        )],
        ProviderError::new(ProviderErrorKind::Network, "connection reset").retryable(),
    )]);

    let stream = provider
        .stream(request(), call_context(Cancellation::new()))
        .await
        .expect("a stream");
    let events = within("the stream", drain(stream)).await;

    assert_eq!(text_deltas(&events), vec!["half an ans"]);
    let failure = error(&events).expect("a terminal error event");
    assert_eq!(failure.kind, ProviderErrorKind::Network);
    assert!(is_retryable(failure));
    assert_eq!(finish(&events), None, "a dead body did not finish");
    assert!(matches!(
        events.last(),
        Some(ProviderStreamEvent::Error { .. })
    ));
}

// -- cancellation ------------------------------------------------------------

#[tokio::test]
async fn cancelling_between_frames_ends_the_stream_with_a_cancelled_error_and_no_finish() {
    let provider = replaying(vec![
        frame(r#"{"choices":[{"delta":{"content":"starting"}}]}"#),
        frame(r#"{"choices":[{"delta":{"content":" and continuing"}}]}"#),
        frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#),
        done(),
    ]);
    let cancel = Cancellation::new();
    let mut stream = provider
        .stream(request(), call_context(cancel.clone()))
        .await
        .expect("a stream");

    let first = within("the first event", stream.next())
        .await
        .expect("a first event");
    assert!(matches!(first, ProviderStreamEvent::TextDelta { .. }));

    cancel.cancel(CancelReason::UserRequested);
    let rest = within("the cancelled stream", drain(stream)).await;

    // The bytes for the rest of the answer were already queued in the
    // transport, so a stream that ignored cancellation would have finished
    // successfully instead.
    assert!(text_deltas(&rest).is_empty(), "{rest:?}");
    assert_eq!(finish(&rest), None, "a cancelled attempt did not finish");
    let failure = error(&rest).expect("a cancelled error event");
    assert_eq!(failure.kind, ProviderErrorKind::Cancelled);
    assert!(!failure.retryable, "cancellation is not a transient fault");
    assert!(
        matches!(rest.last(), Some(ProviderStreamEvent::Error { .. })),
        "cancellation is terminal: {rest:?}"
    );
}

#[tokio::test]
async fn a_request_cancelled_before_it_starts_never_reaches_the_transport() {
    // Cancelling between the decision to call and the call itself must not
    // spend anything. The script holds a perfectly good reply, so a regression
    // would produce a successful turn rather than an obvious crash; the request
    // log is what actually catches it.
    let (seen, provider) = scripted(vec![Reply::Body(vec![done()])]);
    let cancel = Cancellation::new();
    cancel.cancel(CancelReason::Shutdown);

    let failure = provider
        .stream(request(), call_context(cancel))
        .await
        .err()
        .expect("a cancelled request does not become a stream");

    assert_eq!(failure.kind, ProviderErrorKind::Cancelled);
    assert!(sent(&seen).is_empty(), "a cancelled request was still sent");
}

#[tokio::test]
async fn cancelling_a_stream_that_never_receives_a_byte_ends_it_instead_of_hanging() {
    // The only fixture here where a hang is genuinely possible: the transport
    // accepted the request and will never speak again. `within` turns a
    // regression into a failure rather than a stuck suite.
    let (_seen, provider) = scripted(vec![Reply::Silent]);
    let cancel = Cancellation::new();
    let stream = provider
        .stream(request(), call_context(cancel.clone()))
        .await
        .expect("a stream");

    cancel.cancel(CancelReason::UserRequested);
    let events = within("the silent stream", drain(stream)).await;

    assert_eq!(
        error(&events).map(|failure| failure.kind),
        Some(ProviderErrorKind::Cancelled)
    );
}

// -- malformed SSE -----------------------------------------------------------

#[tokio::test]
async fn a_frame_truncated_by_a_dropped_connection_is_a_classified_malformed_stream() {
    // The body ends mid-JSON with no blank line, so the frame is only visible
    // to the end-of-stream flush. Discarding it would turn a truncated answer
    // into a silently short but successful one.
    let events = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"the beginning"}}]}"#),
        r#"data: {"choices":[{"delta":{"cont"#.to_owned(),
    ])
    .await;

    assert_eq!(text_deltas(&events), vec!["the beginning"]);
    let failure = error(&events).expect("a classified error");
    assert_eq!(failure.kind, ProviderErrorKind::MalformedStream);
    assert_eq!(finish(&events), None);
}

#[tokio::test]
async fn an_unparseable_json_payload_ends_the_stream_with_a_malformed_stream_error() {
    let events = events_from(vec![frame("{this is not json}"), done()]).await;

    let failure = error(&events).expect("a classified error");
    assert_eq!(failure.kind, ProviderErrorKind::MalformedStream);
    // Terminal: the `[DONE]` behind it is never reached, so no finish is
    // fabricated for a stream that failed.
    assert_eq!(finish(&events), None);
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn a_body_cut_inside_a_multibyte_character_is_reported_rather_than_patched_over() {
    // A UTF-8 sequence split across transport reads is normal and must be
    // buffered; one that is never completed is corruption and must be said so,
    // not replaced with U+FFFD.
    let complete = frame(r#"{"choices":[{"delta":{"content":"café"}}]}"#).into_bytes();
    // Cut one byte into the two-byte `é`.
    let boundary = complete
        .windows(2)
        .position(|pair| pair == "é".as_bytes())
        .expect("the multibyte character")
        + 1;
    let truncated = complete[..boundary].to_vec();

    let provider = OpenAiProvider::new(ReplayTransport::new(vec![truncated]), config());
    let stream = provider
        .stream(request(), call_context(Cancellation::new()))
        .await
        .expect("a stream");
    let events = within("the stream", drain(stream)).await;

    let failure = error(&events).expect("a classified error");
    assert_eq!(failure.kind, ProviderErrorKind::MalformedStream);
    assert!(failure.message.contains("UTF-8"), "{}", failure.message);
}

#[tokio::test]
async fn an_in_stream_provider_error_is_terminal_and_never_looks_successful() {
    // Compatible endpoints may report a mid-stream failure as a named
    // `event: error` frame. The adapter must terminate on that frame rather
    // than ignoring its event name and accepting the following `[DONE]`.
    let events = events_from(vec![
        "event: error\ndata: {\"error\":{\"message\":\"the model is overloaded\",\"type\":\"server_error\"}}\n\n"
            .to_owned(),
        done(),
    ])
    .await;

    let failure = error(&events).expect("a terminal provider error");
    assert_eq!(failure.kind, ProviderErrorKind::Server);
    assert!(failure.retryable);
    assert_eq!(finish(&events), None);
    assert_eq!(events.len(), 1, "the trailing [DONE] must not be consumed");
}

// -- authorization redaction -------------------------------------------------

#[tokio::test]
async fn the_key_travels_on_the_wire_but_appears_in_no_debug_rendering() {
    let (seen, provider) = scripted(vec![Reply::Body(vec![
        frame(r#"{"choices":[{"delta":{"content":"ok"}}]}"#),
        done(),
    ])]);
    let stream = provider
        .stream(request(), call_context(Cancellation::new()))
        .await
        .expect("a stream");
    let _events = within("the stream", drain(stream)).await;

    let requests = sent(&seen);
    let sent_request = requests.first().expect("one request");

    // Non-vacuous: the credential really is on the request, as a bearer token,
    // and the prompt really is in the body.
    assert!(
        sent_request
            .headers
            .iter()
            .any(|(name, value)| name == "authorization" && value.contains(API_KEY)),
        "the adapter did not send the credential at all"
    );
    let body = String::from_utf8(sent_request.body.clone()).expect("a utf-8 body");
    assert!(body.contains(PROMPT));

    // ...and none of it survives being rendered.
    for rendered in [format!("{sent_request:?}"), format!("{provider:?}")] {
        assert!(!rendered.contains(API_KEY), "the key leaked: {rendered}");
        assert!(!rendered.contains(PROMPT), "the prompt leaked: {rendered}");
    }
    // The renderings are not merely empty.
    assert!(format!("{sent_request:?}").contains("authorization"));
    assert!(format!("{provider:?}").contains("test-model"));
}

#[tokio::test]
async fn a_key_the_server_echoes_back_reaches_no_event_and_no_error() {
    // The nastiest shape: an endpoint that quotes the offending credential in
    // its own error body, and does it inside a payload the adapter cannot
    // parse — the one place a naive implementation attaches the raw data to the
    // error it raises.
    let events = events_from(vec![
        frame(r#"{"choices":[{"delta":{"content":"before the failure"}}]}"#),
        format!("data: {{\"error\": bad key Bearer {API_KEY} for {PROMPT}}}\n\n"),
        done(),
    ])
    .await;

    let failure = error(&events).expect("a classified error");
    assert_eq!(failure.kind, ProviderErrorKind::MalformedStream);

    let exposed = surfaces(&events);
    for secret in [API_KEY, PROMPT] {
        assert!(
            !exposed.contains(secret),
            "`{secret}` leaked into an adapter surface: {exposed}"
        );
    }
    // The surfaces are not merely empty: the text before the failure is there.
    assert!(exposed.contains("before the failure"));
}
