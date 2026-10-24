use super::{BTreeMap, CounterKind, RuntimePolicy, SelectionInventory, UsageRecord};

/// A later unattributed record must not hide matching per-model counters.
pub(super) fn last_session_usage(
    paths: &smith_runtime::session::SessionPaths,
    session: &agent_runtime_core::ids::SessionId,
    records: &[UsageRecord],
) -> Option<smith_client::usage_log::SessionUsageRecord> {
    let mut restored = smith_client::status::Status::new("", "");
    restore_usage_records(&mut restored, records);
    let totals = restored
        .session_usage()
        .totals
        .iter()
        .map(|(kind, value)| {
            (
                smith_client::status::counter_label(*kind).to_owned(),
                *value,
            )
        })
        .collect::<BTreeMap<_, _>>();
    smith_client::usage_log::read_all(&smith_client::usage_log::default_path(paths.directory()))
        .into_iter()
        .rev()
        .find(|record| {
            record.session == session.as_str()
                && record.totals == totals
                && usage_bindings_are_attributed(record)
        })
}

fn usage_bindings_are_attributed(record: &smith_client::usage_log::SessionUsageRecord) -> bool {
    matches!(record.schema_version, 5 | 6)
        && (!record.bindings.is_empty() || (record.schema_version == 6 && record.totals.is_empty()))
        && record.bindings.iter().all(|binding| {
            (smith_config::cli_agents::parse_cli_model_id(&binding.model).is_some()
                || binding
                    .provider
                    .as_deref()
                    .is_some_and(|provider| !provider.trim().is_empty()))
                && !binding.model.trim().is_empty()
                && binding.model != "earlier models"
        })
}

/// Seeds the status projection from durable Runtime records without losing
/// their typed provenance. In particular, synthetic cache attempts count
/// toward session spend but never become an ordinary user turn.
pub(super) fn restore_usage_records(
    status: &mut smith_client::status::Status,
    records: &[UsageRecord],
) {
    for record in records {
        status.record_usage_record(record);
    }
}

/// Matching logs preserve serving identities; v6 additionally freezes prices.
/// Earlier logs retain attribution while unrecoverable historical prices stay
/// unknown instead of borrowing the newest catalog revision.
pub(super) fn restore_usage_with_bindings<'a>(
    status: &mut smith_client::status::Status,
    records: &[UsageRecord],
    logged: Option<&smith_client::usage_log::SessionUsageRecord>,
    manifests: impl IntoIterator<Item = (&'a str, &'a str)>,
    _price_for: impl FnMut(&str, &str) -> Option<smith_client::status::PriceReference>,
) {
    if records.is_empty() {
        return;
    }
    let provider = status.provider.clone();
    let model = status.model.clone();
    let price = status.price().cloned();
    let active = status.session_usage();
    status.set_advisor_price(None);
    status.set_vision_price(None);
    let bindings = manifests
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if bindings.len() == 1
        && let Some((provider, model)) = bindings.iter().next()
    {
        status.switch_model(Some((*provider).to_owned()), *model);
        // Manifests identify models, not the rates in force for old attempts.
        status.set_price(None);
    } else {
        status.switch_model(None, "earlier models");
    }
    restore_usage_records(status, records);
    if let Some(logged) = logged {
        restore_logged_bindings(status, logged);
    }
    status.switch_model(provider, model);
    status.set_price(price);
    status.set_advisor_price(active.advisor_price);
    status.set_vision_price(active.vision_price);
}

