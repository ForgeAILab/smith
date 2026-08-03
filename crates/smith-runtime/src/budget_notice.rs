//! A warning to the model that the context boundary is close.
//!
//! Compaction is not free. Summarizing rewrites history, which invalidates the
//! provider's cached prefix from the rewrite point onward, and because
//! summarization is most effective on the *oldest* history, an effective
//! compaction is also a maximally destructive one for cache. The model can
//! often avoid some of that cost if it knows the boundary is coming — finish
//! the current thread, write findings to a file, stop opening new ones.
//!
//! So this notice sits in [`ContextLane::TailContext`], after the conversation.
//! Two consequences, both deliberate:
//!
//! - **It cannot invalidate anything.** A provider cache prefix is a prefix;
//!   adding or removing the very last block leaves every earlier byte where it
//!   was. Recording the warning as a conversation message instead would append
//!   to history permanently and cost a real prefix extension.
//! - **It needs no one-shot claim flag.** Because it is re-rendered context
//!   rather than history, it is simply present while the condition holds and
//!   gone when it does not. That is both simpler than a claim flag and more
//!   useful: a model that ignored the warning on one turn sees it on the next.
//!
//! Pressure is observed at turn commit, where the usage ledger is available,
//! and projected at context assembly, where it is not.

use std::fmt;

use agent_runtime::context::{
    CacheClass, ContextFragment, ContextLane, ContextPosition, FragmentContent, FragmentKind,
    FragmentSource, Sensitivity,
};
use agent_runtime::harness::{
    ComponentDescriptor, ContextContributor, ContextPatch, ContextView, SessionStatePatch,
    TurnCommitHook, TurnCommitPatch, TurnCommitView,
};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::store::{SessionStateSensitivity, VersionedSessionState};
use agent_runtime_core::usage::{CounterKind, UsageRecord, UsageSource};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Component identity and state namespace.
pub const BUDGET_NOTICE_COMPONENT: &str = "smith.budget_notice";
/// State wire version.
pub const BUDGET_NOTICE_STATE_SCHEMA_VERSION: u32 = 1;
/// Default remaining-input threshold that arms the notice.
pub const DEFAULT_NOTICE_THRESHOLD_TOKENS: u64 = 12_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NoticeState {
    schema_version: u32,
    /// Remaining input tokens observed at the last commit.
    remaining_tokens: u64,
}

/// Observes context pressure and warns the model before the boundary.
#[derive(Clone)]
pub struct BudgetNoticeComponent {
    input_budget_tokens: u64,
    threshold_tokens: u64,
}

impl fmt::Debug for BudgetNoticeComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BudgetNoticeComponent")
            .field("input_budget_tokens", &self.input_budget_tokens)
            .field("threshold_tokens", &self.threshold_tokens)
            .finish()
    }
}

impl BudgetNoticeComponent {
    /// Creates a component measuring against a resolved input budget.
    pub fn new(input_budget_tokens: u64, threshold_tokens: u64) -> Result<Self, RuntimeError> {
        if input_budget_tokens == 0 {
            return Err(RuntimeError::config(
                "the budget notice needs a positive input budget to measure against",
            ));
        }
        if threshold_tokens == 0 || threshold_tokens >= input_budget_tokens {
            return Err(RuntimeError::config(
                "the budget notice threshold must be positive and smaller than the input budget",
            ));
        }
        Ok(Self {
            input_budget_tokens,
            threshold_tokens,
        })
    }

    fn descriptor_value() -> ComponentDescriptor {
        ComponentDescriptor::new(
            BUDGET_NOTICE_COMPONENT,
            RegistryRevision::new("smith-budget-notice-v1"),
        )
    }

    /// Remaining input budget implied by the newest provider attempt.
    fn remaining(&self, usage: &[UsageRecord]) -> Option<u64> {
        let latest = usage
            .iter()
            .filter(|record| record.source == UsageSource::ProviderAttempt)
            .map(|record| {
                record.delta.get(CounterKind::InputUncached)
                    + record.delta.get(CounterKind::InputCached)
            })
            .rfind(|tokens| *tokens > 0)?;
        Some(self.input_budget_tokens.saturating_sub(latest))
    }

    fn decode(&self, state: &VersionedSessionState) -> Result<NoticeState, RuntimeError> {
        let descriptor = Self::descriptor_value();
        if state.revision != *descriptor.revision() {
            return Err(RuntimeError::conflict(format!(
                "budget notice state revision `{}` is incompatible with `{}`",
                state.revision,
                descriptor.revision()
            )));
        }
        serde_json::from_value(state.value.clone())
            .map_err(|error| RuntimeError::internal(format!("budget notice state: {error}")))
    }

    fn body(remaining: u64) -> String {
        format!(
            "About {remaining} input tokens remain before this conversation is compacted. \
             Bring the current thread to a natural stopping point, and persist anything you \
             will need afterwards — findings to a file, progress to the plan — rather than \
             relying on earlier messages staying verbatim."
        )
    }
}

#[async_trait]
impl TurnCommitHook for BudgetNoticeComponent {
    fn descriptor(&self) -> ComponentDescriptor {
        Self::descriptor_value()
    }

