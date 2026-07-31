//! Header status, and the honesty rules from `DESIGN.md` §7.
//!
//! The whole point of this module is that a number Smith did not receive from a
//! provider must never look like one it did. Three renderings are distinct and
//! stay distinct:
//!
//! | Rendering | Meaning |
//! | --- | --- |
//! | `12.4k` | The provider reported it |
//! | `~12.4k` | Smith estimated it |
//! | `?` | Nobody knows — and it is **not** `0` |
//!
//! A zero that means "no tokens were used" and a blank that means "the provider
//! never told us" are different facts. Collapsing them is how a status line
//! starts lying.

use std::collections::BTreeMap;

use std::time::Duration;

use agent_runtime_core::event::EstimationConfidence;
use agent_runtime_core::manifest::SegmentKind;
use agent_runtime_core::usage::{CounterKind, UsageDelta};

/// How a displayed quantity was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// The provider reported it.
    Reported,
    /// Smith derived or estimated it.
    Estimated,
    /// No value is available.
    Unknown,
}

/// A token count with its provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenCount {
    /// The value, meaningless when `confidence` is [`Confidence::Unknown`].
    pub value: u64,
    /// How the value was obtained.
    pub confidence: Confidence,
}

impl TokenCount {
    /// A count nobody has reported.
    pub const UNKNOWN: Self = Self {
        value: 0,
        confidence: Confidence::Unknown,
    };

    /// A provider-reported count.
    pub fn reported(value: u64) -> Self {
        Self {
            value,
            confidence: Confidence::Reported,
        }
    }

    /// An estimated count.
    pub fn estimated(value: u64) -> Self {
        Self {
            value,
            confidence: Confidence::Estimated,
        }
    }

    /// Renders the count with its provenance marker.
    pub fn render(self) -> String {
        match self.confidence {
            Confidence::Unknown => "?".to_owned(),
            Confidence::Reported => compact_tokens(self.value),
            Confidence::Estimated => format!("~{}", compact_tokens(self.value)),
        }
    }
}

/// Formats a token count compactly: `847`, `12.4k`, `1.2M`.
fn compact_tokens(value: u64) -> String {
    match value {
        0..1_000 => value.to_string(),
        1_000..1_000_000 => {
            let tenths = value / 100;
            // Drop a trailing `.0` so `12000` reads `12k`, not `12.0k`.
            if tenths.is_multiple_of(10) {
                format!("{}k", tenths / 10)
            } else {
                format!("{}.{}k", tenths / 10, tenths % 10)
            }
        }
        _ => {
            let tenths = value / 100_000;
            if tenths.is_multiple_of(10) {
                format!("{}M", tenths / 10)
            } else {
                format!("{}.{}M", tenths / 10, tenths % 10)
            }
        }
    }
}

/// The latest context plan the runtime actually enforced for a provider
/// request. It contains metrics only; raw context content has no field here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPlanStatus {
    /// Counted input tokens in the assembled request.
    pub input_tokens: u32,
    /// The enforced input ceiling after reserves and model limits.
    pub input_budget_tokens: u32,
    /// Output and reasoning tokens held out of the input budget.
    pub reserved_tokens: u32,
    /// Number of bounded plan segments.
    pub segment_count: u32,
    /// Token totals by stable segment-kind label.
    pub totals: BTreeMap<String, u32>,
    /// Whether the plan used an authoritative tokenizer or a fallback
    /// estimator.
    pub confidence: EstimationConfidence,
}

impl ContextPlanStatus {
    /// Builds display state from a canonical planning event.
    pub fn new(
        input_tokens: u32,
        input_budget_tokens: u32,
        reserved_tokens: u32,
        segment_count: u32,
        totals: &BTreeMap<SegmentKind, u32>,
        confidence: EstimationConfidence,
    ) -> Self {
        Self {
            input_tokens,
            input_budget_tokens,
            reserved_tokens,
            segment_count,
            totals: totals
                .iter()
                .map(|(kind, tokens)| (kind.as_str().to_owned(), *tokens))
                .collect(),
            confidence,
        }
    }

    /// Input-budget tokens that remain after the latest plan.
    pub fn remaining_tokens(&self) -> u32 {
        self.input_budget_tokens.saturating_sub(self.input_tokens)
    }

    /// Whole percent of the enforced input budget still available.
    pub fn percent_left(&self) -> u32 {
        if self.input_budget_tokens == 0 {
            return 0;
        }
        self.remaining_tokens()
            .saturating_mul(100)
            .checked_div(self.input_budget_tokens)
            .unwrap_or(0)
    }

    /// Renders latest-plan input with exact/estimated provenance.
    pub fn render_input(&self) -> String {
        match self.confidence {
            EstimationConfidence::Exact => {
                TokenCount::reported(u64::from(self.input_tokens)).render()
            }
            EstimationConfidence::Estimated => {
                TokenCount::estimated(u64::from(self.input_tokens)).render()
            }
        }
    }

