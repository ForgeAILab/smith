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
use agent_runtime_core::goal::GoalProjection;
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

/// What one session spent, with no conversation content in it.
///
/// The counters are the provider's own disjoint categories rather than a single
/// blended total, because they price differently and a cache read is the number
/// worth watching: it is the direct evidence that the stable prefix ordering is
/// doing its job.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionUsage {
    /// Turns that produced provider usage.
    pub turns: u32,
    /// Whether any counter came from the provider rather than an estimate.
    pub reported: bool,
    /// Per-counter totals, omitting counters the provider never reported.
    pub totals: BTreeMap<CounterKind, u64>,
    /// Context compactions observed.
    pub compactions: u32,
    /// Tokens those compactions reclaimed.
    pub reclaimed_tokens: u64,
    /// Per-counter usage delegated children reported on their own streams
    /// this process observed. Kept separate from `totals` rather than
    /// blended into it, per `usage-accounting`'s "Delegated usage is
    /// accounted separately" — the approval boundary explicitly forbids
    /// blending child counters into the root counters so the two cannot be
    /// told apart.
    pub delegated_totals: BTreeMap<CounterKind, u64>,
    /// Distinct children that reported any delegated usage.
    pub delegated_contributors: u32,
}

impl SessionUsage {
    /// Whether anything at all was observed, including a delegated-only
    /// session that never accumulated any root usage of its own.
    pub fn is_empty(&self) -> bool {
        self.totals.is_empty() && self.turns == 0 && self.delegated_totals.is_empty()
    }

    /// The root session's own counter total.
    ///
    /// Deliberately root-only and unchanged in meaning: every existing
    /// caller of this method expects the session's own spend, not a figure
    /// blended with delegated usage. [`Self::merged_total_tokens`] is the
    /// explicit combined figure for callers that want the sum this
    /// method's own doc used to imply before delegation existed.
    pub fn total_tokens(&self) -> u64 {
        self.totals.values().copied().sum()
    }

    /// Every counter's total across both root and delegated usage.
    pub fn merged_total_tokens(&self) -> u64 {
        self.total_tokens() + self.delegated_totals.values().copied().sum::<u64>()
    }

    /// A human-facing summary, or `None` when nothing was observed.
    ///
    /// An unreported session is marked as estimated rather than shown as a
    /// confident zero: "0 tokens" and "the provider told us nothing" are
    /// different facts, and only one of them is a bill.
    ///
    /// When nothing was delegated this is exactly the root's own one-line
    /// summary, unchanged from before delegated accounting existed. When
    /// something was, a merged total line leads, followed by indented
    /// `root` and `agents` sub-lines that break it down — the `agents` line
    /// names how many children contributed.
    ///
    /// The merged line carries counters only, never a turn count. A child's
    /// turns belong to the delegation coordinator and never enter this
    /// projection, so the only turn figure available here is the root's —
    /// and printing merged tokens beside the root's turn count would read as
    /// a claim that those turns spent those tokens. Compactions stay on the
    /// root line for the same reason: they are a root context event, and
    /// repeating them against a merged figure would double-attribute them.
    pub fn render(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mark = if self.reported { "" } else { "~" };
        let root_parts = render_counter_parts(&self.totals, mark);
        let root_line = format_usage_line(
            self.turns,
            &root_parts,
            self.reported,
            self.compactions,
            self.reclaimed_tokens,
        );
        if self.delegated_totals.is_empty() {
            return Some(root_line);
        }

        let merged = merge_counter_totals(&self.totals, &self.delegated_totals);
        let mut merged_line = format!(
            "total · {}",
            render_counter_parts(&merged, mark).join(" · ")
        );
        if !self.reported {
            merged_line.push_str(" · estimated");
        }
        let agent_parts = render_counter_parts(&self.delegated_totals, mark);
        Some(format!(
            "{merged_line}\n  root: {root_line}\n  agents: {} agent(s) · {}",
            self.delegated_contributors,
            agent_parts.join(" · "),
        ))
    }
}

