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
    record.schema_version == 5
        && !record.bindings.is_empty()
        && record.bindings.iter().all(|binding| {
            binding
                .provider
                .as_deref()
                .is_some_and(|provider| !provider.trim().is_empty())
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

/// A matching v5 log preserves model switches that identity-free runtime
/// records cannot recover. Stale or older logs use the single-manifest rule
/// rather than assigning all earlier usage to the last model.
pub(super) fn restore_usage_with_bindings<'a>(
    status: &mut smith_client::status::Status,
    records: &[UsageRecord],
    logged: Option<&smith_client::usage_log::SessionUsageRecord>,
    manifests: impl IntoIterator<Item = (&'a str, &'a str)>,
    mut price_for: impl FnMut(&str, &str) -> Option<smith_client::status::PriceReference>,
) {
    if records.is_empty() {
        return;
    }
    let provider = status.provider.clone();
    let model = status.model.clone();
    let price = status.price().cloned();
    let bindings = manifests
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if bindings.len() == 1
        && let Some((provider, model)) = bindings.iter().next()
    {
        status.switch_model(Some((*provider).to_owned()), *model);
        status.set_price(price_for(provider, model));
    } else {
        status.switch_model(None, "earlier models");
    }
    restore_usage_records(status, records);
    if let Some(logged) = logged {
        restore_logged_bindings(status, logged, &mut price_for);
    }
    if status.provider != provider || status.model != model {
        status.switch_model(provider, model);
        status.set_price(price);
    }
}

/// Checks the root rollup and the binding partition before using the log:
/// v4's synthesized last-model bucket is not evidence of earlier attribution.
fn restore_logged_bindings(
    status: &mut smith_client::status::Status,
    logged: &smith_client::usage_log::SessionUsageRecord,
    price_for: &mut impl FnMut(&str, &str) -> Option<smith_client::status::PriceReference>,
) {
    use smith_client::status::{BindingUsage, counter_label};
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
    let kinds = [
        CounterKind::InputUncached,
        CounterKind::InputCached,
        CounterKind::CacheWrite,
        CounterKind::Output,
        CounterKind::Reasoning,
    ];
    let mut bindings = Vec::new();
    for entry in &logged.bindings {
        let mut binding = BindingUsage::new(entry.provider.clone(), &entry.model, None);
        for (label, value) in &entry.totals {
            let Some(kind) = kinds.iter().find(|kind| counter_label(**kind) == label) else {
                return;
            };
            binding.totals.insert(*kind, *value);
        }
        binding.reported = logged.reported && restored.bindings.iter().all(|entry| entry.reported);
        bindings.push(binding);
    }
    for binding in &mut bindings {
        binding.price = binding
            .provider
            .as_deref()
            .and_then(|provider| price_for(provider, &binding.model));
    }
    restored.bindings = bindings;
    status.retain_usage_bindings(&restored);
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
    let catalog_provider = smith_config::catalog::catalog_provider_for(
        &policy.provider_kind,
        policy.endpoint.as_deref(),
    );
    resolve_catalog_price(
        &policy.provider_name,
        policy.model.as_str(),
        catalog_provider,
        catalog,
    )
}

pub(super) fn resolve_catalog_price(
    provider: &str,
    model: &str,
    catalog_provider: Option<&str>,
    catalog: &smith_config::catalog::CatalogSnapshot,
) -> Option<smith_client::status::PriceReference> {
    let cost = catalog_provider
        .and_then(|provider| catalog.provider(provider))
        .and_then(|provider| provider.models.get(model))
        .and_then(|model| model.cost.as_ref())?;
    Some(smith_client::status::PriceReference::from_catalog(
        provider, model, cost,
    ))
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
    let catalog_provider = inventory
        .models
        .iter()
        .find(|entry| entry.provider == provider && entry.model == model)
        .and_then(|entry| entry.catalog_provider.as_deref());
    let price = resolve_catalog_price(provider, model, catalog_provider, catalog);
    Some(smith_client::status::BindingUsage::new(
        Some(provider.to_owned()),
        model,
        price,
    ))
}
