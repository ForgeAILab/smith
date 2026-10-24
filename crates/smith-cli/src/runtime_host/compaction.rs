//! Compaction route selection and persistent setting preflight.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use smith_config::resolve::resolve;
use smith_runtime::factory::{HostSurface, RuntimeRequest, VisionProfileRequest};
use smith_runtime::host::HostSession;
use smith_runtime::model_catalog::runtime_catalog_source;

use super::preflight_request;
use crate::cli::Selection;

/// `env` is the environment the running session was resolved from. Reading
/// the process environment here instead would let an injected resolution and
/// this preflight disagree, for example on an unrelated `SMITH_*` variable.
pub(crate) fn prepare_compaction_change<'a>(
    host: &'a HostSession,
    project: &'a std::path::Path,
    selection: &'a Selection,
    env: BTreeMap<String, String>,
    catalog: Arc<smith_config::catalog::CatalogSnapshot>,
    action: smith_client::commands::CompactAction,
) -> futures_util::future::BoxFuture<
    'a,
    Result<(smith_config::user_config::CommittedConfigEdit, String)>,
> {
    Box::pin(prepare_compaction_change_inner(
        host, project, selection, env, catalog, action,
    ))
}

async fn prepare_compaction_change_inner(
    host: &HostSession,
    project: &std::path::Path,
    selection: &Selection,
    env: BTreeMap<String, String>,
    catalog: Arc<smith_config::catalog::CatalogSnapshot>,
    action: smith_client::commands::CompactAction,
) -> Result<(smith_config::user_config::CommittedConfigEdit, String)> {
    use smith_client::commands::CompactAction;
    use smith_config::user_config::{CompactionSetting, prepare_compaction_edit};
    let setting = match action {
        CompactAction::Model(value) => CompactionSetting::Model(value),
        CompactAction::Instructions(value) => CompactionSetting::Instructions(value),
        CompactAction::Idle(milliseconds) => CompactionSetting::Idle {
            model: format!(
                "{}/{}",
                host.runtime().policy().provider_name,
                host.runtime().policy().model
            ),
            milliseconds,
        },
        CompactAction::Run(_) => anyhow::bail!("an attempt does not change configuration"),
    };
    let edit = prepare_compaction_edit(&host.runtime().policy().user_dir, &setting)?;
    let mut request = smith_config::resolve::ResolveRequest::new(project)
        .with_known_modules(crate::modules::known_modules())
        .with_env(env)
        .with_cli(selection.overrides())
        .with_session(selection.session_overrides())
        .with_catalog(catalog.clone());
    if let Some(home) = host.runtime().policy().user_dir.parent() {
        request = request.with_home_dir(home);
    }
    request.synthetic_cache_spend = if selection.allow_synthetic_cache_spend {
        smith_config::resolve::SyntheticCacheSpendAuthority::Allow
    } else {
        smith_config::resolve::SyntheticCacheSpendAuthority::Deny
    };
    request.advisor_override = selection.advisor.clone();
    let mut request = crate::plugins::with_known_plugins(request)?;
    request.user_config_override = Some(Box::new(edit.candidate_config()?));
    let resolution = resolve(&request).map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut runtime =
        preflight_request(&resolution, project, HostSurface::Terminal, Some(catalog))?;
    runtime.approval = Some(Arc::new(agent_runtime_core::approval::DenyAll));
    runtime.artifact_store = host.runtime().artifact_store().cloned();
    if runtime.artifact_store.is_some() {
        let mut summary = smith_runtime::summary::SmithSemanticSummaryConfig::standard();
        summary.instructions = Arc::new(smith_runtime::summary::SummaryInstructions::new(
            resolution
                .config
                .context
                .compaction_instructions
                .as_ref()
                .map(|value| value.value.as_str()),
        ));
        runtime.semantic_summary = Some(summary);
    } else if let Some(binding) = runtime.compaction_profile.take() {
        // Ephemeral sessions still validate a saved target without creating
        // summary storage or dispatching a model request.
        runtime = RuntimeRequest {
            credentials: runtime.credentials.clone(),
            model_catalog: runtime.model_catalog.clone(),
            catalog_sources: binding.catalog_sources,
            ..RuntimeRequest::new(binding.config, HostSurface::Child)
        };
    }
    let harness =
        smith_runtime::harness::resolve(smith_runtime::harness::HarnessSpec::trusted(runtime))
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    let _preflight = Box::pin(smith_runtime::factory::build(harness))
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let context = &resolution.config.context;
    let notice = format!(
        "compaction defaults saved · model {} · idle {}ms · source {}",
        context
            .compaction_model
            .as_ref()
            .map_or("conversation model", |value| value.value.as_str()),
        context.cache.inactivity_limit_ms.value,
        context.cache.inactivity_limit_ms.source
    );
    Ok((edit.commit(true)?, notice))
}

pub(super) fn resolve_compaction_profile(
    config: &smith_config::resolve::ResolvedConfig,
    mut request: smith_config::resolve::ResolveRequest,
) -> Result<Option<Box<VisionProfileRequest>>> {
    use smith_config::resolve::{AdvisorTarget, Sourced};
    let Some(selected) = &config.context.compaction_model else {
        return Ok(None);
    };
    let target = AdvisorTarget::parse(&selected.value)
        .ok_or_else(|| anyhow::anyhow!("invalid compaction model target"))?;
    if matches!(target, AdvisorTarget::InstalledAgent { .. }) {
        anyhow::bail!("compaction requires an API model, not an installed agent");
    }
    request.cli = Default::default();
    request.session = Default::default();
    request.advisor_override = None;
    let resolution = resolve(
        &request
            .clone()
            .with_advisor_route(Sourced::new(target, selected.source.clone())),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut catalog_sources = Vec::new();
    if let Some(catalog) = request.catalog.as_ref()
        && let Some(source) = runtime_catalog_source(
            catalog,
            &resolution.config.provider.name.value,
            &resolution.config.provider.kind.value,
            resolution
                .config
                .provider
                .base_url
                .as_ref()
                .map(|url| url.value.as_str()),
        )
    {
        catalog_sources.push(source);
    }
    Ok(Some(Box::new(VisionProfileRequest {
        config: resolution.config,
        catalog_sources,
        provider: None,
    })))
}