/// Renders one counter/value pair per entry, e.g. `input ~12.4k`.
fn render_counter_parts(totals: &BTreeMap<CounterKind, u64>, mark: &str) -> Vec<String> {
    totals
        .iter()
        .map(|(kind, value)| format!("{} {mark}{}", counter_label(*kind), compact_tokens(*value)))
        .collect()
}

/// Sums two per-counter total maps without mutating either input.
fn merge_counter_totals(
    root: &BTreeMap<CounterKind, u64>,
    delegated: &BTreeMap<CounterKind, u64>,
) -> BTreeMap<CounterKind, u64> {
    let mut merged = root.clone();
    for (kind, value) in delegated {
        *merged.entry(*kind).or_insert(0) += value;
    }
    merged
}

/// The shared `N turn(s) · … [· estimated] [· N compaction(s) …]` shape
/// both the root line and the merged total line render.
fn format_usage_line(
    turns: u32,
    parts: &[String],
    reported: bool,
    compactions: u32,
    reclaimed_tokens: u64,
) -> String {
    let mut line = if parts.is_empty() {
        format!("{turns} turn(s)")
    } else {
        format!("{turns} turn(s) · {}", parts.join(" · "))
    };
    if !reported {
        line.push_str(" · estimated");
    }
    if compactions > 0 {
        line.push_str(&format!(
            " · {compactions} compaction(s) reclaiming {}",
            compact_tokens(reclaimed_tokens)
        ));
    }
    line
}

/// A short label for one provider counter.
pub fn counter_label(kind: CounterKind) -> &'static str {
    match kind {
        CounterKind::InputUncached => "input",
        CounterKind::InputCached => "cached",
        CounterKind::CacheWrite => "cache-write",
        CounterKind::Output => "output",
        CounterKind::Reasoning => "reasoning",
    }
}

/// One counter's price, in micro-USD (1e-6 USD) per million tokens.
///
/// Mirrors `smith_config::catalog::CatalogModelCost` in shape but is
/// deliberately its own type: `smith-tui` has no dependency on
/// `smith-config`, and pricing here is a presentation concern applied to a
/// value `smith-cli` — which depends on both crates — already resolved
/// against the catalog snapshot using the exact binding the runtime factory
/// itself resolves models against (`RuntimePolicy`'s
/// `provider_kind`/`endpoint`/`model`; see
/// `crates/smith-runtime/src/factory.rs`'s `prepare_factory_inputs`).
/// Pricing any other entry would price the wrong model, which is exactly
/// what `usage-accounting`'s "Labelled cost calculation" forbids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PriceTable {
    /// Micro-USD per million uncached input tokens.
    pub input: Option<u64>,
    /// Micro-USD per million output tokens.
    pub output: Option<u64>,
    /// Micro-USD per million cache-read tokens.
    pub cache_read: Option<u64>,
    /// Micro-USD per million cache-write tokens.
    pub cache_write: Option<u64>,
}

impl PriceTable {
    /// The micro-USD-per-million price for one counter, absent when the
    /// catalog never published it.
    ///
    /// `CounterKind::Reasoning` always resolves to `None`. Reasoning tokens
    /// are billed separately from output tokens, not folded into them —
    /// `agent_runtime_core::usage::CounterKind::Reasoning`'s own doc says so
    /// — and Models.dev, the catalog's only source, publishes no distinct
    /// reasoning price. Charging reasoning tokens at the output rate would
    /// present a price the source never published as if it had: exactly
    /// what `usage-accounting`'s "Labelled cost calculation" forbids
    /// ("Smith SHALL calculate cost only from a versioned price reference
    /// and compatible usage counters"). A nonzero reasoning counter is
    /// therefore unpriceable everywhere in this table, which downgrades a
    /// session's cost label to estimated rather than contributing silently
    /// as zero — see [`SessionCost::compute`].
    fn price_for(self, kind: CounterKind) -> Option<u64> {
        match kind {
            CounterKind::InputUncached => self.input,
            CounterKind::InputCached => self.cache_read,
            CounterKind::CacheWrite => self.cache_write,
            CounterKind::Output => self.output,
            CounterKind::Reasoning => None,
        }
    }
}

