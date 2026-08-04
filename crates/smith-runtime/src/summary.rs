//! Smith semantic-summary model and policy composition.
//!
//! The generic coordinator owns durable originals, validation, protected
//! state, history projection, and fallback. Smith selects the product policy
//! and adapts the already-resolved provider/model to the separately attributed
//! `context.semantic_summary` purpose.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use agent_runtime::context::Sensitivity;
use agent_runtime::harness::{
    SEMANTIC_SUMMARY_PURPOSE, SemanticSummaryPolicy, SummaryModel, SummaryModelRequest,
    SummaryModelResponse,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::artifact::ArtifactRetention;
use agent_runtime_core::cancel::{CancelReason, Cancellation};
use agent_runtime_core::clock::{Clock, Deadline};
use agent_runtime_core::content::Message;
use agent_runtime_core::error::{ErrorKind, RuntimeError};
use agent_runtime_core::ids::{AttemptId, RequestId, SessionId};
use agent_runtime_core::provider::{
    FinishReason, ModelId, Provider, ProviderCallContext, ProviderRequest, ProviderStreamEvent,
    ToolChoice,
};
use agent_runtime_core::usage::UsageDelta;
use async_trait::async_trait;
use futures_util::StreamExt;

/// Smith's semantic-summary policy revision.
pub const SMITH_SEMANTIC_SUMMARY_POLICY_REVISION: &str = "smith-semantic-summary-policy-1";
/// Smith's provider adapter revision.
pub const SMITH_SUMMARY_MODEL_ADAPTER_REVISION: &str = "smith-summary-model-adapter-1";
/// Default provider-call timeout for summary work.
pub const DEFAULT_SUMMARY_TIMEOUT_MS: u64 = 30_000;
/// Separate output token cap for summary work.
pub const DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS: u32 = 2_048;

const SUMMARY_INSTRUCTION: &str = "\
Summarize the supplied completed conversation turns for future task continuity. \
Preserve decisions, constraints, file/symbol names, tool evidence, unresolved work, \
and user preferences. Do not invent results, permissions, or facts. Do not issue \
tool calls. Return only the concise semantic summary.";

/// Smith-selected semantic summary composition.
#[derive(Clone)]
pub struct SmithSemanticSummaryConfig {
    /// Generic coordinator policy, including spend/sensitivity/retention.
    pub policy: SemanticSummaryPolicy,
    /// Dedicated host model override. `None` adapts the selected run provider
    /// and model with a separately versioned summary prompt.
    pub model: Option<Arc<dyn SummaryModel>>,
    /// Provider generation cap for the standard adapter.
    pub max_output_tokens: u32,
    /// Provider wall-clock bound for the standard adapter.
    pub timeout_ms: u64,
}

impl fmt::Debug for SmithSemanticSummaryConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithSemanticSummaryConfig")
            .field("policy", &self.policy)
            .field(
                "model",
                &self
                    .model
                    .as_ref()
                    .map(|model| (model.id(), model.revision())),
            )
            .field("max_output_tokens", &self.max_output_tokens)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl SmithSemanticSummaryConfig {
    /// Smith's standard persisted-session policy.
    pub fn standard() -> Self {
        Self {
            policy: SemanticSummaryPolicy {
                // A floor, not a trigger: pressure decides when, this only
                // prevents summarizing a session too young to be worth it.
                min_turns: 4,
                trigger_percent: 85,
                // Filled in from the resolved model limits by the factory; a
                // zero budget fails validation rather than silently disabling
                // summarization.
                input_budget_tokens: 0,
                retain_turns: 2,
                max_summary_chars: 8_000,
                max_usage_tokens: 32_000,
                sensitivity: Sensitivity::Sensitive,
                retention: ArtifactRetention::Session,
                ..SemanticSummaryPolicy::new(RegistryRevision::new(
                    SMITH_SEMANTIC_SUMMARY_POLICY_REVISION,
                ))
            },
            model: None,
            max_output_tokens: DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS,
            timeout_ms: DEFAULT_SUMMARY_TIMEOUT_MS,
        }
    }

    /// Validates Smith-specific provider bounds plus the generic policy.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        self.policy.validate()?;
        if self.max_output_tokens == 0 || self.timeout_ms == 0 {
            return Err(RuntimeError::config(
                "Smith semantic summary output and timeout limits must be positive",
            ));
        }
        Ok(())
    }
}

