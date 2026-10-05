use super::*;
/// Counters tied to the binding that produced them, so later model selections
/// cannot change their rates or measurement confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingUsage {
    /// Absent when the host could not resolve a serving provider.
    pub provider: Option<String>,
    /// Model identity, or a bounded explanation such as `earlier models`.
    pub model: String,
    /// Disjoint counters retained independently of the session rollup.
    pub totals: BTreeMap<CounterKind, u64>,
    /// Synthetic counters retain the binding active at reporting time, so a
    /// later model selection cannot reprice cache maintenance. The session's
    /// synthetic rollups remain separate and unchanged. Idle compaction is
    /// retained only in those rollups because its summary model may differ.
    pub synthetic_by_purpose: BTreeMap<ProviderAttemptPurpose, BTreeMap<CounterKind, u64>>,
    /// Whether all counters in this bucket were provider-reported.
    pub reported: bool,
    /// Frozen catalog reference; absence never borrows another binding's rates.
    pub price: Option<PriceReference>,
}

impl BindingUsage {
    /// Starts a bucket before usage arrives, preserving an unresolved identity.
    pub fn new(
        provider: Option<String>,
        model: impl Into<String>,
        price: Option<PriceReference>,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            totals: BTreeMap::new(),
            synthetic_by_purpose: BTreeMap::new(),
            reported: true,
            price,
        }
    }

    /// Accumulates counters without allowing a later report to erase estimates.
    pub fn record(&mut self, delta: &UsageDelta, reported: bool) {
        self.reported &= reported;
        for (kind, value) in delta.iter() {
            let total = self.totals.entry(kind).or_default();
            *total = total.saturating_add(value);
        }
    }

    /// Synthetic-only buckets must survive switches and freeze their rates.
    pub(super) fn has_usage(&self) -> bool {
        !self.totals.is_empty() || !self.synthetic_by_purpose.is_empty()
    }

    /// Idle compaction can use a separate summary model, so its counters
    /// never use this binding's root-model rates.
    fn cost_totals(&self) -> impl Iterator<Item = (&CounterKind, &u64)> {
        self.totals.iter().chain(
            self.synthetic_by_purpose
                .iter()
                .filter(|(purpose, _)| **purpose != ProviderAttemptPurpose::IdleCompaction)
                .flat_map(|(_, totals)| totals.iter()),
        )
    }

    fn name(&self) -> String {
        self.provider.as_ref().map_or_else(
            || self.model.clone(),
            |provider| format!("{provider}/{}", self.model),
        )
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
    /// Presentation-only attribution alongside unchanged root/child rollups.
    pub bindings: Vec<BindingUsage>,
    /// User-started root conversation turns, independent of provider attempts.
    pub turns: u32,
    /// Whether any counter came from the provider rather than an estimate.
    pub reported: bool,
    /// Per-counter totals, omitting counters the provider never reported.
    pub totals: BTreeMap<CounterKind, u64>,
    /// Provider usage from cache-maintenance and idle-compaction attempts.
    /// These counters count toward the session's provider spend, but remain
    /// disjoint from root and delegated turn usage.
    pub synthetic_totals: BTreeMap<CounterKind, u64>,
    /// Synthetic provider counters partitioned by their typed Runtime
    /// purpose. Keys are never inferred from text labels.
    pub synthetic_by_purpose: BTreeMap<ProviderAttemptPurpose, BTreeMap<CounterKind, u64>>,
    /// Reviewer provider counters, disjoint from ordinary root and child work.
    pub advisor_totals: BTreeMap<CounterKind, u64>,
    /// Independently resolved advisor rates; absence leaves its spend unpriced.
    pub advisor_price: Option<PriceReference>,
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
    /// Canonical cache-miss attempts observed in the root session. This is a
    /// derived diagnostic, not a usage counter.
    pub cache_miss_count: u32,
    /// Canonical missed tokens observed in the root session. These tokens are
    /// already present in ordinary input counters and are never added again.
    pub cache_rebilled_tokens: u64,
}