/// The catalog price one session is billed against, and who it names.
///
/// Resolved once by `crates/smith-cli` at startup (and on a provider/model
/// change) and stored on [`Status`], so `/status` and the exit report price
/// from the identical reference instead of each re-deriving it — the same
/// discipline [`Status::switch_model`] already applies to cache evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceReference {
    /// The serving provider's name, so a rendered cost can say where the
    /// price came from.
    pub provider: String,
    /// The model identity the price describes.
    pub model: String,
    /// Per-counter micro-USD-per-million prices.
    pub table: PriceTable,
}

/// Whether a computed session cost is trustworthy as a bill or only a useful
/// approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostLabel {
    /// Every contributing counter was provider-reported and priced.
    Exact,
    /// At least one contributing counter was unreported, or the catalog
    /// published no price for it.
    Estimated,
}

impl CostLabel {
    /// Stable lowercase label, matching `DESIGN.md` §7's `$0.031`/`~$0.031`
    /// convention in words.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Estimated => "estimated",
        }
    }
}

/// A session's computed price: an honest amount, never a guess presented as
/// a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCost {
    /// The total in micro-USD (1e-6 USD), summed from every counter this
    /// session accumulated that the price reference actually prices.
    pub micro_usd: u128,
    /// Whether every contributing counter was both provider-reported and
    /// priced.
    pub label: CostLabel,
}

impl SessionCost {
    /// Prices `usage` — root and delegated totals alike — against `price`,
    /// using the identical per-counter reference for both.
    ///
    /// Delegated tokens are priced by the root's own reference even though a
    /// child may have run a different model, per `usage-accounting`'s
    /// "Delegated counters keep their categories": "the delegated totals are
    /// priced by the same per-counter reference the root totals are."
    /// Pricing each child by its own model would need a price reference per
    /// child rather than the one binding this session resolved, which is a
    /// larger feature this task does not add.
    ///
    /// Uses a `u128` intermediate for the multiply so a long session's token
    /// counts cannot overflow the arithmetic before the division back down
    /// to micro-USD.
    pub fn compute(usage: &SessionUsage, price: &PriceReference) -> Self {
        let mut micro_usd: u128 = 0;
        let mut all_priced = true;
        for (kind, tokens) in usage.totals.iter().chain(usage.delegated_totals.iter()) {
            if *tokens == 0 {
                continue;
            }
            match price.table.price_for(*kind) {
                Some(rate) => {
                    micro_usd += u128::from(*tokens) * u128::from(rate) / 1_000_000;
                }
                None => all_priced = false,
            }
        }
        // `usage.reported` is the provider-reported signal for the whole
        // session (`SessionUsage`'s own doc: "Whether any counter came from
        // the provider rather than an estimate"); an unpriced contributing
        // counter downgrades the label independently, per
        // `usage-accounting`'s "An estimated counter downgrades the label".
        let label = if usage.reported && all_priced {
            CostLabel::Exact
        } else {
            CostLabel::Estimated
        };
        Self { micro_usd, label }
    }

    /// Renders the amount with its honesty glyph: `$0.031` exact, `~$0.031`
    /// estimated (`DESIGN.md` §7).
    pub fn render(&self) -> String {
        let amount = format_usd(self.micro_usd);
        match self.label {
            CostLabel::Exact => amount,
            CostLabel::Estimated => format!("~{amount}"),
        }
    }
}

/// Formats a micro-USD amount at Smith's established cost precision: three
/// decimal places (`$0.031`, `DESIGN.md` §7) for anything at or above a
/// tenth of a cent. Below that, three decimals would round every real,
/// nonzero spend down to the same `$0.000` a literally free session
/// renders — the dollar-figure version of the zero/unknown collapse this
/// module exists to prevent — so that range widens to full micro-USD
/// precision instead, keeping a genuine sub-cent spend visibly distinct from
/// nothing at all.
fn format_usd(micro_usd: u128) -> String {
    let dollars = micro_usd / 1_000_000;
    let thousandths = (micro_usd / 1_000) % 1_000;
    if micro_usd > 0 && dollars == 0 && thousandths == 0 {
        return format!("$0.{:06}", micro_usd % 1_000_000);
    }
    format!("${dollars}.{thousandths:03}")
}