    /// Stable lowercase confidence label.
    pub fn confidence_label(&self) -> &'static str {
        match self.confidence {
            EstimationConfidence::Exact => "exact tokenizer",
            EstimationConfidence::Estimated => "estimated",
        }
    }

    /// Compact footer summary based on active-plan state, not cumulative
    /// provider usage.
    pub fn render_footer(&self) -> String {
        let prefix = if self.confidence == EstimationConfidence::Estimated {
            "~"
        } else {
            ""
        };
        format!("{prefix}{}% ctx", self.percent_left())
    }
}

/// What the agent is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Activity {
    /// Waiting for input.
    #[default]
    Idle,
    /// A turn is running.
    Working,
    /// A turn is being cancelled.
    Interrupting,
    /// The session has shut down.
    Ended,
}

/// Renders a monotonic turn duration without noisy sub-second precision.
pub fn render_elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

impl Activity {
    /// The word shown beside the spinner. Paired with the glyph, never
    /// replaced by it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "ready",
            Self::Working => "working",
            Self::Interrupting => "interrupting",
            Self::Ended => "ended",
        }
    }
}

/// The header's model of session status.
#[derive(Debug, Clone)]
pub struct Status {
    /// The serving provider's name, once resolved.
    pub provider: Option<String>,
    /// The model in use.
    pub model: String,
    /// The project root, shown abbreviated.
    pub project: String,
    /// Cumulative provider-reported input for this session.
    pub context: TokenCount,
    /// Latest enforced request plan, when at least one turn was planned.
    pub context_plan: Option<ContextPlanStatus>,
    /// Cache tokens read, when the provider reports cache evidence.
    pub cache_read: Option<u64>,
    /// What the agent is doing.
    pub activity: Activity,
    /// Whether any provider usage has been reported this session.
    usage_reported: bool,
}

