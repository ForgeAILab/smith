//! Smith's retry delays, carried to the shared retry loop as minimum waits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_runtime::provider::retry::is_retryable;
use agent_runtime_core::event::{EventEnvelope, RuntimeEvent};
use agent_runtime_core::ids::{AttemptId, SessionId};
use agent_runtime_core::observer::EventObserver;
use agent_runtime_core::provider::{
    CacheResourceProvider, Capabilities, ModelDescriptor, ModelId, Provider, ProviderCallContext,
    ProviderError, ProviderErrorKind, ProviderRequest, ProviderStream, ProviderStreamEvent,
};
use async_trait::async_trait;
use futures_util::StreamExt;

/// Ceiling of the first retry's jitter window.
pub(crate) const INITIAL_BACKOFF_MS: u64 = 1_000;
/// Ceiling of subsequent jitter windows; server hints may exceed it.
pub(crate) const MAX_BACKOFF_MS: u64 = 30_000;

/// Leaves retry admission, sleeping, cancellation, and events in the shared loop.
#[derive(Debug)]
pub(crate) struct ProviderBackoff {
    inner: Arc<dyn Provider>,
    attempts: Mutex<HashMap<(SessionId, AttemptId), u32>>,
}

impl ProviderBackoff {
    pub(crate) fn new(inner: Arc<dyn Provider>) -> Self {
        Self {
            inner,
            attempts: Mutex::new(HashMap::new()),
        }
    }
}

impl EventObserver for ProviderBackoff {
    fn observe(&self, event: &EventEnvelope) {
        match &event.payload {
            RuntimeEvent::ProviderAttemptStarted { attempt, index, .. } => {
                self.attempts
                    .lock()
                    .expect("provider backoff attempts poisoned")
                    .insert((event.session.clone(), attempt.clone()), *index);
            }
            RuntimeEvent::ProviderAttemptFinished { attempt, .. } => {
                self.attempts
                    .lock()
                    .expect("provider backoff attempts poisoned")
                    .remove(&(event.session.clone(), attempt.clone()));
            }
            _ => {}
        }
    }
}

#[async_trait]
impl Provider for ProviderBackoff {
    async fn prepare_request(
        &self,
        request: &ProviderRequest,
        ctx: &ProviderCallContext,
    ) -> Result<(), ProviderError> {
        self.inner.prepare_request(request, ctx).await
    }
    fn pre_output_replay_safe(&self, request: &ProviderRequest) -> bool {
        self.inner.pre_output_replay_safe(request)
    }
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    fn requires_nonempty_assistant_content(&self) -> bool {
        self.inner.requires_nonempty_assistant_content()
    }

    fn cache_resource_provider(&self) -> Option<&dyn CacheResourceProvider> {
        self.inner.cache_resource_provider()
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        // The start event supplies the authoritative index, including attempts
        // spent on credential recovery or argument correction. Consume it here
        // so cancelled streams cannot leave request state behind.
        let index = self
            .attempts
            .lock()
            .expect("provider backoff attempts poisoned")
            .remove(&(ctx.session.clone(), ctx.attempt_id.clone()))
            .unwrap_or(0);
        let stream = self
            .inner
            .stream(request, ctx)
            .await
            .map_err(|error| with_backoff(error, index))?;
        Ok(Box::pin(stream.map(move |event| match event {
            ProviderStreamEvent::Error { error } => ProviderStreamEvent::Error {
                error: with_backoff(error, index),
            },
            other => other,
        })))
    }
}

fn with_backoff(mut error: ProviderError, index: u32) -> ProviderError {
    if is_retryable(&error) {
        let delay = backoff_ms(index, uuid::Uuid::new_v4().as_u64_pair().1);
        let delay =
            if error.kind == ProviderErrorKind::RateLimited && error.retry_after_ms.is_none() {
                delay.max(2_000)
            } else {
                delay
            };
        error.retry_after_ms = Some(error.retry_after_ms.unwrap_or(0).max(delay));
    }
    error
}

fn backoff_ms(index: u32, entropy: u64) -> u64 {
    let ceiling = INITIAL_BACKOFF_MS
        .saturating_mul(2u64.saturating_pow(index))
        .min(MAX_BACKOFF_MS);
    let floor = ceiling / 2;
    floor + entropy % (ceiling - floor + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_cannot_overflow_or_escape_the_capped_window() {
        for index in [0, 1, 4, 5, 32, 64, u32::MAX] {
            let ceiling = match index {
                0 => 1_000,
                1 => 2_000,
                4 => 16_000,
                _ => MAX_BACKOFF_MS,
            };
            assert_eq!(backoff_ms(index, 0), ceiling / 2);
            assert_eq!(backoff_ms(index, ceiling / 2), ceiling);
            assert!((ceiling / 2..=ceiling).contains(&backoff_ms(index, u64::MAX)));
        }
    }
}