/// The active pool account, as the footer shows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AccountStatus {
    /// The credential reference, never its value.
    pub label: String,
    /// Server-reported consumption, absent when nothing measured it.
    pub used_percent: Option<f64>,
}

/// The header's model of session status.
#[derive(Debug, Clone)]

pub struct Status {
    /// Active main agent profile.
    pub agent: String,
    /// The serving provider's name, once resolved.
    pub provider: Option<String>,
    /// The model in use.
    pub model: String,
    /// Compact non-default reasoning override, when one is active.
    pub reasoning_hint: Option<String>,
    /// The credential-pool account serving attempts, when the provider
    /// declares a pool. Absent for a single-credential provider, which has no
    /// account to disambiguate.
    pub account: Option<AccountStatus>,
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
    /// What the agent is doing.
    pub activity: Activity,
    /// Latest durability-aligned persistent-goal projection.
    pub goal: Option<GoalProjection>,
    /// Whether any provider usage has been reported this session.
    usage_reported: bool,
    /// Per-counter session totals, kept separately from the cumulative input
    /// figure the header shows so an exit report can name each counter.
    totals: BTreeMap<CounterKind, u64>,
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
    /// The account segment for the footer, when the provider declares a pool.
    ///
    /// Deliberately its own segment rather than part of the context or token
    /// counters: a rate-limit window is a server-reported percentage of an
    /// account's plan, and the counters are Smith's disjoint token
    /// measurement of one session. Adjacent, never merged.
    pub fn render_account_footer(&self) -> Option<String> {
        let account = self.account.as_ref()?;
        Some(match account.used_percent {
            Some(percent) => format!("{} {}%", account.label, percent.round() as i64),
            // Unknown stays unknown: the account is named, its consumption is
            // not guessed at.
            None => account.label.clone(),
        })
    }

