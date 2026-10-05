//! Session status, accounting, and the honesty rules from `DESIGN.md` §7.
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

use agent_runtime_core::goal::GoalProjection;
use agent_runtime_core::manifest::SegmentKind;
use agent_runtime_core::provider::ProviderAttemptPurpose;
use agent_runtime_core::usage::{CounterKind, UsageDelta, UsageRecord, UsageSource};
use smith_runtime::advisor::ADVISOR_USAGE_PURPOSE;
use smith_runtime::client::{EstimationConfidence, SmithEvent as EventEnvelope};

use crate::format::{compact_tokens, format_usd, plural};

use crate::cache::{CacheLifecycleSummary, CachePrice, CacheProjection, CacheTurnSummary};

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

/// Output flow for one active root turn, separate from session spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnUsage {
    /// Output tokens received so far, with their measurement provenance.
    pub output: TokenCount,
}

impl Default for TurnUsage {
    fn default() -> Self {
        Self {
            output: TokenCount::UNKNOWN,
        }
    }
}

impl TurnUsage {
    /// Adds root output, including failed attempts, without counting rollups
    /// or separately attributed cache and advisor work a second time.
    pub fn record(&mut self, record: &UsageRecord) {
        if !matches!(
            record.source,
            UsageSource::ProviderAttempt | UsageSource::ExternalAgent
        ) || record.provenance.purpose.as_deref() == Some(ADVISOR_USAGE_PURPOSE)
            || record
                .provenance
                .attempt_purpose
                .is_some_and(|purpose| purpose.is_synthetic_cache())
        {
            return;
        }
        let output = record.delta.get(CounterKind::Output);
        // UsageDelta is sparse: zero cannot distinguish absent output from a
        // reported zero, so it supplies no output-flow measurement.
        if output > 0 {
            self.output = TokenCount {
                value: self.output.value.saturating_add(output),
                confidence: if self.output.confidence == Confidence::Estimated {
                    Confidence::Estimated
                } else {
                    Confidence::Reported
                },
            };
        }
    }
}

/// The latest context plan the runtime actually enforced for a provider
/// request. It contains metrics only; raw context content has no field here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPlanStatus {
    /// Immutable context-plan fingerprint.
    pub fingerprint: String,
    /// Provider cache-plan fingerprint paired with this context.
    pub cache_fingerprint: String,
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

/// Borrowed fields from one canonical context-planning event.
#[derive(Debug, Clone, Copy)]
pub struct ContextPlanUpdate<'a> {
    /// Immutable context-plan fingerprint.
    pub fingerprint: &'a str,
    /// Provider cache-plan fingerprint paired with this context.
    pub cache_fingerprint: &'a str,
    /// Counted input tokens in the assembled request.
    pub input_tokens: u32,
    /// The enforced input ceiling after reserves and model limits.
    pub input_budget_tokens: u32,
    /// Output and reasoning tokens held out of the input budget.
    pub reserved_tokens: u32,
    /// Number of bounded plan segments.
    pub segment_count: u32,
    /// Token totals by canonical segment kind.
    pub totals: &'a BTreeMap<SegmentKind, u32>,
    /// Whether the plan used an authoritative tokenizer or a fallback
    /// estimator.
    pub confidence: EstimationConfidence,
}

