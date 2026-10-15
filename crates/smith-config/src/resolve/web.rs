//! Web-tool resolution and user-owned network policy.

use std::collections::BTreeSet;
use std::path::Path;

use crate::credential::CredentialRef;
use crate::model::{WebSearchBackend, WebSearchMode};

use super::load::source_for;
use super::provenance::*;
use super::provider::{required_flag, required_text, text};
use super::types::*;

pub(super) fn defaults() -> Vec<Contribution> {
    [
        (
            "agent.read_only_network_modules",
            SettingValue::List(vec!["web".to_owned(), "vision".to_owned()]),
        ),
        (
            "tools.web_fetch.allow_private_networks",
            SettingValue::Flag(false),
        ),
        (
            "tools.web_search.mode",
            SettingValue::Text(WebSearchMode::default().as_str().to_owned()),
        ),
        (
            "tools.web_search.backend",
            SettingValue::Text(WebSearchBackend::default().as_str().to_owned()),
        ),
    ]
    .into_iter()
    .map(|(key, value)| Contribution {
        key: key.to_owned(),
        source: Source::built_in(key),
        value,
    })
    .collect()
}

/// Check the file's authority before profile lifting or secret flattening.
pub(super) fn validate_file_setting(
    key: &str,
    layer: Layer,
    path: Option<&Path>,
) -> Result<(), ConfigError> {
    const USER_ONLY: &[&str] = &[
        "agent.read_only_network_modules",
        "tools.web_fetch.allow_private_networks",
        "tools.web_search.backend",
        "tools.web_search.api_key",
        "tools.web_search.searxng_url",
    ];
    if layer != Layer::BuiltIn
        && layer != Layer::UserFile
        && USER_ONLY.iter().any(|setting| {
            key == *setting
                || key
                    .strip_suffix(setting)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
    {
        return Err(ConfigError::InvalidValue {
            source: source_for(layer, path, key),
            message: "network authority and backend settings are user-scoped; repository configuration cannot self-authorize or redirect network requests".to_owned(),
        });
    }
    Ok(())
}

pub(super) fn credential(value: &str, source: &Source) -> Result<CredentialRef, ConfigError> {
    CredentialRef::parse(value).map_err(|error| ConfigError::InvalidValue {
        source: source.clone(),
        message: error.to_string(),
    })
}

pub(super) fn resolve_web(provenance: &Provenance) -> Result<ResolvedWeb, ConfigError> {
    let mode = required_text(provenance, "tools.web_search.mode")?;
    let mode = Sourced::new(
        WebSearchMode::parse(&mode.value).ok_or_else(|| ConfigError::InvalidValue {
            source: mode.source.clone(),
            message: "web search mode must be auto, hosted, backend, or off".to_owned(),
        })?,
        mode.source,
    );
    let backend = required_text(provenance, "tools.web_search.backend")?;
    let backend = Sourced::new(
        WebSearchBackend::parse(&backend.value).ok_or_else(|| ConfigError::InvalidValue {
            source: backend.source.clone(),
            message: "web search backend must be duckduckgo, brave, tavily, or searxng".to_owned(),
        })?,
        backend.source,
    );
    let api_key = text(provenance, "tools.web_search.api_key")?
        .map(|value| {
            credential(&value.value, &value.source)
                .map(|reference| Sourced::new(reference, value.source))
        })
        .transpose()?;
    let searxng_url = text(provenance, "tools.web_search.searxng_url")?;
    if let Some(value) = &searxng_url {
        let valid = url::Url::parse(&value.value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        });
        if !valid {
            return Err(ConfigError::InvalidValue {
                source: value.source.clone(),
                message: "SearXNG URL must be an absolute http(s) URL without userinfo".to_owned(),
            });
        }
    }
    Ok(ResolvedWeb {
        fetch: ResolvedWebFetch {
            allow_private_networks: required_flag(
                provenance,
                "tools.web_fetch.allow_private_networks",
            )?,
        },
        search: ResolvedWebSearch {
            mode,
            backend,
            api_key,
            searxng_url,
        },
    })
}

pub(super) fn validate_read_only_network_modules(
    modules: &Sourced<Vec<String>>,
    known: &[KnownModule],
) -> Result<(), ConfigError> {
    // Include native ids even when the host omits its optional module catalog.
    let known_ids: BTreeSet<&str> = ["budget-notice", "image-generation", "vision", "web"]
        .into_iter()
        .chain(known.iter().map(|module| module.id.as_str()))
        .collect();
    for id in &modules.value {
        if !known_ids.contains(id.as_str()) {
            return Err(ConfigError::UnknownModule {
                id: id.clone(),
                source: modules.source.clone(),
                known_ids: known_ids.iter().map(|id| (*id).to_owned()).collect(),
            });
        }
    }
    Ok(())
}