    /// A status for a session that has not yet run a turn.
    pub fn new(model: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            agent: "build".to_owned(),
            provider: None,
            model: model.into(),
            reasoning_hint: None,
            account: None,
            project: project.into(),
            context: TokenCount::UNKNOWN,
            context_plan: None,
            capabilities: CapabilityStatus::default(),
            cache_read: None,
            activity: Activity::Idle,
            goal: None,
            usage_reported: false,
            totals: BTreeMap::new(),
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
        self.turns = self.turns.saturating_add(1);
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

    /// A bounded, content-free summary of what this session spent.
    ///
    /// Root-only: `Status` has no visibility into delegated children, so a
    /// caller that wants the whole session's usage — root plus delegated —
    /// goes through `App::session_usage` instead, which fills in the
    /// delegated fields this leaves at their empty default.
    pub fn session_usage(&self) -> SessionUsage {
        SessionUsage {
            turns: self.turns,
            reported: self.usage_reported,
            totals: self.totals.clone(),
            compactions: self.capabilities.compactions,
            reclaimed_tokens: self.capabilities.reclaimed_tokens,
            delegated_totals: BTreeMap::new(),
            delegated_contributors: 0,
        }
    }

    /// Resolves the price this session bills against.
    ///
    /// `Status` has no catalog access of its own: `crates/smith-cli` looks
    /// up the catalog entry using the exact binding the runtime factory
    /// resolved the model against and hands the result here once, so
    /// `/status` and the exit report both read this identical reference
    /// instead of each re-deriving it from the catalog. Pass `None` when the
    /// catalog carries no price entry for the active model — never a price
    /// substituted from another model, provider, or a hard-coded default.
    pub fn set_price(&mut self, price: Option<PriceReference>) {
        self.price = price;
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
    /// evidence is cleared rather than carried over. The resolved price is
    /// cleared for the same reason: it described the old binding, this
    /// method has no catalog access to re-resolve one for the new binding,
    /// and a stale price would misprice the session exactly as badly as a
    /// stale cache figure would misreport it. The caller that does have
    /// catalog access (`crates/smith-cli`, at startup) calls
    /// [`Self::set_price`] right after switching.
    pub fn switch_model(&mut self, provider: Option<String>, model: impl Into<String>) {
        self.provider = provider;
        self.model = model.into();
        self.cache_read = None;
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
    fn terminal_elapsed_time_is_honest_below_one_second() {
        assert_eq!(render_terminal_elapsed(Duration::ZERO), "<1ms");
        assert_eq!(render_terminal_elapsed(Duration::from_millis(842)), "842ms");
        assert_eq!(render_terminal_elapsed(Duration::from_secs(1)), "1s");
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
        status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-exact",
            cache_fingerprint: "cache-exact",
            input_tokens: 2_500,
            input_budget_tokens: 10_000,
            reserved_tokens: 2_000,
            segment_count: 2,
            totals: &totals,
            confidence: EstimationConfidence::Exact,
        });

        let plan = status.context_plan.as_ref().expect("a plan");
        assert_eq!(plan.remaining_tokens(), 7_500);
        assert_eq!(plan.percent_left(), 75);
        assert_eq!(plan.render_input(), "2.5k");
        assert_eq!(plan.render_footer(), "75% ctx");
        assert_eq!(plan.totals["history"], 2_000);

        status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-estimated",
            cache_fingerprint: "cache-estimated",
            input_tokens: 2_500,
            input_budget_tokens: 10_000,
            reserved_tokens: 2_000,
            segment_count: 2,
            totals: &totals,
            confidence: EstimationConfidence::Estimated,
        });
        let estimated = status.context_plan.as_ref().expect("estimated plan");
        assert_eq!(estimated.render_input(), "~2.5k");
        assert_eq!(estimated.render_footer(), "~75% ctx");
    }

    #[test]
    fn switching_models_clears_a_plan_that_no_longer_applies() {
        let mut status = Status::new("gpt-5.3", "~/work/api");
        let totals = BTreeMap::from([(SegmentKind::new("history"), 100)]);
        status.record_context_plan(ContextPlanUpdate {
            fingerprint: "context-before-switch",
            cache_fingerprint: "cache-before-switch",
            input_tokens: 100,
            input_budget_tokens: 1_000,
            reserved_tokens: 100,
            segment_count: 1,
            totals: &totals,
            confidence: EstimationConfidence::Exact,
        });
        assert_eq!(status.render_context_footer(), "90% ctx");

        status.switch_model(Some("anthropic".into()), "claude-opus-5");
        assert!(status.context_plan.is_none());
        assert_eq!(status.render_context_footer(), "? ctx");
    }

    #[test]
    fn the_account_footer_shows_the_reference_and_its_meter() {
        let mut status = Status::new("gpt-5.3", "~/work");
        assert_eq!(status.render_account_footer(), None, "no pool, no segment");

        status.account = Some(AccountStatus {
            label: "keychain:smith/work".to_owned(),
            used_percent: Some(82.4),
        });
        assert_eq!(
            status.render_account_footer().as_deref(),
            Some("keychain:smith/work 82%")
        );
    }

    #[test]
    fn an_unmeasured_account_is_named_without_a_number() {
        let mut status = Status::new("gpt-5.3", "~/work");
        status.account = Some(AccountStatus {
            label: "keychain:smith/work".to_owned(),
            used_percent: None,
        });
        // Naming the account is useful; guessing its consumption is not.
        assert_eq!(
            status.render_account_footer().as_deref(),
            Some("keychain:smith/work")
        );
    }

    #[test]
    fn the_account_meter_is_separate_from_the_context_footer() {
        let mut status = Status::new("gpt-5.3", "~/work");
        status.account = Some(AccountStatus {
            label: "keychain:smith/work".to_owned(),
            used_percent: Some(82.0),
        });
        // A plan percentage and a token count are different measurements and
        // must never end up in one segment.
        assert!(!status.render_context_footer().contains("82%"));
        assert!(!status.render_context_footer().contains("keychain"));
    }

    fn priced(input: u64, output: u64, cache_read: u64, cache_write: u64) -> PriceReference {
        PriceReference {
            provider: "openai".to_owned(),
            model: "gpt-5.3".to_owned(),
            table: PriceTable {
                input: Some(input),
                output: Some(output),
                cache_read: Some(cache_read),
                cache_write: Some(cache_write),
            },
        }
    }

    #[test]
    fn a_priced_model_with_reported_counters_renders_one_exact_figure() {
        // usage-accounting: "A priced model with reported counters" — every
        // counter the session accumulated is priced and provider-reported,
        // so the figure is exact.
        let mut totals = BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_000_000); // 1M tokens
        totals.insert(CounterKind::Output, 500_000); // 0.5M tokens
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            ..SessionUsage::default()
        };
        // $2/million input, $8/million output.
        let price = priced(2_000_000, 8_000_000, 0, 0);
        let cost = SessionCost::compute(&usage, &price);
        assert_eq!(cost.label, CostLabel::Exact);
        // 1M * $2 + 0.5M * $8 = $2 + $4 = $6.
        assert_eq!(cost.micro_usd, 6_000_000);
        assert_eq!(cost.render(), "$6.000");
    }

    #[test]
    fn an_unreported_session_downgrades_the_cost_label() {
        // usage-accounting: "An estimated counter downgrades the label" — a
        // tokenizer-estimated session must not present its cost as exact
        // just because the price reference itself is exact.
        let mut totals = BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_000_000);
        let usage = SessionUsage {
            turns: 1,
            reported: false,
            totals,
            ..SessionUsage::default()
        };
        let price = priced(2_000_000, 8_000_000, 0, 0);
        let cost = SessionCost::compute(&usage, &price);
        assert_eq!(cost.label, CostLabel::Estimated);
        assert_eq!(cost.render(), "~$2.000");
    }

    #[test]
    fn an_unpriced_contributing_counter_downgrades_the_label_but_still_prices_what_it_can() {
        // A cache-write counter with no catalog price must not be silently
        // folded in as zero without consequence: it downgrades the label,
        // per the task's explicit rule for `CounterKind::Reasoning` applied
        // here to any unpriced counter.
        let mut totals = BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_000_000);
        totals.insert(CounterKind::CacheWrite, 1_000_000);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            ..SessionUsage::default()
        };
        // No cache_write price configured.
        let price = priced(2_000_000, 8_000_000, 0, 0);
        let price = PriceReference {
            table: PriceTable {
                cache_write: None,
                ..price.table
            },
            ..price
        };
        let cost = SessionCost::compute(&usage, &price);
        assert_eq!(cost.label, CostLabel::Estimated);
        // Only the priced input counter contributes; the unpriced cache
        // write contributes nothing rather than being guessed at.
        assert_eq!(cost.micro_usd, 2_000_000);
    }

    #[test]
    fn reasoning_tokens_are_never_priced_and_downgrade_the_label() {
        // Models.dev publishes no reasoning price, and reasoning tokens are
        // billed separately from output tokens (disjoint counters, per
        // `agent_runtime_core::usage::CounterKind::Reasoning`'s own doc), so
        // a nonzero reasoning counter can never be exact even when every
        // other counter is fully priced and provider-reported.
        let mut totals = BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_000_000);
        totals.insert(CounterKind::Reasoning, 200_000);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            ..SessionUsage::default()
        };
        let price = priced(2_000_000, 8_000_000, 4_000_000, 1_000_000);
        let cost = SessionCost::compute(&usage, &price);
        assert_eq!(cost.label, CostLabel::Estimated);
        // Reasoning tokens contribute nothing to the dollar figure — never a
        // price the catalog did not publish — but the priced input still
        // counts.
        assert_eq!(cost.micro_usd, 2_000_000);
    }

    #[test]
    fn delegated_totals_are_priced_by_the_same_reference_as_root() {
        // usage-accounting: "Delegated counters keep their categories" — the
        // delegated totals are priced by the same per-counter reference the
        // root totals are, not by a different (possibly cheaper or more
        // expensive) child model.
        let mut totals = BTreeMap::new();
        totals.insert(CounterKind::InputUncached, 1_000_000);
        let mut delegated_totals = BTreeMap::new();
        delegated_totals.insert(CounterKind::Output, 500_000);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            delegated_totals,
            delegated_contributors: 2,
            ..SessionUsage::default()
        };
        let price = priced(2_000_000, 8_000_000, 0, 0);
        let cost = SessionCost::compute(&usage, &price);
        assert_eq!(cost.label, CostLabel::Exact);
        // Root: 1M * $2 = $2. Delegated: 0.5M * $8 = $4. Total $6, from one
        // reference.
        assert_eq!(cost.micro_usd, 6_000_000);
    }

    #[test]
    fn sub_cent_costs_render_distinctly_from_a_free_session() {
        // A session that spent a real but tiny amount must never render
        // identically to one that spent nothing — the dollar-figure version
        // of the zero/unknown collapse this module exists to prevent.
        let tiny = SessionCost {
            micro_usd: 400, // $0.0004
            label: CostLabel::Exact,
        };
        assert_eq!(tiny.render(), "$0.000400");
        let free = SessionCost {
            micro_usd: 0,
            label: CostLabel::Exact,
        };
        assert_eq!(free.render(), "$0.000");
        assert_ne!(tiny.render(), free.render());
    }

    #[test]
    fn the_established_three_decimal_rendering_matches_design_doc() {
        // DESIGN.md §7: "$0.031" exact, "~$0.031" estimated.
        assert_eq!(
            SessionCost {
                micro_usd: 31_000,
                label: CostLabel::Exact,
            }
            .render(),
            "$0.031"
        );
        assert_eq!(
            SessionCost {
                micro_usd: 31_000,
                label: CostLabel::Estimated,
            }
            .render(),
            "~$0.031"
        );
    }

    #[test]
    fn a_large_session_does_not_overflow_or_panic() {
        // Extreme (not merely large) token and price magnitudes: `u64::MAX`
        // tokens at a `u64::MAX` micro-USD-per-million rate. The
        // intermediate product overflows `u64` (debug builds panic on
        // overflow), which is exactly why `SessionCost::compute` widens to
        // `u128` before dividing back down to micro-USD.
        let mut totals = BTreeMap::new();
        totals.insert(CounterKind::InputUncached, u64::MAX);
        let usage = SessionUsage {
            turns: 1,
            reported: true,
            totals,
            ..SessionUsage::default()
        };
        let price = priced(u64::MAX, 0, 0, 0);
        let cost = SessionCost::compute(&usage, &price);
        let expected = u128::from(u64::MAX) * u128::from(u64::MAX) / 1_000_000;
        assert_eq!(cost.micro_usd, expected);
        assert_eq!(cost.label, CostLabel::Exact);
    }

    #[test]
    fn resolving_a_price_does_not_change_the_recorded_usage_shape() {
        // usage-accounting: "Cost changes no decision" — pinned at the one
        // artifact that actually reaches persistence and every tool-facing
        // surface. `SessionUsage` is what `SessionUsageRecord::new` records
        // and what `App::session_usage` hands to every other surface; it
        // carries no price field at all, so resolving (or clearing) a price
        // on `Status` cannot change what gets recorded or read back,
        // regardless of what the price is.
        let mut status = Status::new("gpt-5.3", "~/work/api");
        status.record_usage(
            &UsageDelta::new()
                .with(CounterKind::InputUncached, 1_000)
                .with(CounterKind::Output, 200),
        );
        let usage_without_price = status.session_usage();

        status.set_price(Some(priced(2_000_000, 8_000_000, 0, 0)));
        let usage_with_price = status.session_usage();

        assert_eq!(
            usage_without_price, usage_with_price,
            "a resolved price must not change the recorded usage shape"
        );
    }

    #[test]
    fn switching_models_clears_a_price_that_no_longer_describes_the_binding() {
        let mut status = Status::new("gpt-5.3", "~/work/api");
        status.set_price(Some(priced(2_000_000, 8_000_000, 0, 0)));
        assert!(status.price().is_some());

        status.switch_model(Some("anthropic".into()), "claude-opus-5");
        assert!(
            status.price().is_none(),
            "the old provider's price does not describe the new binding"
        );
    }
}