impl Status {
    /// A status for a session that has not yet run a turn.
    pub fn new(model: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            provider: None,
            model: model.into(),
            project: project.into(),
            context: TokenCount::UNKNOWN,
            context_plan: None,
            cache_read: None,
            activity: Activity::Idle,
            usage_reported: false,
        }
    }

    /// Folds a provider-reported usage delta into the running totals.
    ///
    /// Input categories are disjoint in the runtime's accounting, so context is
    /// their sum; output and reasoning tokens are not context.
    pub fn record_usage(&mut self, delta: &UsageDelta) {
        let input = delta
            .get(CounterKind::InputUncached)
            .saturating_add(delta.get(CounterKind::InputCached));
        // `UsageDelta` cannot distinguish an omitted input counter from an
        // explicitly reported zero. An output-only record therefore provides
        // no evidence about context consumption; keep `?` instead of turning
        // an absent counter into a hard zero.
        if input == 0 {
            return;
        }
        self.usage_reported = true;
        self.context = TokenCount::reported(self.context.value.saturating_add(input));
    }

    /// Records a cache observation.
    pub fn record_cache(&mut self, read_tokens: u64) {
        self.cache_read = Some(self.cache_read.unwrap_or(0).saturating_add(read_tokens));
    }

    /// Records the latest canonical context plan without retaining any
    /// segment content.
    pub fn record_context_plan(
        &mut self,
        input_tokens: u32,
        input_budget_tokens: u32,
        reserved_tokens: u32,
        segment_count: u32,
        totals: &BTreeMap<SegmentKind, u32>,
        confidence: EstimationConfidence,
    ) {
        self.context_plan = Some(ContextPlanStatus::new(
            input_tokens,
            input_budget_tokens,
            reserved_tokens,
            segment_count,
            totals,
            confidence,
        ));
    }

    /// Switches provider or model, resetting everything the new provider has
    /// not yet told us.
    ///
    /// The old provider's cache does not transfer and its token accounting does
    /// not describe the new one, so context drops back to estimated and cache
    /// evidence is cleared rather than carried over.
    pub fn switch_model(&mut self, provider: Option<String>, model: impl Into<String>) {
        self.provider = provider;
        self.model = model.into();
        self.cache_read = None;
        self.context_plan = None;
        self.usage_reported = false;
        if self.context.confidence == Confidence::Reported {
            self.context.confidence = Confidence::Estimated;
        }
    }

    /// Whether any usage has been reported since the last model change.
    pub fn has_reported_usage(&self) -> bool {
        self.usage_reported
    }

    /// Renders the cache segment, distinguishing "no evidence" from "zero".
    pub fn render_cache(&self) -> String {
        match self.cache_read {
            Some(tokens) => compact_tokens(tokens),
            None => "?".to_owned(),
        }
    }

    /// Footer context derived from the latest enforced plan.
    pub fn render_context_footer(&self) -> String {
        self.context_plan
            .as_ref()
            .map_or_else(|| "? ctx".to_owned(), ContextPlanStatus::render_footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_time_stays_compact_from_seconds_through_hours() {
        assert_eq!(render_elapsed(Duration::ZERO), "0s");
        assert_eq!(render_elapsed(Duration::from_secs(59)), "59s");
        assert_eq!(render_elapsed(Duration::from_secs(65)), "1m 05s");
        assert_eq!(render_elapsed(Duration::from_secs(3_725)), "1h 02m 05s");
    }

    #[test]
    fn token_counts_are_compact_and_lose_a_pointless_decimal() {
        assert_eq!(compact_tokens(0), "0");
        assert_eq!(compact_tokens(847), "847");
        assert_eq!(compact_tokens(12_400), "12.4k");
        assert_eq!(compact_tokens(12_000), "12k");
        assert_eq!(compact_tokens(1_250_000), "1.2M");
        assert_eq!(compact_tokens(2_000_000), "2M");
    }

    #[test]
    fn provenance_is_visible_in_the_rendering() {
        assert_eq!(TokenCount::reported(12_400).render(), "12.4k");
        assert_eq!(TokenCount::estimated(12_400).render(), "~12.4k");
        assert_eq!(TokenCount::UNKNOWN.render(), "?");
    }

    #[test]
    fn an_unknown_count_never_renders_as_zero() {
        // The distinction this test protects: "no tokens were used" and "the
        // provider never told us" must not look the same.
        assert_eq!(TokenCount::UNKNOWN.render(), "?");
        assert_eq!(TokenCount::reported(0).render(), "0");
        assert_ne!(
            TokenCount::UNKNOWN.render(),
            TokenCount::reported(0).render()
        );
    }

    #[test]
    fn reported_usage_accumulates_disjoint_input_categories() {
        let mut status = Status::new("gpt-5.3", "~/work/api");
        assert_eq!(status.context.render(), "?");

        status.record_usage(
            &UsageDelta::new()
                .with(CounterKind::InputUncached, 500)
                .with(CounterKind::InputCached, 8_000)
                .with(CounterKind::Output, 300),
        );
        // Input categories sum; output is not context.
        assert_eq!(status.context.render(), "8.5k");
        assert!(status.has_reported_usage());

        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 1_500));
        assert_eq!(status.context.render(), "10k");
    }

    #[test]
    fn a_model_switch_downgrades_reported_context_to_estimated() {
        let mut status = Status::new("gpt-5.3", "~/work/api");
        status.record_usage(&UsageDelta::new().with(CounterKind::InputUncached, 9_000));
        status.record_cache(8_000);
        assert_eq!(status.context.render(), "9k");
        assert_eq!(status.render_cache(), "8k");

        status.switch_model(Some("anthropic".into()), "claude-opus-5");
        assert_eq!(
            status.context.render(),
            "~9k",
            "the old provider's count no longer describes the new one"
        );
        assert_eq!(
            status.render_cache(),
            "?",
            "the previous provider's cache does not transfer"
        );
        assert!(!status.has_reported_usage());
    }

    #[test]
    fn cache_evidence_is_distinct_from_a_zero_hit() {
        let mut status = Status::new("gpt-5.3", "~/work/api");
        assert_eq!(status.render_cache(), "?");
        status.record_cache(0);
        assert_eq!(status.render_cache(), "0");
    }

    #[test]
    fn output_only_usage_does_not_turn_unknown_input_into_reported_zero() {
        let mut status = Status::new("gpt-5.3", "~/work/api");
        status.record_usage(&UsageDelta::new().with(CounterKind::Output, 300));

        assert_eq!(status.context, TokenCount::UNKNOWN);
        assert!(!status.has_reported_usage());
    }

    #[test]
    fn the_latest_context_plan_reports_capacity_remaining_and_provenance() {
        let totals = BTreeMap::from([
            (SegmentKind::new("history"), 2_000),
            (SegmentKind::new("tool_schema"), 500),
        ]);
        let mut status = Status::new("gpt-5.3", "~/work/api");
        status.record_context_plan(
            2_500,
            10_000,
            2_000,
            2,
            &totals,
            EstimationConfidence::Exact,
        );

        let plan = status.context_plan.as_ref().expect("a plan");
        assert_eq!(plan.remaining_tokens(), 7_500);
        assert_eq!(plan.percent_left(), 75);
        assert_eq!(plan.render_input(), "2.5k");
        assert_eq!(plan.render_footer(), "75% ctx");
        assert_eq!(plan.totals["history"], 2_000);

        status.record_context_plan(
            2_500,
            10_000,
            2_000,
            2,
            &totals,
            EstimationConfidence::Estimated,
        );
        let estimated = status.context_plan.as_ref().expect("estimated plan");
        assert_eq!(estimated.render_input(), "~2.5k");
        assert_eq!(estimated.render_footer(), "~75% ctx");
    }

    #[test]
    fn switching_models_clears_a_plan_that_no_longer_applies() {
        let mut status = Status::new("gpt-5.3", "~/work/api");
        status.record_context_plan(
            100,
            1_000,
            100,
            1,
            &BTreeMap::from([(SegmentKind::new("history"), 100)]),
            EstimationConfidence::Exact,
        );
        assert_eq!(status.render_context_footer(), "90% ctx");

        status.switch_model(Some("anthropic".into()), "claude-opus-5");
        assert!(status.context_plan.is_none());
        assert_eq!(status.render_context_footer(), "? ctx");
    }
}