impl SessionUsage {
    /// Finds a retained price even when the last root binding is unpriced.
    /// Legacy callers without binding buckets keep their supplied reference.
    pub fn cost_price<'a>(
        &'a self,
        active: Option<&'a PriceReference>,
    ) -> Option<&'a PriceReference> {
        if self.bindings.is_empty() {
            return active;
        }
        let retained = self
            .bindings
            .iter()
            .find_map(|binding| binding.price.as_ref());
        active
            .filter(|_| retained.is_some())
            .or(retained)
            .or_else(|| {
                self.advisor_price
                    .as_ref()
                    .filter(|_| !self.advisor_totals.is_empty())
            })
    }

    /// Whether anything at all was observed, including a delegated-only
    /// session that never accumulated any root usage of its own.
    pub fn is_empty(&self) -> bool {
        self.totals.is_empty()
            && self.turns == 0
            && self.synthetic_totals.is_empty()
            && self.advisor_totals.is_empty()
            && self.delegated_totals.is_empty()
            && self.cache_miss_count == 0
            && self.cache_rebilled_tokens == 0
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

    /// Every counter's total across root, delegated, and synthetic provider
    /// usage. Synthetic counters remain separately inspectable even though
    /// they participate in this provider/session total.
    pub fn merged_total_tokens(&self) -> u64 {
        self.total_tokens()
            + self.delegated_totals.values().copied().sum::<u64>()
            + self.synthetic_totals.values().copied().sum::<u64>()
            + self.advisor_totals.values().copied().sum::<u64>()
    }

    /// Replaces the synthetic bucket from Runtime's final canonical usage
    /// ledger. Interactive exit uses this after shutdown so an attempt that
    /// completed or cancelled during the drain is counted exactly once.
    /// Live binding attribution is retained because these durable records do
    /// not identify the serving model; newly recovered, unattributed counters
    /// remain unpriced rather than borrowing the last model's rates.
    pub fn reconcile_synthetic_records(&mut self, records: &[UsageRecord]) {
        self.synthetic_totals.clear();
        self.synthetic_by_purpose.clear();
        for record in records {
            let Some(purpose) = record.provenance.attempt_purpose else {
                continue;
            };
            if !purpose.is_synthetic_cache() || record.delta.is_empty() {
                continue;
            }
            self.reported = true;
            let purpose_totals = self.synthetic_by_purpose.entry(purpose).or_default();
            for (kind, value) in record.delta.iter() {
                *self.synthetic_totals.entry(kind).or_insert(0) += value;
                *purpose_totals.entry(kind).or_insert(0) += value;
            }
        }
    }

    /// Replaces advisor counters from the durable ledger after shutdown.
    pub fn reconcile_advisor_records(&mut self, records: &[UsageRecord]) {
        self.advisor_totals.clear();
        for record in records {
            if record.provenance.purpose.as_deref() == Some(ADVISOR_USAGE_PURPOSE) {
                self.reported |= !record.delta.is_empty();
                for (kind, value) in record.delta.iter() {
                    *self.advisor_totals.entry(kind).or_default() += value;
                }
            }
        }
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
        let root_line =
            append_cache_diagnostics(root_line, self.cache_miss_count, self.cache_rebilled_tokens);
        if self.delegated_totals.is_empty()
            && self.synthetic_totals.is_empty()
            && self.advisor_totals.is_empty()
        {
            return Some(root_line);
        }

        let merged = merge_counter_totals(
            &merge_counter_totals(&self.totals, &self.delegated_totals),
            &merge_counter_totals(&self.synthetic_totals, &self.advisor_totals),
        );
        let mut merged_line = format!(
            "total · {}",
            render_counter_parts(&merged, mark).join(" · ")
        );
        if !self.reported {
            merged_line.push_str(" · estimated");
        }
        let mut lines = vec![merged_line, format!("  root: {root_line}")];
        if !self.delegated_totals.is_empty() {
            let agent_parts = render_counter_parts(&self.delegated_totals, mark);
            lines.push(format!(
                "  agents: {} · {}",
                plural(self.delegated_contributors, "agent", "agents"),
                agent_parts.join(" · "),
            ));
        }
        if !self.synthetic_totals.is_empty() {
            let purposes = self
                .synthetic_by_purpose
                .iter()
                .map(|(purpose, totals)| {
                    format!(
                        "{} ({})",
                        purpose.as_str(),
                        render_counter_parts(totals, mark).join(" · ")
                    )
                })
                .collect::<Vec<_>>();
            lines.push(format!("  cache maintenance: {}", purposes.join("; ")));
        }
        if !self.advisor_totals.is_empty() {
            lines.push(format!(
                "  advisor: {}",
                render_counter_parts(&self.advisor_totals, mark).join(" · ")
            ));
        }
        Some(lines.join("\n"))
    }
}

