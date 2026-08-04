//! Provider response compatibility owned by Smith.
//!
//! Some OpenAI-compatible endpoints emit a user-facing final answer only as
//! reasoning content. The shared adapter is correct to normalize that wire
//! field as reasoning; Smith's provider-specific policy is what may rescue a
//! *successful reasoning-only attempt* as visible text. The wrapper buffers
//! events until it can reclassify reasoning-only output as text or preserve the
//! original stream when visible text, tool calls, or an unsuccessful finish
//! make the reasoning classification meaningful.

use std::collections::VecDeque;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use agent_runtime_core::provider::{
    Capabilities, FinishReason, ModelDescriptor, ModelId, Provider, ProviderCallContext,
    ProviderError, ProviderRequest, ProviderStream, ProviderStreamEvent,
};
use async_trait::async_trait;
use futures_core::Stream;
use smith_config::model::ReasoningOnlyBehavior;

/// Applies the configured response policy without changing omitted/default
/// behavior.
pub fn apply_response_policy(
    provider: Arc<dyn Provider>,
    behavior: Option<ReasoningOnlyBehavior>,
) -> Arc<dyn Provider> {
    match behavior {
        Some(ReasoningOnlyBehavior::Text) => {
            Arc::new(ReasoningOnlyTextProvider { inner: provider })
        }
        None | Some(ReasoningOnlyBehavior::Reasoning) => provider,
    }
}

struct ReasoningOnlyTextProvider {
    inner: Arc<dyn Provider>,
}

impl fmt::Debug for ReasoningOnlyTextProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReasoningOnlyTextProvider")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl Provider for ReasoningOnlyTextProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        context: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        let inner = self.inner.stream(request, context).await?;
        Ok(Box::pin(ReasoningOnlyTextStream {
            inner,
            buffered: VecDeque::new(),
            ready: VecDeque::new(),
            reasoning: String::new(),
            classifying: true,
        }))
    }
}

struct ReasoningOnlyTextStream {
    inner: ProviderStream,
    buffered: VecDeque<ProviderStreamEvent>,
    ready: VecDeque<ProviderStreamEvent>,
    reasoning: String,
    classifying: bool,
}

impl ReasoningOnlyTextStream {
    fn preserve_buffered(&mut self) {
        self.ready.append(&mut self.buffered);
        self.reasoning.clear();
        self.classifying = false;
    }

    fn promote_buffered(&mut self) {
        let reasoning = std::mem::take(&mut self.reasoning);
        let mut promoted_text = Some(reasoning.trim_start().to_owned());

        while let Some(event) = self.buffered.pop_front() {
            match event {
                ProviderStreamEvent::ReasoningDelta {
                    redacted: false, ..
                } => {
                    if let Some(text) = promoted_text.take() {
                        self.ready
                            .push_back(ProviderStreamEvent::TextDelta { text });
                    }
                }
                event => self.ready.push_back(event),
            }
        }

        debug_assert!(promoted_text.is_none());
        self.classifying = false;
    }
}

impl Stream for ReasoningOnlyTextStream {
    type Item = ProviderStreamEvent;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Some(event) = this.ready.pop_front() {
                return Poll::Ready(Some(event));
            }
            if !this.classifying {
                return this.inner.as_mut().poll_next(context);
            }

            match this.inner.as_mut().poll_next(context) {
                Poll::Ready(Some(event)) => {
                    enum Action {
                        Buffer,
                        Preserve,
                        Promote,
                    }

                    let action = match &event {
                        ProviderStreamEvent::ReasoningDelta {
                            text,
                            redacted: false,
                            ..
                        } => {
                            this.reasoning.push_str(text);
                            Action::Buffer
                        }
                        ProviderStreamEvent::ReasoningDelta { redacted: true, .. }
                        | ProviderStreamEvent::TextDelta { .. }
                        | ProviderStreamEvent::ToolCallDelta { .. }
                        | ProviderStreamEvent::Error { .. } => Action::Preserve,
                        ProviderStreamEvent::Finish { reason }
                            if this
                                .reasoning
                                .chars()
                                .any(|character| !character.is_whitespace())
                                && matches!(reason, FinishReason::Stop | FinishReason::Length) =>
                        {
                            Action::Promote
                        }
                        ProviderStreamEvent::Finish { .. } => Action::Preserve,
                        ProviderStreamEvent::Usage { .. }
                        | ProviderStreamEvent::CacheObservation { .. }
                        | ProviderStreamEvent::RateLimit { .. }
                        | ProviderStreamEvent::Downgrade { .. }
                        | ProviderStreamEvent::VendorMetadata { .. } => Action::Buffer,
                    };

                    this.buffered.push_back(event);
                    match action {
                        Action::Buffer => continue,
                        Action::Preserve => this.preserve_buffered(),
                        Action::Promote => this.promote_buffered(),
                    }
                }
                Poll::Ready(None) => {
                    if this.buffered.is_empty() {
                        this.classifying = false;
                        return Poll::Ready(None);
                    }
                    this.preserve_buffered();
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use agent_runtime_core::provider::{AuthKind, ReasoningSupport};
    use agent_runtime_testkit::conformance::provider::call_ctx;
    use futures_util::{StreamExt, stream};

    #[derive(Debug)]
    struct ReplayProvider {
        events: Vec<ProviderStreamEvent>,
    }

    #[async_trait]
    impl Provider for ReplayProvider {
        fn describe(&self) -> Vec<ModelDescriptor> {
            vec![ModelDescriptor {
                id: ModelId::new("m"),
                display_name: "Replay".into(),
                vendor: "test".into(),
                capabilities: self.capabilities(&ModelId::new("m")).expect("caps"),
            }]
        }

        fn capabilities(&self, _model: &ModelId) -> Option<Capabilities> {
            Some(Capabilities {
                reasoning: ReasoningSupport::Fixed,
                auth: AuthKind::None,
                ..Capabilities::basic_streaming()
            })
        }

        async fn stream(
            &self,
            _request: ProviderRequest,
            _context: ProviderCallContext,
        ) -> Result<ProviderStream, ProviderError> {
            Ok(Box::pin(stream::iter(self.events.clone())))
        }
    }

    async fn replay(
        events: Vec<ProviderStreamEvent>,
        behavior: Option<ReasoningOnlyBehavior>,
    ) -> Vec<ProviderStreamEvent> {
        let provider = apply_response_policy(Arc::new(ReplayProvider { events }), behavior);
        let (context, _cancel) = call_ctx();
        provider
            .stream(ProviderRequest::new(ModelId::new("m"), Vec::new()), context)
            .await
            .expect("a stream")
            .collect()
            .await
    }

    fn reasoning(text: &str, redacted: bool) -> ProviderStreamEvent {
        ProviderStreamEvent::ReasoningDelta {
            text: text.into(),
            redacted,
            signature: None,
        }
    }

    fn finish() -> ProviderStreamEvent {
        ProviderStreamEvent::Finish {
            reason: FinishReason::Stop,
        }
    }

    #[tokio::test]
    async fn successful_reasoning_only_output_is_promoted_once_at_finish() {
        let events = replay(
            vec![
                reasoning("\n  hello ", false),
                reasoning("world", false),
                finish(),
            ],
            Some(ReasoningOnlyBehavior::Text),
        )
        .await;
        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::TextDelta {
                    text: "hello world".into()
                },
                finish(),
            ]
        );
    }