/// Safe policy projection retained on [`crate::factory::RuntimePolicy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSummaryRuntimePolicy {
    /// Separately attributed purpose.
    pub purpose: String,
    /// Dedicated model/profile identity.
    pub model: String,
    /// Smith policy revision.
    pub revision: RegistryRevision,
    /// Completed-turn eligibility floor.
    pub min_turns: usize,
    /// Share of the post-opening input budget that triggers summarization.
    pub trigger_percent: u8,
    /// Resolved input budget the share applies to.
    pub input_budget_tokens: u64,
    /// Recent turns retained verbatim.
    pub retain_turns: usize,
    /// Maximum separately attributed usage.
    pub max_usage_tokens: u64,
    /// Original retention policy.
    pub retention: ArtifactRetention,
}

/// Provider/model adapter used only by the semantic-summary hook.
pub struct SmithProviderSummaryModel {
    provider: Arc<dyn Provider>,
    provider_name: String,
    model: ModelId,
    clock: Arc<dyn Clock>,
    max_output_tokens: u32,
    timeout_ms: u64,
    revision: RegistryRevision,
    id: String,
}

impl SmithProviderSummaryModel {
    /// Adapts one already-resolved Smith provider/model.
    pub fn new(
        provider: Arc<dyn Provider>,
        provider_name: impl Into<String>,
        model: ModelId,
        clock: Arc<dyn Clock>,
        max_output_tokens: u32,
        timeout_ms: u64,
    ) -> Result<Self, RuntimeError> {
        if max_output_tokens == 0 || timeout_ms == 0 {
            return Err(RuntimeError::config(
                "semantic summary provider bounds must be positive",
            ));
        }
        let provider_name = provider_name.into();
        if provider.capabilities(&model).is_none() {
            return Err(RuntimeError::config(format!(
                "semantic summary provider does not serve model `{model}`"
            )));
        }
        let id = format!("{provider_name}/{model}:semantic-summary");
        let revision = RegistryRevision::from_content(format!(
            "{SMITH_SUMMARY_MODEL_ADAPTER_REVISION}\n{provider_name}\n{model}\n\
             {max_output_tokens}\n{timeout_ms}\n{SUMMARY_INSTRUCTION}"
        ));
        Ok(Self {
            provider,
            provider_name,
            model,
            clock,
            max_output_tokens,
            timeout_ms,
            revision,
            id,
        })
    }
}