impl ContextPlanStatus {
    /// Builds display state from a canonical planning event.
    pub fn from_update(update: ContextPlanUpdate<'_>) -> Self {
        Self {
            fingerprint: update.fingerprint.to_owned(),
            cache_fingerprint: update.cache_fingerprint.to_owned(),
            input_tokens: update.input_tokens,
            input_budget_tokens: update.input_budget_tokens,
            reserved_tokens: update.reserved_tokens,
            segment_count: update.segment_count,
            totals: update
                .totals
                .iter()
                .map(|(kind, tokens)| (kind.as_str().to_owned(), *tokens))
                .collect(),
            confidence: update.confidence,
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

/// Bounded provenance for the latest live ability lifecycle.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityStatus {
    /// Sealed registry snapshot fingerprint and total entry count.
    pub registry: Option<(String, u32)>,
    /// Policy-scoped view fingerprint and visible entry count.
    pub view: Option<(String, u32)>,
    /// Resolver revision and latest ranked candidate identities.
    pub retrieval: Option<(String, Vec<String>)>,
    /// Latest activation epoch and its ordered capability identities.
    pub activation: Option<(u32, Vec<String>)>,
    /// Number of context compactions observed in this session.
    pub compactions: u32,
    /// Total tokens reclaimed by observed compactions.
    pub reclaimed_tokens: u64,
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

/// Renders canonical terminal duration with honest sub-second precision.
pub fn render_terminal_elapsed(duration: Duration) -> String {
    if duration < Duration::from_secs(1) {
        let millis = duration.as_millis();
        if millis == 0 {
            "<1ms".to_owned()
        } else {
            format!("{millis}ms")
        }
    } else {
        render_elapsed(duration)
    }
}

/// Client-neutral session status and accounting projection.
#[derive(Debug, Clone)]

pub struct Status {
    /// Active main agent profile.
    pub agent: String,
    /// The installed coding agent executing turns, when the profile selected
    /// one. Present means the turn runs on a CLI rather than through Smith's
    /// own loop, which the header labels rather than leaving implicit.
    pub harness: Option<String>,
    /// The serving provider's name, once resolved.
    pub provider: Option<String>,
    /// The model in use.
    pub model: String,
    /// Active named context window for models that expose alternatives.
    pub context_window: Option<String>,
    /// Compact non-default reasoning override, when one is active.
    pub reasoning_hint: Option<String>,
    /// The project root, shown abbreviated.
    pub project: String,
    /// Cumulative provider-reported input for this session.
    pub context: TokenCount,
    /// Latest enforced request plan, when at least one turn was planned.
    pub context_plan: Option<ContextPlanStatus>,
    /// Latest registry/view/retrieval/activation lifecycle provenance.
    pub capabilities: CapabilityStatus,
    /// Cache tokens read, when the provider reports cache evidence.
    pub cache_read: Option<u64>,
    /// Canonical retry-safe cache evidence and latest completed-turn rollup.
    pub cache_projection: CacheProjection,
    /// Latest durability-aligned persistent-goal projection.
    pub goal: Option<GoalProjection>,
    /// Whether any provider usage has been reported this session.
    usage_reported: bool,
    /// Per-counter session totals, kept separately from the cumulative input
    /// figure the header shows so an exit report can name each counter.
    totals: BTreeMap<CounterKind, u64>,
    /// Closed root buckets retain the prices installed before their turns.
    bindings: Vec<BindingUsage>,
    /// Explicit host attribution; legacy rollup-only callers supply a price
    /// directly to SessionCost and keep their existing accessor behavior.
    active_binding: Option<BindingUsage>,
    /// Provider-reported cache-maintenance counters, excluded from ordinary
    /// root turns while retained in whole-session spend.
    synthetic_totals: BTreeMap<CounterKind, u64>,
    /// Synthetic counters keyed by Runtime's typed attempt purpose.
    synthetic_by_purpose: BTreeMap<ProviderAttemptPurpose, BTreeMap<CounterKind, u64>>,
    advisor_totals: BTreeMap<CounterKind, u64>,
    advisor_price: Option<PriceReference>,
    /// Turns that produced provider usage this session.
    turns: u32,
    /// The active provider/model's catalog price, resolved once by
    /// `crates/smith-cli` against the exact binding the runtime factory
    /// used, or `None` when the catalog carries no price entry for it.
    /// Never filled in from another model, provider, or a hard-coded
    /// default. See [`Self::set_price`] and [`Self::switch_model`].
    price: Option<PriceReference>,
}

impl Status {
    /// A status for a session that has not yet run a turn.
    pub fn new(model: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            harness: None,
            agent: "build".to_owned(),
            provider: None,
            model: model.into(),
            context_window: None,
            reasoning_hint: None,
            project: project.into(),
            context: TokenCount::UNKNOWN,
            context_plan: None,
            capabilities: CapabilityStatus::default(),
            cache_read: None,
            cache_projection: CacheProjection::default(),
            goal: None,
            usage_reported: false,
            totals: BTreeMap::new(),
            bindings: Vec::new(),
            active_binding: None,
            synthetic_totals: BTreeMap::new(),
            synthetic_by_purpose: BTreeMap::new(),
            advisor_totals: BTreeMap::new(),
            advisor_price: None,
            turns: 0,
            price: None,
        }
    }

    /// Sets the active agent profile shown at the point of action.
    pub fn set_agent(&mut self, agent: impl Into<String>) {
        self.agent = agent.into();
    }

    /// Sets the compact footer hint for a non-default reasoning selection.
    pub fn set_reasoning_hint(&mut self, hint: Option<String>) {
        self.reasoning_hint = hint;
    }

    /// Replaces the compact persistent-goal projection.
    pub fn set_goal(&mut self, goal: Option<GoalProjection>) {
        self.goal = goal;
    }

    /// Renders the compact, provenance-aware goal footer segment.
    pub fn render_goal_footer(&self) -> Option<String> {
        self.goal.as_ref().map(|goal| {
            let status = goal.status.as_str();
            let used = goal
                .usage
                .charged_tokens
                .map_or_else(|| "?".to_owned(), compact_tokens);
            let tokens = goal.token_budget.map_or_else(
                || format!("{used} tok"),
                |budget| format!("{used}/{} tok", compact_tokens(budget)),
            );
            format!("goal {status} · {tokens}")
        })
    }

    /// Counts admission of a user-started root turn so retries and tool calls
    /// cannot inflate the conversation count. Internal turns use no admission.
    pub fn record_user_turn(&mut self) {
        self.turns = self.turns.saturating_add(1);
    }

    /// Seeds the durable completed-turn identity used by session listings.
    /// Usage records carry provider attempts, so cannot reconstruct this count.
    pub fn restore_turn_count(&mut self, turns: u64) {
        self.turns = u32::try_from(turns).unwrap_or(u32::MAX);
    }

    /// Folds a provider-reported usage delta into the running totals.
    ///
    /// Input categories are disjoint in the runtime's accounting, so context is
    /// their sum; output and reasoning tokens are not context.
    pub fn record_usage(&mut self, delta: &UsageDelta) {
        // Every input category, cache writes included: a provider bills them
        // differently but each one occupied the window. Anthropic reports the
        // cacheable prefix as a cache write on the turn that establishes it,
        // so omitting that counter understates the session's real context.
        let input = delta.input_tokens();
        // `UsageDelta` cannot distinguish an omitted input counter from an
        // explicitly reported zero. An output-only record therefore provides
        // no evidence about context consumption; keep `?` instead of turning
        // an absent counter into a hard zero.
        if input == 0 {
            return;
        }
        self.usage_reported = true;
        self.context = TokenCount::reported(self.context.value.saturating_add(input));
        if let Some(binding) = &mut self.active_binding {
            binding.record(delta, true);
        }
        for kind in [
            CounterKind::InputUncached,
            CounterKind::InputCached,
            CounterKind::CacheWrite,
            CounterKind::Output,
            CounterKind::Reasoning,
        ] {
            let value = delta.get(kind);
            if value > 0 {
                *self.totals.entry(kind).or_insert(0) += value;
            }
        }
    }

    /// Routes one canonical Runtime usage record without allowing synthetic
    /// cache work to masquerade as a root/user turn.
    pub fn record_usage_record(&mut self, record: &UsageRecord) {
        if record.provenance.purpose.as_deref() == Some(ADVISOR_USAGE_PURPOSE) {
            self.usage_reported |= !record.delta.is_empty();
            for (kind, value) in record.delta.iter() {
                *self.advisor_totals.entry(kind).or_default() += value;
            }
            return;
        }
        if let Some(purpose) = record.provenance.attempt_purpose
            && purpose.is_synthetic_cache()
        {
            self.record_synthetic_usage(purpose, &record.delta);
            return;
        }
        self.record_usage(&record.delta);
        if record.delta.input_tokens() > 0
            && let Some(binding) = &mut self.active_binding
        {
            binding.reported &= matches!(
                record.source,
                UsageSource::ProviderAttempt | UsageSource::ExternalAgent
            );
        }
    }

    /// Reconciles advisor counters after a terminal event, including reported
    /// usage from an interruption that bypassed canonical commit hooks.
    pub fn reconcile_advisor_records(&mut self, records: &[UsageRecord]) {
        let mut usage = self.session_usage();
        usage.reconcile_advisor_records(records);
        self.advisor_totals = usage.advisor_totals;
        self.usage_reported = usage.reported;
    }

    /// Accounts provider usage under a typed synthetic purpose. This updates
    /// provider/session spend only: context and ordinary turn counts remain
    /// untouched. A separate per-binding copy preserves the rates active when
    /// the counters arrived without changing the existing synthetic rollups.
    pub fn record_synthetic_usage(&mut self, purpose: ProviderAttemptPurpose, delta: &UsageDelta) {
        if !purpose.is_synthetic_cache() || delta.is_empty() {
            return;
        }
        self.usage_reported = true;
        if purpose != ProviderAttemptPurpose::IdleCompaction
            && let Some(binding) = &mut self.active_binding
        {
            let totals = binding.synthetic_by_purpose.entry(purpose).or_default();
            for (kind, value) in delta.iter() {
                let total = totals.entry(kind).or_default();
                *total = total.saturating_add(value);
            }
        }
        let purpose_totals = self.synthetic_by_purpose.entry(purpose).or_default();
        for kind in [
            CounterKind::InputUncached,
            CounterKind::InputCached,
            CounterKind::CacheWrite,
            CounterKind::Output,
            CounterKind::Reasoning,
        ] {
            let value = delta.get(kind);
            if value > 0 {
                *self.synthetic_totals.entry(kind).or_insert(0) += value;
                *purpose_totals.entry(kind).or_insert(0) += value;
            }
        }
    }

    /// A bounded, content-free summary of what this session spent.
    ///
    /// Root-only: `Status` has no visibility into delegated children, so a
    /// caller that wants the whole session's usage — root plus delegated —
    /// goes through `App::session_usage` instead, which fills in the
    /// delegated fields this leaves at their empty default.
    pub fn session_usage(&self) -> SessionUsage {
        SessionUsage {
            bindings: self
                .bindings
                .iter()
                .chain(self.active_binding.iter())
                .filter(|binding| binding.has_usage())
                .cloned()
                .collect(),
            turns: self.turns,
            reported: self.usage_reported,
            totals: self.totals.clone(),
            advisor_totals: self.advisor_totals.clone(),
            advisor_price: self.advisor_price.clone(),
            synthetic_totals: self.synthetic_totals.clone(),
            synthetic_by_purpose: self.synthetic_by_purpose.clone(),
            compactions: self.capabilities.compactions,
            reclaimed_tokens: self.capabilities.reclaimed_tokens,
            delegated_totals: BTreeMap::new(),
            delegated_contributors: 0,
            cache_miss_count: self.cache_projection.session_miss_count(),
            cache_rebilled_tokens: self.cache_projection.session_rebilled_tokens(),
        }
    }

    /// Keeps live binding attribution across host rebuilds only when the
    /// durable root rollup agrees, avoiding duplicate or invented usage.
    pub fn retain_usage_bindings(&mut self, previous: &SessionUsage) {
        if !previous.bindings.is_empty() && previous.totals == self.totals {
            self.bindings = previous.bindings.clone();
            self.active_binding = Some(BindingUsage::new(
                self.provider.clone(),
                &self.model,
                self.price.clone(),
            ));
        }
    }

    /// Installs the active binding's reference before counters arrive, so
    /// closed buckets keep the rates they originally used.
    ///
    /// `Status` has no catalog access of its own: `crates/smith-cli` looks
    /// up the catalog entry using the exact binding the runtime factory
    /// resolved the model against and hands the result here once, so
    /// `/status` and the exit report both read this identical reference
    /// instead of each re-deriving it from the catalog. Pass `None` when the
    /// catalog carries no price entry for the active model — never a price
    /// substituted from another model, provider, or a hard-coded default.
    pub fn set_price(&mut self, price: Option<PriceReference>) {
        if let Some(binding) = &mut self.active_binding {
            if !binding.has_usage() {
                binding.price = price.clone();
            }
        } else if self.totals.is_empty()
            && self.synthetic_totals.is_empty()
            && let Some(price) = &price
        {
            self.active_binding = Some(BindingUsage::new(
                Some(price.provider.clone()),
                &price.model,
                Some(price.clone()),
            ));
        }
        self.price = price;
    }

    /// Sets the advisor model's own catalog reference for session spend.
    pub fn set_advisor_price(&mut self, price: Option<PriceReference>) {
        self.advisor_price = price;
    }

    /// The resolved price reference, when the catalog prices this session's
    /// active model.
    pub fn price(&self) -> Option<&PriceReference> {
        self.price.as_ref()
    }

    /// Records a cache observation.
    pub fn record_cache(&mut self, read_tokens: u64) {
        self.cache_read = Some(self.cache_read.unwrap_or(0).saturating_add(read_tokens));
    }

    /// Folds a canonical event into the cache projection. Live events and
    /// journal replay both use this method, so a retry or duplicate replay
    /// cannot inflate derived diagnostics.
    pub fn record_cache_event(&mut self, envelope: &EventEnvelope) {
        let identity_event = matches!(
            &envelope.payload,
            smith_runtime::client::SmithEventKind::ModelProfileResolved { .. }
        );
        self.cache_projection.apply(envelope);
        if identity_event
            && self
                .cache_projection
                .latest_completed()
                .is_some_and(|summary| {
                    summary.state == crate::cache::CacheVisibilityState::Suspended
                })
        {
            self.cache_read = None;
            return;
        }
        if let Some(read) = self
            .cache_projection
            .session_observed_read()
            .or_else(|| self.cache_projection.legacy_read())
        {
            self.cache_read = Some(read);
        }
    }

    /// Replays canonical cache events without touching conversation state.
    pub fn replay_cache_events<I>(&mut self, events: I)
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        for event in events {
            self.record_cache_event(&event);
        }
    }

    /// Latest completed root-turn cache summary, with derived cost when the
    /// active binding has a compatible catalog price.
    pub fn cache_summary(&self) -> Option<CacheTurnSummary> {
        let summary = self.cache_projection.latest_completed()?;
        let price = self
            .price
            .as_ref()
            .map(|price| CachePrice::from(&price.table));
        Some(match price {
            Some(price) => self.cache_projection.with_price(summary, price),
            None => summary.clone(),
        })
    }

    /// Latest canonical cache-operation and provider-evidence lifecycle.
    pub fn cache_lifecycle(&self) -> &CacheLifecycleSummary {
        self.cache_projection.lifecycle()
    }

    /// The latest completed turn's provider-reported cache-read percentage.
    /// Explicit zero is `0%`; absent evidence is `?`.
    pub fn render_cache_hit_rate(&self) -> String {
        self.cache_summary()
            .map_or_else(|| "?".to_owned(), |summary| summary.render_ch())
    }

    /// A significant latest-turn cache notice, if one is available.
    pub fn cache_notice(&self) -> Option<String> {
        let summary = self.cache_summary()?;
        summary.significant().then(|| summary.render_notice())
    }

    /// Records the latest canonical context plan without retaining any
    /// segment content.
    pub fn record_context_plan(&mut self, update: ContextPlanUpdate<'_>) {
        self.context_plan = Some(ContextPlanStatus::from_update(update));
    }

    /// Records one sealed ability registry snapshot.
    pub fn record_registry(&mut self, fingerprint: impl Into<String>, entries: u32) {
        self.capabilities.registry = Some((fingerprint.into(), entries));
    }

    /// Records the current policy-scoped ability view.
    pub fn record_scoped_view(&mut self, fingerprint: impl Into<String>, visible: u32) {
        self.capabilities.view = Some((fingerprint.into(), visible));
    }

    /// Records bounded retrieval identities without retaining query text.
    pub fn record_retrieval(
        &mut self,
        resolver_revision: impl Into<String>,
        candidates: Vec<String>,
    ) {
        self.capabilities.retrieval = Some((resolver_revision.into(), candidates));
    }

    /// Records the latest frozen activation epoch.
    pub fn record_activation(&mut self, epoch: u32, capabilities: Vec<String>) {
        self.capabilities.activation = Some((epoch, capabilities));
    }

    /// Records context compaction totals.
    pub fn record_compaction(&mut self, reclaimed_tokens: u32) {
        self.capabilities.compactions = self.capabilities.compactions.saturating_add(1);
        self.capabilities.reclaimed_tokens = self
            .capabilities
            .reclaimed_tokens
            .saturating_add(u64::from(reclaimed_tokens));
    }

    /// Switches provider or model, resetting everything the new provider has
    /// not yet told us.
    ///
    /// The old provider's cache does not transfer and its token accounting does
    /// not describe the new one, so context drops back to estimated and cache
    /// evidence is cleared rather than carried over. The old counter bucket
    /// closes with its price intact. The active price is cleared because it
    /// described the old binding, this
    /// method has no catalog access to re-resolve one for the new binding,
    /// and a stale price would misprice the session exactly as badly as a
    /// stale cache figure would misreport it. The caller that does have
    /// catalog access (`crates/smith-cli`, at startup) calls
    /// [`Self::set_price`] right after switching.
    pub fn switch_model(&mut self, provider: Option<String>, model: impl Into<String>) {
        if let Some(binding) = self.active_binding.take()
            && binding.has_usage()
        {
            self.bindings.push(binding);
        }
        self.provider = provider;
        self.model = model.into();
        self.active_binding = Some(BindingUsage::new(self.provider.clone(), &self.model, None));
        self.context_window = None;
        self.cache_read = None;
        self.cache_projection.suspend();
        self.context_plan = None;
        self.usage_reported = false;
        self.price = None;
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
        self.context_plan.as_ref().map_or_else(
            || "unknown ctx".to_owned(),
            ContextPlanStatus::render_footer,
        )
    }
}

#[cfg(test)]
mod tests;

mod accounting;

pub use accounting::{
    BindingUsage, CostLabel, PriceReference, PriceTable, SessionCost, SessionUsage, counter_label,
};
