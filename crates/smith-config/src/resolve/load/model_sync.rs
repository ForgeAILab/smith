//! User-owned model synchronization policy and endpoint discovery authority.

use std::collections::BTreeMap;
use std::path::Path;

use crate::model::KIND_OPENAI_COMPATIBLE;
use crate::resolve::provenance::{CONFIG_FILE, Layer, Provenance, SettingValue, Source, Sourced};
use crate::resolve::provider::{nearest, required_flag, required_u32, resolve_provider};
use crate::resolve::types::{
    ConfigError, ReferenceKind, Resolution, ResolveRequest, ResolvedModelSync, ResolvedProvider,
};

use super::{Declarations, built_in_defaults, contributions_of, discover, env_contributions, load};

/// Resolves one provider from user-owned declarations without selecting a model.
///
/// This credential-free path lets background discovery retain user authority
/// even when the active project or profile selects a different deployment.
pub fn resolve_user_provider(user_dir: &Path, name: &str) -> Result<ResolvedProvider, ConfigError> {
    let (provenance, declared) = user_provenance(user_dir)?;
    let declaration =
        declared
            .providers
            .get(name)
            .ok_or_else(|| ConfigError::UnusableReference {
                source: Source::built_in("model synchronization provider selection"),
                what: ReferenceKind::Provider,
                name: name.to_owned(),
                suggestions: nearest(name, declared.providers.keys().map(String::as_str)),
            })?;
    resolve_provider(
        &provenance,
        Sourced::new(name.to_owned(), declaration.clone()),
    )
}

/// Whether effective serving authority matches this user's discovery authority.
///
/// Project layers may select or override providers for serving. Their account,
/// tenant headers, or credentials must never inherit metadata fetched under a
/// different user-owned binding merely because the local provider name and URL
/// match. This compares typed field winners without resolving any credential.
pub fn endpoint_discovery_authorized(
    resolution: &Resolution,
    provider_name: &str,
) -> Result<bool, ConfigError> {
    let (user_provenance, declared) = user_provenance(&resolution.layout.user_dir)?;
    let Some(declaration) = declared.providers.get(provider_name) else {
        return Ok(false);
    };
    let authorized = resolve_provider(
        &user_provenance,
        Sourced::new(provider_name.to_owned(), declaration.clone()),
    )?;
    let effective = if resolution.config.provider.name.value == provider_name {
        resolution.config.provider.clone()
    } else {
        resolve_provider(
            &resolution.provenance,
            Sourced::new(
                provider_name.to_owned(),
                Source::built_in("endpoint metadata authority comparison"),
            ),
        )?
    };
    Ok(same_endpoint_authority(&authorized, &effective))
}

fn same_endpoint_authority(authorized: &ResolvedProvider, effective: &ResolvedProvider) -> bool {
    let endpoint = |provider: &ResolvedProvider| {
        provider
            .base_url
            .as_ref()
            .and_then(|value| crate::catalog::normalized_endpoint(&value.value))
    };
    let authorized_endpoint = endpoint(authorized);
    authorized.kind.value == KIND_OPENAI_COMPATIBLE
        && effective.kind.value == authorized.kind.value
        && authorized_endpoint.is_some()
        && endpoint(effective) == authorized_endpoint
        && authorized
            .credentials
            .iter()
            .map(|credential| &credential.value)
            .eq(effective
                .credentials
                .iter()
                .map(|credential| &credential.value))
        && authorized.api_key.as_ref().map(|key| &key.value)
            == effective.api_key.as_ref().map(|key| &key.value)
        && provider_headers(authorized)
            .zip(provider_headers(effective))
            .is_some_and(|(authorized, effective)| authorized == effective)
}

/// HTTP header names are case-insensitive. Duplicate spellings remain
/// ambiguous and cannot establish equivalent discovery authority.
fn provider_headers(provider: &ResolvedProvider) -> Option<BTreeMap<String, &str>> {
    let mut headers = BTreeMap::new();
    for (name, value) in &provider.headers {
        if headers
            .insert(name.to_ascii_lowercase(), value.value.as_str())
            .is_some()
        {
            return None;
        }
    }
    Some(headers)
}

/// Resolves the user's synchronization policy without requiring an active model.
pub fn resolve_user_model_sync(user_dir: &Path) -> Result<ResolvedModelSync, ConfigError> {
    let (provenance, _) = user_provenance(user_dir)?;
    resolve_model_sync(&provenance)
}

/// Validates synchronization authority before loading authenticated caches.
///
/// Model/profile selection may need those caches to become usable, so this
/// independent pass reads and validates declarations without selecting a run
/// or resolving any provider credential.
pub fn resolve_model_sync_request(
    request: &ResolveRequest,
) -> Result<ResolvedModelSync, ConfigError> {
    let layout = discover(request)?;
    let mut provenance = Provenance::default();
    provenance.extend(contributions_of(
        &built_in_defaults(&layout.user_dir),
        Layer::BuiltIn,
        None,
    )?);
    for file in &layout.files {
        provenance.extend(load(&file.path, file.layer)?.contributions);
    }
    provenance.extend(env_contributions(
        &request.env,
        &request.known_modules,
        &request.known_plugins,
    )?);
    validate_model_sync_policy(&provenance)?;
    resolve_model_sync(&provenance)
}

fn user_provenance(user_dir: &Path) -> Result<(Provenance, Declarations), ConfigError> {
    let mut provenance = Provenance::default();
    let mut declared = Declarations::default();
    let defaults = built_in_defaults(user_dir);
    declared.absorb(&defaults, Layer::BuiltIn, None);
    provenance.extend(contributions_of(&defaults, Layer::BuiltIn, None)?);
    let path = user_dir.join(CONFIG_FILE);
    if path.try_exists().map_err(|error| ConfigError::Unreadable {
        path: path.clone(),
        message: error.to_string(),
    })? {
        let loaded = load(&path, Layer::UserFile)?;
        declared.absorb(&loaded.file, Layer::UserFile, Some(&path));
        provenance.extend(loaded.contributions);
    }
    validate_model_sync_policy(&provenance)?;
    Ok((provenance, declared))
}

pub(in crate::resolve) fn resolve_model_sync(
    provenance: &Provenance,
) -> Result<ResolvedModelSync, ConfigError> {
    Ok(ResolvedModelSync {
        enabled: required_flag(provenance, "model_sync.enabled")?,
        // Every declaration was range-checked before extracting winners.
        interval_hours: required_u32(provenance, "model_sync.interval_hours")?,
    })
}

/// Check every declaration, including values hidden by a later contribution.
pub(super) fn validate_model_sync_policy(provenance: &Provenance) -> Result<(), ConfigError> {
    for key in ["model_sync.enabled", "model_sync.interval_hours"] {
        let explanation = provenance.explain(key)?;
        for entry in std::iter::once(crate::resolve::Entry {
            value: explanation.value,
            source: explanation.source,
        })
        .chain(explanation.overridden)
        {
            if !matches!(entry.source.layer, Layer::BuiltIn | Layer::UserFile) {
                return Err(ConfigError::InvalidValue {
                    source: entry.source,
                    message: "model synchronization policy is user-only; configure it in `~/.smith/config.toml`"
                        .to_owned(),
                });
            }
            if key == "model_sync.interval_hours"
                && !matches!(entry.value, SettingValue::Integer(1..=168))
            {
                return Err(ConfigError::InvalidValue {
                    source: entry.source,
                    message: "`model_sync.interval_hours` must be an integer from 1 through 168"
                        .to_owned(),
                });
            }
        }
    }
    Ok(())
}