impl fmt::Debug for SmithProviderSummaryModel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmithProviderSummaryModel")
            .field("provider", &self.provider_name)
            .field("model", &self.model)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("timeout_ms", &self.timeout_ms)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SummaryModel for SmithProviderSummaryModel {
    fn id(&self) -> &str {
        &self.id
    }

    fn revision(&self) -> RegistryRevision {
        self.revision.clone()
    }

    async fn summarize(
        &self,
        request: &SummaryModelRequest,
    ) -> Result<SummaryModelResponse, RuntimeError> {
        if request.purpose != SEMANTIC_SUMMARY_PURPOSE {
            return Err(RuntimeError::config(
                "Smith summary adapter received an unexpected purpose",
            ));
        }
        let mut messages = Vec::with_capacity(request.messages.len() + 1);
        messages.push(Message::system(format!(
            "<summary-purpose id=\"{}\" max_chars=\"{}\">\n{}\n</summary-purpose>",
            request.purpose, request.max_output_chars, SUMMARY_INSTRUCTION
        )));
        messages.extend(request.messages.iter().cloned());
        let mut provider_request = ProviderRequest::new(self.model.clone(), messages);
        provider_request.tool_choice = ToolChoice::None;
        provider_request.max_output_tokens = Some(self.max_output_tokens);

        let cancel = Cancellation::new();
        let context = ProviderCallContext {
            // Summary work is separately attributed and must not share the
            // main conversation's cache partition: its prefix is a different
            // prompt entirely.
            session: SessionId::new(format!("summary-{}", request.idempotency_key)),
            request_id: RequestId::new(format!("summary-{}", request.idempotency_key)),
            attempt_id: AttemptId::new(format!("summary-{}", request.idempotency_key)),
            cancel: cancel.clone(),
            deadline: Deadline::after(self.clock.as_ref(), self.timeout_ms),
        };
        let call = async {
            let mut stream = self.provider.stream(provider_request, context).await?;
            let mut text = String::new();
            let mut usage = UsageDelta::new();
            let mut finish = None;
            while let Some(event) = stream.next().await {
                match event {
                    ProviderStreamEvent::TextDelta { text: delta } => {
                        text.push_str(&delta);
                        if text.chars().count() > request.max_output_chars {
                            return Err(RuntimeError::limit(
                                "semantic summary exceeded its character limit",
                            ));
                        }
                    }
                    ProviderStreamEvent::Usage { delta } => usage.merge(&delta),
                    ProviderStreamEvent::Finish { reason } => {
                        finish = Some(reason);
                        break;
                    }
                    ProviderStreamEvent::Error { error } => return Err(error.into()),
                    ProviderStreamEvent::ToolCallDelta { .. } => {
                        return Err(RuntimeError::new(
                            ErrorKind::Provider,
                            "semantic summary model attempted a tool call",
                        ));
                    }
                    ProviderStreamEvent::ReasoningDelta { .. }
                    | ProviderStreamEvent::CacheObservation { .. }
                    | ProviderStreamEvent::RateLimit { .. }
                    | ProviderStreamEvent::Downgrade { .. }
                    | ProviderStreamEvent::VendorMetadata { .. } => {}
                }
            }
            if finish != Some(FinishReason::Stop) {
                return Err(RuntimeError::new(
                    ErrorKind::Provider,
                    "semantic summary model did not finish normally",
                ));
            }
            Ok(SummaryModelResponse { text, usage })
        };
        match tokio::time::timeout(Duration::from_millis(self.timeout_ms), call).await {
            Ok(result) => result,
            Err(_) => {
                cancel.cancel(CancelReason::Timeout);
                Err(RuntimeError::new(
                    ErrorKind::Timeout,
                    "semantic summary model timed out",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use agent_runtime::provider::fake::{FakeProvider, ScriptedStream};
    use agent_runtime_core::clock::SystemClock;
    use agent_runtime_core::provider::{Capabilities, ProviderStreamEvent};
    use agent_runtime_core::usage::CounterKind;

    use super::*;

    #[tokio::test]
    async fn adapter_uses_no_tools_and_keeps_usage_disjoint() {
        let provider = Arc::new(FakeProvider::new(
            "model",
            Capabilities::basic_streaming(),
            vec![ScriptedStream::new(vec![
                ProviderStreamEvent::TextDelta {
                    text: "bounded summary".into(),
                },
                ProviderStreamEvent::Usage {
                    delta: UsageDelta::new().with(CounterKind::Output, 4),
                },
                ProviderStreamEvent::Finish {
                    reason: FinishReason::Stop,
                },
            ])],
        ));
        let model = SmithProviderSummaryModel::new(
            provider.clone(),
            "fake",
            ModelId::new("model"),
            Arc::new(SystemClock),
            128,
            1_000,
        )
        .unwrap();
        let response = model
            .summarize(&SummaryModelRequest {
                messages: Arc::from(vec![Message::user("old request")]),
                purpose: SEMANTIC_SUMMARY_PURPOSE.into(),
                idempotency_key: "stable".into(),
                max_output_chars: 256,
            })
            .await
            .unwrap();
        assert_eq!(response.text, "bounded summary");
        assert_eq!(response.usage.get(CounterKind::Output), 4);
        let request = &provider.requests()[0];
        assert!(request.tools.is_empty());
        assert_eq!(request.tool_choice, ToolChoice::None);
        assert!(
            request.messages[0]
                .joined_text()
                .contains(SEMANTIC_SUMMARY_PURPOSE)
        );
    }
}
