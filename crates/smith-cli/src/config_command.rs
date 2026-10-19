//! Configuration readiness, resolution, explain, and module listing commands.

use std::path::PathBuf;

use anyhow::{Context, Result};
use smith_config::resolve::{
    ConfigReadiness, Resolution, ResolveRequest, SyntheticCacheSpendAuthority, inspect, resolve,
};

use crate::cli::Selection;

pub(super) struct Prepared {
    pub(super) resolution: Resolution,
    pub(super) project: PathBuf,
    pub(super) catalog: std::sync::Arc<smith_config::catalog::CatalogSnapshot>,
}

pub(super) fn inspect_selection(selection: &Selection) -> Result<ConfigReadiness> {
    let (_, request) = resolution_request(selection)?;
    Ok(inspect(&request))
}

pub(super) fn prepare(selection: &Selection) -> Result<Prepared> {
    prepare_with_catalog(selection, None)
}

pub(super) fn prepare_with_catalog(
    selection: &Selection,
    catalog: Option<std::sync::Arc<smith_config::catalog::CatalogSnapshot>>,
) -> Result<Prepared> {
    let (start, request) = resolution_request_with_catalog(selection, catalog)?;
    let catalog = request
        .catalog
        .clone()
        .expect("host supplies a frozen catalog");
    let resolution = resolve(&request)
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("resolving Smith configuration")?;
    let project = resolution.layout.project_root.clone().unwrap_or(start);
    Ok(Prepared {
        resolution,
        project,
        catalog,
    })
}

pub(super) fn resolution_request(selection: &Selection) -> Result<(PathBuf, ResolveRequest)> {
    resolution_request_with_catalog(selection, None)
}

pub(super) fn resolution_request_with_catalog(
    selection: &Selection,
    catalog: Option<std::sync::Arc<smith_config::catalog::CatalogSnapshot>>,
) -> Result<(PathBuf, ResolveRequest)> {
    let start = match &selection.project {
        Some(project) => project.clone(),
        None => std::env::current_dir().context("reading the current directory")?,
    };
    let start = start
        .canonicalize()
        .with_context(|| format!("resolving project path `{}`", start.display()))?;
    if !start.is_dir() {
        anyhow::bail!("project path `{}` is not a directory", start.display());
    }

    let authority = if selection.allow_synthetic_cache_spend {
        SyntheticCacheSpendAuthority::Allow
    } else {
        SyntheticCacheSpendAuthority::Deny
    };
    let frozen_catalog = catalog.is_some();
    let mut request = ResolveRequest::new(&start)
        .with_known_modules(crate::modules::known_modules())
        .with_env(std::env::vars())
        .with_cli(selection.overrides())
        .with_session(selection.session_overrides())
        .with_synthetic_cache_spend(authority);
    if let Some(catalog) = catalog {
        request = request.with_catalog(catalog);
    }
    let mut request = crate::plugins::with_known_plugins(request)?;
    if !frozen_catalog {
        request = request_catalog(request)?;
    } else {
        request = authorize_catalog(request)?;
    }
    let request = match &selection.advisor {
        Some(choice) => request.with_advisor_override(choice.clone()),
        None => request,
    };
    Ok((start, request))
}

/// Prepare endpoint snapshots only after ordinary configuration validation.
/// Invalid project policy cannot trigger even a local credential read.
pub(super) fn request_catalog(mut request: ResolveRequest) -> Result<ResolveRequest> {
    smith_config::resolve::resolve_model_sync_request(&request)
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let user_dir = match &request.home_dir {
        Some(home) => home.join(".smith"),
        None => smith_config::credential::user_state_root()?,
    };
    request.catalog = Some(crate::model_sync::cached_snapshot(&user_dir)?);
    authorize_catalog(request)
}

/// Recheck authority for fresh and frozen request snapshots before their
/// metadata contributes to inventory or runtime resolution.
pub(super) fn authorize_catalog(mut request: ResolveRequest) -> Result<ResolveRequest> {
    let Some(snapshot) = request.catalog.clone() else {
        return Ok(request);
    };
    let next = match resolve(&request) {
        Ok(resolution) => crate::model_sync::catalog_for_resolution(&resolution, snapshot)?,
        Err(_) if snapshot.endpoint_catalogs.is_empty() => snapshot,
        Err(_) => {
            let mut snapshot = snapshot;
            std::sync::Arc::make_mut(&mut snapshot)
                .endpoint_catalogs
                .clear();
            snapshot
        }
    };
    request.catalog = Some(next);
    Ok(request)
}

pub(super) fn explain_config(key: &str, selection: &Selection) -> Result<()> {
    let prepared = prepare(selection)?;
    let provenance = &prepared.resolution.provenance;
    let canonical_key = prepared.resolution.config.modules.keys().find_map(|id| {
        let alias = smith_plugin::module_id_alias(id)?;
        (key == format!("modules.\"{alias}\".enabled")).then(|| format!("modules.\"{id}\".enabled"))
    });
    let key = canonical_key.as_deref().unwrap_or(key);
    let keys: Vec<&str> = if matches!(key, "tools.web_search" | "tools.web_fetch") {
        let prefix = format!("{key}.");
        provenance
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .collect()
    } else {
        vec![key]
    };
    let diagnostic = prepared.resolution.config.web.search.diagnostic();
    for key in keys {
        // An unset credential or URL still has an actionable explanation.
        if provenance.source(key).is_none() && diagnostic.is_some_and(|(setting, _)| setting == key)
        {
            continue;
        }
        let explanation = provenance
            .explain(key)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        println!("{} = {}", explanation.key, explanation.value);
        println!("source: {}", explanation.source);
        for entry in explanation.overridden {
            println!("overrode: {} from {}", entry.value, entry.source);
        }
    }
    if let Some((setting, message)) = diagnostic
        && (key == "tools.web_search" || key == setting || key == "tools.web_search.backend")
    {
        println!("diagnostic: {setting}: {message}");
    }
    Ok(())
}

pub(super) async fn modules_report(
    prepared: &Prepared,
) -> Result<smith_client::modules_report::ModulesReport> {
    let request = crate::runtime_host::preflight_request(
        &prepared.resolution,
        &prepared.project,
        smith_runtime::factory::HostSurface::Headless,
        Some(prepared.catalog.clone()),
    )?;
    let outcomes = smith_runtime::factory::module_report(&request).await?;
    let report =
        smith_client::modules_report::module_report(&outcomes, &prepared.resolution.config.modules);
    Ok(report)
}

pub(super) async fn list_modules(selection: &Selection) -> Result<()> {
    let prepared = prepare(selection)?;
    let report = modules_report(&prepared).await?;
    println!("{}", smith_client::modules_report::render_plain(&report));
    Ok(())
}