/// Checks the root rollup and the binding partition before using the log:
/// v4's synthesized last-model bucket is not evidence of earlier attribution.
fn restore_logged_bindings(
    status: &mut smith_client::status::Status,
    logged: &smith_client::usage_log::SessionUsageRecord,
) {
    use smith_client::status::counter_label;
    let mut restored = status.session_usage();
    let totals = restored
        .totals
        .iter()
        .map(|(kind, value)| (counter_label(*kind).to_owned(), *value))
        .collect::<std::collections::BTreeMap<_, _>>();
    if logged.totals != totals || !usage_bindings_are_attributed(logged) {
        return;
    }
    let mut partition = std::collections::BTreeMap::<String, u64>::new();
    for binding in &logged.bindings {
        for (kind, value) in &binding.totals {
            let total = partition.entry(kind.clone()).or_default();
            let Some(sum) = total.checked_add(*value) else {
                return;
            };
            *total = sum;
        }
    }
    let mut expected = totals;
    for (kind, value) in &logged.delegated_totals {
        let total = expected.entry(kind.clone()).or_default();
        let Some(sum) = total.checked_add(*value) else {
            return;
        };
        *total = sum;
    }
    if partition != expected {
        return;
    }
    let versioned = logged.schema_version == 6;
    let Some(bindings) = bindings_from_log(&logged.bindings, versioned, logged.reported) else {
        return;
    };
    // A same-root-total log can still predate advisor/cache maintenance work.
    // Keep the live ledger authoritative and only restore matching partitions.
    restored.bindings = bindings;
    if versioned {
        let synthetic = restored.bindings.iter().fold(
            BTreeMap::<String, BTreeMap<String, u64>>::new(),
            |mut all, binding| {
                for (purpose, totals) in &binding.synthetic_by_purpose {
                    for (kind, value) in totals {
                        let total = all
                            .entry(purpose.as_str().into())
                            .or_default()
                            .entry(counter_label(*kind).into())
                            .or_default();
                        *total = total.saturating_add(*value);
                    }
                }
                all
            },
        );
        for (purpose, totals) in synthetic {
            if logged.synthetic_by_purpose.get(&purpose) != Some(&totals) {
                return;
            }
            let canonical = restored
                .synthetic_by_purpose
                .iter()
                .find(|(known, _)| known.as_str() == purpose)
                .map(|(_, totals)| totals);
            if totals.iter().any(|(label, value)| {
                canonical.is_none_or(|canonical| {
                    canonical
                        .iter()
                        .find(|(kind, _)| counter_label(**kind) == label)
                        .is_none_or(|(_, known)| value > known)
                })
            }) {
                return;
            }
        }
        for (entries, target, expected) in [
            (
                &logged.advisor_bindings,
                &mut restored.advisor_bindings,
                &restored.advisor_totals,
            ),
            (
                &logged.vision_bindings,
                &mut restored.vision_bindings,
                &restored.vision_totals,
            ),
        ] {
            if let Some(bindings) = bindings_from_log(entries, true, logged.reported)
                && binding_totals(&bindings) == *expected
            {
                *target = bindings;
            }
        }
    }
    status.retain_usage_bindings(&restored);
    let mut module_usage = BTreeMap::new();
    for (module, entries) in &logged.module_usage {
        let Some(bindings) = bindings_from_log(entries, versioned, logged.reported) else {
            return;
        };
        module_usage.insert(module.clone(), bindings);
    }
    let expected_modules = restored.module_totals();
    restored.module_usage = module_usage;
    if restored.module_totals() == expected_modules {
        status.retain_usage_bindings(&restored);
    }
}

fn binding_totals(bindings: &[smith_client::status::BindingUsage]) -> BTreeMap<CounterKind, u64> {
    let mut totals = BTreeMap::<CounterKind, u64>::new();
    for binding in bindings {
        for (kind, value) in &binding.totals {
            let total = totals.entry(*kind).or_default();
            *total = total.saturating_add(*value);
        }
    }
    totals
}