    #[tokio::test]
    async fn whitespace_only_reasoning_is_not_promoted_to_an_empty_answer() {
        let events = vec![reasoning("\n  ", false), finish()];
        assert_eq!(
            replay(events.clone(), Some(ReasoningOnlyBehavior::Text)).await,
            events
        );
    }

    #[tokio::test]
    async fn ordinary_text_keeps_prior_reasoning_classified_as_reasoning() {
        let events = vec![
            reasoning("private", false),
            ProviderStreamEvent::TextDelta {
                text: "visible".into(),
            },
            finish(),
        ];
        assert_eq!(
            replay(events.clone(), Some(ReasoningOnlyBehavior::Text)).await,
            events
        );
    }

    #[tokio::test]
    async fn a_tool_call_prevents_promotion_and_is_unchanged() {
        let events = vec![
            reasoning("planning", false),
            ProviderStreamEvent::ToolCallDelta {
                index: 0,
                id: Some("call-1".into()),
                name: Some("read".into()),
                arguments_fragment: "{}".into(),
            },
            ProviderStreamEvent::Finish {
                reason: FinishReason::ToolCalls,
            },
        ];
        assert_eq!(
            replay(events.clone(), Some(ReasoningOnlyBehavior::Text)).await,
            events
        );
    }

    #[tokio::test]
    async fn redacted_reasoning_is_never_promoted() {
        let events = vec![reasoning("[redacted]", true), finish()];
        assert_eq!(
            replay(events.clone(), Some(ReasoningOnlyBehavior::Text)).await,
            events
        );
    }

    #[tokio::test]
    async fn content_filtered_reasoning_is_not_promoted_as_a_successful_answer() {
        let events = vec![
            reasoning("provider-filtered draft", false),
            ProviderStreamEvent::Finish {
                reason: FinishReason::ContentFilter,
            },
        ];
        assert_eq!(
            replay(events.clone(), Some(ReasoningOnlyBehavior::Text)).await,
            events
        );
    }

    #[tokio::test]
    async fn usage_keeps_its_order_when_reasoning_is_reclassified() {
        let usage = ProviderStreamEvent::Usage {
            delta: Default::default(),
        };
        let events = replay(
            vec![reasoning("visible once", false), usage.clone(), finish()],
            Some(ReasoningOnlyBehavior::Text),
        )
        .await;
        assert_eq!(
            events,
            vec![
                ProviderStreamEvent::TextDelta {
                    text: "visible once".into()
                },
                usage,
                finish(),
            ]
        );
    }

    #[tokio::test]
    async fn unterminated_reasoning_is_preserved_instead_of_promoted() {
        let events = vec![reasoning("incomplete", false)];
        assert_eq!(
            replay(events.clone(), Some(ReasoningOnlyBehavior::Text)).await,
            events
        );
    }

    #[tokio::test]
    async fn omitted_or_reasoning_policy_preserves_openrouter_style_events() {
        let events = vec![
            ProviderStreamEvent::TextDelta {
                text: "ordinary response".into(),
            },
            finish(),
        ];
        assert_eq!(replay(events.clone(), None).await, events);

        let reasoning_events = vec![reasoning("still reasoning", false), finish()];
        assert_eq!(
            replay(
                reasoning_events.clone(),
                Some(ReasoningOnlyBehavior::Reasoning)
            )
            .await,
            reasoning_events
        );
    }
}