fn append_cache_diagnostics(line: String, miss_count: u32, rebilled_tokens: u64) -> String {
    if miss_count == 0 && rebilled_tokens == 0 {
        return line;
    }
    format!(
        "{line} · cache re-billed {} · {miss_count} miss{}",
        compact_tokens(rebilled_tokens),
        if miss_count == 1 { "" } else { "es" },
    )
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

/// Keeps root usage counts and compaction wording consistent across surfaces;
/// provider attempts never substitute for the user-started turn count.
fn format_usage_line(
    turns: u32,
    parts: &[String],
    reported: bool,
    compactions: u32,
    reclaimed_tokens: u64,
) -> String {
    let turns = plural(turns, "turn", "turns");
    let mut line = if parts.is_empty() {
        turns
    } else {
        format!("{turns} · {}", parts.join(" · "))
    };
    if !reported {
        line.push_str(" · estimated");
    }
    if compactions > 0 {
        line.push_str(&format!(
            " · {} reclaiming {}",
            plural(compactions, "compaction", "compactions"),
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

/// The catalog's canonical per-counter micro-USD-per-million prices.
///
/// The client retains its established name without mirroring the catalog type.
pub use smith_config::catalog::CatalogModelCost as PriceTable;

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
fn price_for(table: &PriceTable, kind: CounterKind) -> Option<u64> {
    match kind {
        CounterKind::InputUncached => table.input,
        CounterKind::InputCached => table.cache_read,
        CounterKind::CacheWrite => table.cache_write,
        CounterKind::Output => table.output,
        CounterKind::Reasoning => None,
    }
}

/// One binding's catalog price, retained with the counters it produced.
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

impl PriceReference {
    /// Labels the active binding's canonical catalog prices without retyping them.
    pub fn from_catalog(
        provider: impl Into<String>,
        model: impl Into<String>,
        cost: &smith_config::catalog::CatalogModelCost,
    ) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            table: *cost,
        }
    }

    /// Names every model reference contributing to the displayed session cost.
    pub fn render_sources(&self, usage: &SessionUsage) -> String {
        let mut shares: Vec<(String, Option<u128>)> = Vec::new();
        for binding in &usage.bindings {
            if binding.cost_totals().next().is_none() {
                continue;
            }
            let name = binding.name();
            let amount = binding.price.as_ref().map(|price| {
                let mut amount = 0;
                let mut all_priced = true;
                for (kind, tokens) in binding.cost_totals() {
                    accumulate_price(*kind, *tokens, &price.table, &mut amount, &mut all_priced);
                }
                amount
            });
            if let Some((_, total)) = shares
                .iter_mut()
                .find(|(source, total)| *source == name && total.is_some() == amount.is_some())
            {
                if let (Some(total), Some(amount)) = (total, amount) {
                    *total += amount;
                }
            } else {
                shares.push((name, amount));
            }
        }
        let multiple = shares.len() > 1;
        let mut sources = if shares.is_empty() {
            format!("{}/{}", self.provider, self.model)
        } else {
            shares
                .into_iter()
                .map(|(name, amount)| match amount {
                    Some(amount) if multiple => format!("{name} {}", format_usd(amount)),
                    Some(_) => name,
                    None => format!("price unknown for {name}"),
                })
                .collect::<Vec<_>>()
                .join(" · ")
        };
        if !usage.advisor_totals.is_empty() {
            match &usage.advisor_price {
                Some(advisor) => sources.push_str(&format!(
                    " · advisor {}/{}",
                    advisor.provider, advisor.model
                )),
                None => sources.push_str(" · advisor price unknown"),
            }
        }
        sources
    }
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
    /// Prices each attributed root/child bucket, including same-model
    /// synthetic work, at its frozen reference.
    /// The supplied reference preserves the legacy single-binding API for
    /// callers that construct rollups without attribution.
    ///
    /// Uses a `u128` intermediate for the multiply so a long session's token
    /// counts cannot overflow the arithmetic before the division back down
    /// to micro-USD.
    pub fn compute(usage: &SessionUsage, price: &PriceReference) -> Self {
        let mut micro_usd: u128 = 0;
        let mut all_priced = true;
        let mut all_reported = usage.reported;
        if usage.bindings.is_empty() {
            for (kind, tokens) in usage.totals.iter().chain(usage.delegated_totals.iter()) {
                accumulate_price(
                    *kind,
                    *tokens,
                    &price.table,
                    &mut micro_usd,
                    &mut all_priced,
                );
            }
        } else {
            all_reported = true;
            for binding in &usage.bindings {
                if binding.cost_totals().next().is_none() {
                    continue;
                }
                all_reported &= binding.reported;
                for (kind, tokens) in binding.cost_totals() {
                    if let Some(price) = &binding.price {
                        accumulate_price(
                            *kind,
                            *tokens,
                            &price.table,
                            &mut micro_usd,
                            &mut all_priced,
                        );
                    } else if *tokens > 0 {
                        all_priced = false;
                    }
                }
            }
        }
        // Attributed synthetic counters were priced with their binding above.
        // Only legacy rollup-only callers use the supplied reference. Idle
        // compaction may use a separate summary model and stays unpriced.
        if usage.synthetic_by_purpose.is_empty() && !usage.synthetic_totals.is_empty() {
            all_priced = false;
        }
        for (purpose, totals) in &usage.synthetic_by_purpose {
            if *purpose == ProviderAttemptPurpose::IdleCompaction {
                if totals.values().any(|tokens| *tokens > 0) {
                    all_priced = false;
                }
                continue;
            }
            for (kind, tokens) in totals {
                if usage.bindings.is_empty() {
                    accumulate_price(
                        *kind,
                        *tokens,
                        &price.table,
                        &mut micro_usd,
                        &mut all_priced,
                    );
                } else {
                    let attributed = usage
                        .bindings
                        .iter()
                        .filter_map(|binding| binding.synthetic_by_purpose.get(purpose))
                        .filter_map(|totals| totals.get(kind))
                        .fold(0_u64, |total, tokens| total.saturating_add(*tokens));
                    if *tokens > attributed {
                        all_priced = false;
                    }
                }
            }
        }
        for (kind, tokens) in &usage.advisor_totals {
            if let Some(advisor_price) = &usage.advisor_price {
                accumulate_price(
                    *kind,
                    *tokens,
                    &advisor_price.table,
                    &mut micro_usd,
                    &mut all_priced,
                );
            } else if *tokens > 0 {
                all_priced = false;
            }
        }
        // A later provider report cannot upgrade an earlier bucket's
        // estimates. Unpriced counters downgrade the label independently.
        let label = if all_reported && all_priced {
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

fn accumulate_price(
    kind: CounterKind,
    tokens: u64,
    table: &PriceTable,
    micro_usd: &mut u128,
    all_priced: &mut bool,
) {
    if tokens == 0 {
        return;
    }
    match price_for(table, kind) {
        Some(rate) => {
            *micro_usd += u128::from(tokens) * u128::from(rate) / 1_000_000;
        }
        None => *all_priced = false,
    }
}