fn bindings_from_log(
    entries: &[smith_client::usage_log::BindingUsageRecord],
    versioned: bool,
    reported: bool,
) -> Option<Vec<smith_client::status::BindingUsage>> {
    use agent_runtime_core::provider::ProviderAttemptPurpose;
    use smith_client::status::{BindingUsage, counter_label};
    let kinds = [
        CounterKind::InputUncached,
        CounterKind::InputCached,
        CounterKind::CacheWrite,
        CounterKind::Output,
        CounterKind::Reasoning,
        CounterKind::WebSearchRequests,
    ];
    let purposes = [
        ProviderAttemptPurpose::CacheKeepalive,
        ProviderAttemptPurpose::CacheHandoffCheckpoint,
        ProviderAttemptPurpose::IdleCompaction,
        ProviderAttemptPurpose::CacheResourceCreate,
        ProviderAttemptPurpose::CacheResourceExtend,
        ProviderAttemptPurpose::CacheResourceInspect,
        ProviderAttemptPurpose::CacheResourceDelete,
    ];
    entries
        .iter()
        .map(|entry| {
            let price = entry
                .price
                .as_ref()
                .filter(|price| {
                    versioned
                        && entry.provider.as_deref() == Some(price.provider.as_str())
                        && entry.model == price.model
                })
                .cloned();
            let mut binding = BindingUsage::new(entry.provider.clone(), &entry.model, price);
            for (label, value) in &entry.totals {
                let kind = kinds.iter().find(|kind| counter_label(**kind) == label)?;
                binding.totals.insert(*kind, *value);
            }
            if versioned {
                for (purpose, totals) in &entry.synthetic_by_purpose {
                    let purpose = purposes.iter().find(|known| known.as_str() == purpose)?;
                    let mut converted = BTreeMap::new();
                    for (label, value) in totals {
                        let kind = kinds.iter().find(|kind| counter_label(**kind) == label)?;
                        converted.insert(*kind, *value);
                    }
                    binding.synthetic_by_purpose.insert(*purpose, converted);
                }
            }
            binding.reported = reported && entry.reported;
            Some(binding)
        })
        .collect()
}

