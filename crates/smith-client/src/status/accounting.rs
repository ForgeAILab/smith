use super::*;
/// Counters tied to the binding that produced them, so later model selections
/// cannot change their rates or measurement confidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingUsage {
    /// Absent for CLI agents or an unresolved serving provider.
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
        let model = model.into();
        // The CLI owns its billing; the profile's provider did not serve it.
        let (provider, price) = if smith_config::cli_agents::parse_cli_model_id(&model).is_some() {
            (None, None)
        } else {
            (provider, price)
        };
        Self {
            provider,
            model,
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
    /// Module provider counters and frozen serving bindings, keyed by module id.
    pub module_usage: BTreeMap<String, Vec<BindingUsage>>,
    /// Module work observed on child streams, separate from root reconciliation.
    pub delegated_module_usage: BTreeMap<String, Vec<BindingUsage>>,
    /// Independently resolved advisor rates; absence leaves its spend unpriced.
    pub advisor_price: Option<PriceReference>,
    /// Advisor counters frozen at the price revision used for each attempt.
    pub advisor_bindings: Vec<BindingUsage>,
    /// Vision provider counters, disjoint from ordinary root and child work.
    pub vision_totals: BTreeMap<CounterKind, u64>,
    /// Independently resolved vision rates; absence leaves its spend unpriced.
    pub vision_price: Option<PriceReference>,
    /// Vision counters frozen at the price revision used for each attempt.
    pub vision_bindings: Vec<BindingUsage>,
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
            return active.or_else(|| {
                self.advisor_bindings
                    .iter()
                    .chain(&self.vision_bindings)
                    .find_map(|binding| binding.price.as_ref())
                    .or_else(|| {
                        self.vision_price
                            .as_ref()
                            .filter(|_| !self.vision_totals.is_empty())
                    })
                    .or_else(|| {
                        self.module_usage
                            .values()
                            .chain(self.delegated_module_usage.values())
                            .flatten()
                            .find_map(|binding| binding.price.as_ref())
                    })
            });
        }
        let retained = self
            .bindings
            .iter()
            .find_map(|binding| binding.price.as_ref());
        active
            .filter(|_| retained.is_some())
            .or(retained)
            .or_else(|| {
                self.module_usage
                    .values()
                    .chain(self.delegated_module_usage.values())
                    .flatten()
                    .find_map(|binding| binding.price.as_ref())
            })
            .or_else(|| {
                self.advisor_bindings
                    .iter()
                    .chain(&self.vision_bindings)
                    .find_map(|binding| binding.price.as_ref())
                    .or_else(|| {
                        self.advisor_price
                            .as_ref()
                            .filter(|_| !self.advisor_totals.is_empty())
                    })
            })
            .or_else(|| {
                self.vision_price
                    .as_ref()
                    .filter(|_| !self.vision_totals.is_empty())
            })
    }

    /// Whether anything at all was observed, including a delegated-only
    /// session that never accumulated any root usage of its own.
    pub fn is_empty(&self) -> bool {
        self.totals.is_empty()
            && self.turns == 0
            && self.synthetic_totals.is_empty()
            && self.advisor_totals.is_empty()
            && self.vision_totals.is_empty()
            && self.module_usage.is_empty()
            && self.delegated_module_usage.is_empty()
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
        token_total(&self.totals)
    }

    /// Token totals across root, delegated, and synthetic provider
    /// usage. Synthetic counters remain separately inspectable even though
    /// they participate in this provider/session total.
    pub fn merged_total_tokens(&self) -> u64 {
        self.total_tokens()
            + token_total(&self.delegated_totals)
            + token_total(&self.synthetic_totals)
            + token_total(&self.advisor_totals)
            + token_total(&self.vision_totals)
            + self
                .module_usage
                .values()
                .chain(self.delegated_module_usage.values())
                .flatten()
                .map(|binding| token_total(&binding.totals))
                .sum::<u64>()
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
        reconcile_binding_totals(&mut self.advisor_bindings, &self.advisor_totals);
    }

    /// Replaces vision counters from the durable ledger after shutdown.
    pub fn reconcile_vision_records(&mut self, records: &[UsageRecord]) {
        self.vision_totals.clear();
        for record in records {
            if record.provenance.purpose.as_deref() == Some(VISION_USAGE_PURPOSE) {
                self.reported |= !record.delta.is_empty();
                for (kind, value) in record.delta.iter() {
                    *self.vision_totals.entry(kind).or_default() += value;
                }
            }
        }
        reconcile_binding_totals(&mut self.vision_bindings, &self.vision_totals);
    }

    /// Aggregates each module's separately attributed counters across bindings.
    pub fn module_totals(&self) -> BTreeMap<String, BTreeMap<CounterKind, u64>> {
        let mut modules = BTreeMap::<String, BTreeMap<CounterKind, u64>>::new();
        for (module, bindings) in self.module_usage.iter().chain(&self.delegated_module_usage) {
            for binding in bindings {
                for (kind, value) in &binding.totals {
                    *modules
                        .entry(module.clone())
                        .or_default()
                        .entry(*kind)
                        .or_default() += value;
                }
            }
        }
        modules
    }

    /// Reconciles module records without borrowing the last model's rates for
    /// recovered usage whose serving binding is unknown, as for advisor usage.
    pub fn reconcile_module_records(&mut self, records: &[UsageRecord]) {
        let mut expected = BTreeMap::<String, BTreeMap<CounterKind, u64>>::new();
        for record in records {
            if let Some(module) = record
                .provenance
                .purpose
                .as_deref()
                .and_then(|purpose| purpose.strip_prefix(MODULE_USAGE_PREFIX))
                && !record.delta.is_empty()
            {
                self.reported = true;
                for (kind, value) in record.delta.iter() {
                    *expected
                        .entry(module.into())
                        .or_default()
                        .entry(kind)
                        .or_default() += value;
                }
            }
        }
        let mut root = self.clone();
        root.delegated_module_usage.clear();
        let existing = root.module_totals();
        self.module_usage
            .retain(|module, _| expected.contains_key(module));
        for (module, totals) in expected {
            if existing.get(&module) == Some(&totals) {
                continue;
            }
            let previous = existing.get(&module);
            let can_retain = previous.is_some_and(|previous| {
                previous
                    .iter()
                    .all(|(kind, value)| totals.get(kind).copied().unwrap_or(0) >= *value)
            });
            let mut binding = BindingUsage::new(None, "earlier models", None);
            if can_retain {
                let previous = previous.expect("checked previous counters");
                for (kind, value) in totals {
                    let missing = value.saturating_sub(previous.get(&kind).copied().unwrap_or(0));
                    if missing > 0 {
                        binding.totals.insert(kind, missing);
                    }
                }
                self.module_usage.entry(module).or_default().push(binding);
            } else {
                binding.totals = totals;
                self.module_usage.insert(module, vec![binding]);
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
            && self.vision_totals.is_empty()
            && self.module_usage.is_empty()
            && self.delegated_module_usage.is_empty()
        {
            return Some(root_line);
        }

        let module_totals = self.module_totals();
        let module_sum = module_totals
            .values()
            .fold(BTreeMap::new(), |total, counters| {
                merge_counter_totals(&total, counters)
            });
        let merged = merge_counter_totals(
            &merge_counter_totals(&self.totals, &self.delegated_totals),
            &merge_counter_totals(
                &merge_counter_totals(&self.synthetic_totals, &self.advisor_totals),
                &merge_counter_totals(&self.vision_totals, &module_sum),
            ),
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
        if !self.vision_totals.is_empty() {
            lines.push(format!(
                "  vision: {}",
                render_counter_parts(&self.vision_totals, mark).join(" · ")
            ));
        }
        for (module, totals) in module_totals {
            lines.push(format!(
                "  module {module}: {}",
                render_counter_parts(&totals, mark).join(" · ")
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

/// Sums token counters, excluding provider-hosted search requests.
fn token_total(totals: &BTreeMap<CounterKind, u64>) -> u64 {
    totals
        .iter()
        .filter(|(kind, _)| **kind != CounterKind::WebSearchRequests)
        .map(|(_, value)| *value)
        .sum()
}

/// Renders one token counter/value pair per entry, e.g. `input ~12.4k`.
fn render_counter_parts(totals: &BTreeMap<CounterKind, u64>, mark: &str) -> Vec<String> {
    totals
        .iter()
        .filter(|(kind, _)| **kind != CounterKind::WebSearchRequests)
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
        CounterKind::WebSearchRequests => "web-search-requests",
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
/// — and the reviewed catalog price schemas carry no distinct reasoning
/// price. Charging reasoning tokens at the output rate would
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
        CounterKind::Reasoning | CounterKind::WebSearchRequests => None,
    }
}

/// Immutable provenance retained with the rates used by a serving binding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceProvenance {
    /// Secret-free source identity, including an endpoint's account scope.
    pub source: String,
    /// Immutable normalized catalog content revision.
    pub revision: String,
    /// Time this source revision was retrieved, in Unix milliseconds.
    pub retrieved_at_ms: u64,
}

/// One binding's catalog price, retained with the counters it produced.
///
/// Resolved at host composition and frozen before usage arrives, so `/status`,
/// exit reports, and resume all use the same rates after a catalog refresh.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceReference {
    /// The serving provider's name, so a rendered cost can say where the
    /// price came from.
    pub provider: String,
    /// The model identity the price describes.
    pub model: String,
    /// Per-counter micro-USD-per-million prices.
    pub table: PriceTable,
    /// Absent for legacy/manual references that predate versioned catalog prices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<PriceProvenance>,
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
            provenance: None,
        }
    }

    /// Retains catalog provenance instead of consulting a newer snapshot later.
    pub fn with_provenance(
        mut self,
        source: impl Into<String>,
        revision: impl Into<String>,
        retrieved_at_ms: u64,
    ) -> Self {
        self.provenance = Some(PriceProvenance {
            source: source.into(),
            revision: revision.into(),
            retrieved_at_ms,
        });
        self
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
        for (label, totals, bindings, active) in [
            (
                "advisor",
                &usage.advisor_totals,
                &usage.advisor_bindings,
                &usage.advisor_price,
            ),
            (
                "vision",
                &usage.vision_totals,
                &usage.vision_bindings,
                &usage.vision_price,
            ),
        ] {
            if totals.is_empty() {
                continue;
            }
            if bindings.is_empty() {
                match active {
                    Some(price) => {
                        sources.push_str(&format!(" · {label} {}/{}", price.provider, price.model))
                    }
                    None => sources.push_str(&format!(" · {label} price unknown")),
                }
                continue;
            }
            let mut seen = std::collections::BTreeSet::new();
            for binding in bindings {
                if binding.totals.is_empty() {
                    continue;
                }
                let source = match &binding.price {
                    Some(price) => format!("{}/{}", price.provider, price.model),
                    None => "price unknown".into(),
                };
                if seen.insert(source.clone()) {
                    sources.push_str(&format!(" · {label} {source}"));
                }
            }
        }
        for (module, bindings) in usage
            .module_usage
            .iter()
            .chain(&usage.delegated_module_usage)
        {
            for binding in bindings {
                sources.push_str(&format!(" · module {module} {}", binding.name()));
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
        for (totals, bindings, active) in [
            (
                &usage.advisor_totals,
                &usage.advisor_bindings,
                usage.advisor_price.as_ref(),
            ),
            (
                &usage.vision_totals,
                &usage.vision_bindings,
                usage.vision_price.as_ref(),
            ),
        ] {
            if !bindings.is_empty() {
                for binding in bindings {
                    all_reported &= binding.reported;
                    for (kind, tokens) in &binding.totals {
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
                continue;
            }
            for (kind, tokens) in totals {
                if let Some(price) = active {
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
        for binding in usage
            .module_usage
            .values()
            .chain(usage.delegated_module_usage.values())
            .flatten()
        {
            all_reported &= binding.reported;
            for (kind, tokens) in &binding.totals {
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

/// Canonical reconciliation may recover attempts the live stream did not see.
/// Retain known revisions and keep only newly recovered counters unpriced.
fn reconcile_binding_totals(bindings: &mut Vec<BindingUsage>, totals: &BTreeMap<CounterKind, u64>) {
    let mut known = BTreeMap::<CounterKind, u64>::new();
    for binding in bindings.iter() {
        for (kind, value) in &binding.totals {
            let total = known.entry(*kind).or_default();
            *total = total.saturating_add(*value);
        }
    }
    if known == *totals {
        return;
    }
    let retain = known
        .iter()
        .all(|(kind, value)| totals.get(kind).copied().unwrap_or(0) >= *value);
    if !retain {
        bindings.clear();
        known.clear();
    }
    let mut recovered = BindingUsage::new(None, "earlier models", None);
    for (kind, value) in totals {
        let missing = value.saturating_sub(known.get(kind).copied().unwrap_or(0));
        if missing > 0 {
            recovered.totals.insert(*kind, missing);
        }
    }
    if recovered.has_usage() {
        bindings.push(recovered);
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