    async fn after_commit(&self, view: &TurnCommitView) -> Result<TurnCommitPatch, RuntimeError> {
        let Some(remaining) = self.remaining(&view.usage) else {
            return Ok(TurnCommitPatch::default());
        };
        let state = NoticeState {
            schema_version: BUDGET_NOTICE_STATE_SCHEMA_VERSION,
            remaining_tokens: remaining,
        };
        let value = serde_json::to_value(&state)
            .map_err(|error| RuntimeError::internal(format!("budget notice state: {error}")))?;
        Ok(TurnCommitPatch {
            state: Some(SessionStatePatch {
                revision: Self::descriptor_value().revision().clone(),
                // Token counts carry no conversation content.
                sensitivity: SessionStateSensitivity::RedactionSafe,
                value,
            }),
            ..TurnCommitPatch::default()
        })
    }
}

#[async_trait]
impl ContextContributor for BudgetNoticeComponent {
    fn descriptor(&self) -> ComponentDescriptor {
        Self::descriptor_value()
    }

    async fn contribute(&self, view: &ContextView) -> Result<ContextPatch, RuntimeError> {
        let Some(persisted) = &view.state else {
            return Ok(ContextPatch::default());
        };
        let state = self.decode(persisted)?;
        if state.remaining_tokens > self.threshold_tokens {
            return Ok(ContextPatch::default());
        }
        let fragment = ContextFragment::new(
            "smith.budget-notice",
            FragmentKind::DeveloperInstruction,
            FragmentSource::Host,
            RegistryRevision::new("smith-budget-notice-v1"),
            FragmentContent::Text(Self::body(state.remaining_tokens)),
        )
        // Last lane, last position: nothing may follow, so nothing this
        // fragment does can shorten another block's cached prefix.
        .with_position(ContextPosition::new(ContextLane::TailContext, 0))
        .optional()
        .with_priority(100)
        .with_cache_class(CacheClass::Ephemeral)
        .with_sensitivity(Sensitivity::Internal);
        Ok(ContextPatch::new(vec![fragment]))
    }
}

#[cfg(test)]
mod tests {
    use agent_runtime_core::usage::{Provenance, UsageDelta};

    use super::*;

    fn attempt(source: UsageSource, input: u64) -> UsageRecord {
        UsageRecord {
            source,
            provenance: Provenance::default(),
            delta: UsageDelta::new().with(CounterKind::InputUncached, input),
        }
    }

    fn state(remaining: u64) -> VersionedSessionState {
        VersionedSessionState {
            revision: RegistryRevision::new("smith-budget-notice-v1"),
            sensitivity: SessionStateSensitivity::RedactionSafe,
            value: serde_json::json!({
                "schema_version": BUDGET_NOTICE_STATE_SCHEMA_VERSION,
                "remaining_tokens": remaining,
            }),
        }
    }

    fn view(state: Option<VersionedSessionState>) -> ContextView {
        ContextView {
            session: agent_runtime_core::ids::SessionId::new("s"),
            turn: agent_runtime_core::ids::TurnId::new("t"),
            history: std::sync::Arc::from([]),
            activation: agent_runtime::registry::Fingerprint::of_fields([b"a".as_slice()]),
            state,
        }
    }

    #[test]
    fn a_budget_the_threshold_cannot_fit_under_is_rejected() {
        assert!(BudgetNoticeComponent::new(0, 100).is_err());
        assert!(BudgetNoticeComponent::new(1_000, 1_000).is_err());
        assert!(BudgetNoticeComponent::new(1_000, 0).is_err());
        assert!(BudgetNoticeComponent::new(1_000, 100).is_ok());
    }

    #[test]
    fn remaining_reads_the_newest_provider_attempt_only() {
        let component = BudgetNoticeComponent::new(100_000, 12_000).expect("valid");
        let usage = [
            attempt(UsageSource::ProviderAttempt, 10_000),
            attempt(UsageSource::ProviderAttempt, 95_000),
            // Summary spend is separately attributed and is not context.
            attempt(UsageSource::SemanticSummary, 99_999),
        ];
        assert_eq!(component.remaining(&usage), Some(5_000));
    }

    #[tokio::test]
    async fn the_notice_appears_only_under_the_threshold() {
        let component = BudgetNoticeComponent::new(100_000, 12_000).expect("valid");

        let quiet = component
            .contribute(&view(Some(state(50_000))))
            .await
            .expect("a patch");
        assert!(quiet.fragments.is_empty());

        let near = component
            .contribute(&view(Some(state(4_000))))
            .await
            .expect("a patch");
        let [fragment] = near.fragments.as_slice() else {
            panic!("expected one notice fragment: {near:?}");
        };
        assert_eq!(
            fragment.position,
            ContextPosition::new(ContextLane::TailContext, 0)
        );
        assert!(
            !fragment.is_required(),
            "the notice must yield under budget"
        );
        match &fragment.content {
            FragmentContent::Text(body) => assert!(body.contains("4000"), "{body}"),
            other => panic!("expected text: {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_observation_contributes_nothing() {
        let component = BudgetNoticeComponent::new(100_000, 12_000).expect("valid");
        let patch = component.contribute(&view(None)).await.expect("a patch");
        assert!(patch.fragments.is_empty());
    }
}