/// Resolves the active model's catalog price, using **exactly** the binding
/// the runtime factory itself resolves models against — this mirrors
/// `crates/smith-runtime/src/factory.rs`'s `prepare_factory_inputs` catalog
/// lookup line for line, rather than inventing a second resolution that
/// could disagree with it and price the wrong model.
///
/// Returns `None` when the catalog carries no price entry for this binding.
/// That is never treated as "assume some other price" anywhere downstream —
/// `usage-accounting`'s "Labelled cost calculation" forbids substituting a
/// price from another model, provider, or a hard-coded default, and a
/// `None` here is exactly how that absence is represented.
pub(crate) fn resolve_price(
    policy: &RuntimePolicy,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::PriceReference> {
    let binding = catalog.bound_provider(
        &policy.provider_name,
        &policy.provider_kind,
        policy.endpoint.as_deref(),
    )?;
    bound_price(&policy.provider_name, policy.model.as_str(), binding)
}

pub(crate) fn resolve_model_price(
    policy: &RuntimePolicy,
    provider: &str,
    model: &str,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::PriceReference> {
    if provider == policy.provider_name && model == policy.model.as_str() {
        return resolve_price(policy, catalog);
    }
    let route = policy
        .fallback_models
        .iter()
        .find(|route| route.provider_name == provider && route.model.as_str() == model)?;
    let binding =
        catalog.bound_provider(provider, &route.provider_kind, route.endpoint.as_deref())?;
    bound_price(provider, model, binding)
}

pub(crate) fn apply_model_policy(
    status: &mut smith_client::status::Status,
    policy: &RuntimePolicy,
    provider: &str,
    model: &str,
    catalog: &smith_config::catalog::CatalogSnapshot,
) {
    let details = if provider == policy.provider_name && model == policy.model.as_str() {
        Some((&policy.reasoning, &policy.context_window))
    } else {
        policy
            .fallback_models
            .iter()
            .find(|route| route.provider_name == provider && route.model.as_str() == model)
            .map(|route| (&route.reasoning, &route.context_window))
    };
    let Some((reasoning, context_window)) = details else {
        return;
    };
    if status.provider.as_deref() != Some(provider) || status.model != model {
        status.switch_model(Some(provider.to_owned()), model);
    }
    status.set_price(resolve_model_price(policy, provider, model, catalog));
    status.context_window.clone_from(context_window);
    status.set_reasoning_hint(reasoning.has_override().then(|| {
        format!(
            "think {} · effort {}",
            reasoning.effective_state(),
            reasoning.effective_effort()
        )
    }));
}

pub(super) fn bound_price(
    provider: &str,
    model: &str,
    binding: smith_config::catalog::CatalogBinding<'_>,
) -> Option<smith_client::status::PriceReference> {
    if model.starts_with("cli/") {
        return None;
    }
    let cost = binding.provider.models.get(model)?.cost.as_ref()?;
    Some(
        smith_client::status::PriceReference::from_catalog(provider, model, cost).with_provenance(
            binding.source,
            binding.revision,
            binding.retrieved_at_ms,
        ),
    )
}

pub(super) fn resolve_catalog_price(
    provider: &str,
    model: &str,
    catalog_provider: Option<&str>,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::PriceReference> {
    if model.starts_with("cli/") {
        return None;
    }
    let catalog_provider = catalog_provider?;
    if let Some(public) = catalog.provider(catalog_provider) {
        let cost = public.models.get(model)?.cost.as_ref()?;
        return Some(
            smith_client::status::PriceReference::from_catalog(provider, model, cost)
                .with_provenance("models.dev", catalog.revision(), catalog.retrieved_at_ms),
        );
    }
    if let Some(endpoint) = catalog.endpoint_catalogs.get(provider)
        && endpoint.catalog.id == catalog_provider
    {
        // The immutable inventory admitted this exact local provider binding.
        let cost = endpoint.catalog.models.get(model)?.cost.as_ref()?;
        return Some(
            smith_client::status::PriceReference::from_catalog(provider, model, cost)
                .with_provenance(
                    &endpoint.source,
                    &endpoint.revision,
                    endpoint.retrieved_at_ms,
                ),
        );
    }
    None
}

pub(super) fn resolve_inventory_price(
    provider: &str,
    model: &str,
    inventory: &SelectionInventory,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::PriceReference> {
    let entry = inventory
        .models
        .iter()
        .find(|entry| entry.provider == provider && entry.model == model)?;
    let catalog_provider = entry.catalog_provider.as_deref()?;
    if model.starts_with("cli/") {
        return None;
    }
    if let Some(source) = &entry.catalog_source {
        // Price fields were captured from the exact admitted binding along
        // with this immutable picker/host inventory. Do not resolve another
        // same-ID public or endpoint source after that decision.
        return Some(
            smith_client::status::PriceReference::from_catalog(
                provider,
                model,
                entry.catalog_cost.as_ref()?,
            )
            .with_provenance(
                source,
                entry.catalog_revision.as_deref()?,
                entry.catalog_retrieved_at_ms?,
            ),
        );
    }
    resolve_catalog_price(provider, model, Some(catalog_provider), catalog)
}

pub(super) fn resolve_child_usage_binding(
    profile: &str,
    inventory: &SelectionInventory,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::BindingUsage> {
    // The inventory is frozen with this host's config/catalog. Its catalog
    // provider already encodes the exact adapter/endpoint pairing, including
    // local aliases; rereading config at spawn could disagree with the runtime.
    let profile = inventory.profiles.iter().find(|entry| {
        entry.name == profile && entry.uses.contains(&smith_config::model::ProfileUse::Child)
    })?;
    let provider = profile.provider.as_deref()?;
    let model = profile.model.as_deref()?;
    let price = resolve_inventory_price(provider, model, inventory, catalog);
    Some(smith_client::status::BindingUsage::new(
        Some(provider.to_owned()),
        model,
        price,
    ))
}
