//! Declarative, credential-free runtime selection inventory.
//!
//! This module reads only the files already named by a successful
//! [`crate::resolve::Resolution`]. It never resolves a credential and has no
//! network client. Provider/model identities stay paired all the way through
//! selection so a model from one provider cannot be applied to another.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::catalog::{CatalogModel, CatalogSnapshot, catalog_provider_for};
use crate::credential::CredentialRef;
#[cfg(test)]
use crate::model::ReasoningOnlyBehavior;
use crate::model::{
    AgentPosture, ConfigFile, KIND_CHATGPT_RESPONSES, KIND_COMMAND_JSONL, KIND_OPENAI_COMPATIBLE,
    KIND_XAI_RESPONSES, ModelSection, ProfileUse, ProviderResponseSection, ProviderSection,
};
use crate::resolve::{ConfigError, Position, Resolution, Source};
use crate::setup::trusted_model;

/// A locally configured profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileInventoryEntry {
    /// Profile name.
    pub name: String,
    /// Effective provider when the profile declares one.
    pub provider: Option<String>,
    /// Effective model when the profile declares one.
    pub model: Option<String>,
    /// Authority-narrowing behavior selected by the profile.
    pub posture: AgentPosture,
    /// Bounded user-facing description.
    pub description: Option<String>,
    /// Main/child placements where this profile is eligible.
    pub uses: Vec<ProfileUse>,
    /// Deterministic effective profile revision.
    pub revision: String,
    /// Whether this entry came from a one-release legacy adapter.
    pub legacy: bool,
    /// Whether the provider/model pair is selectable.
    pub selectable: bool,
    /// Whether this is the active profile.
    pub active: bool,
    /// Where the profile's provider was selected.
    pub source: Option<Source>,
}

impl ProfileInventoryEntry {
    /// Provider-qualified model label when complete.
    pub fn pair(&self) -> Option<String> {
        Some(format!(
            "{}/{}",
            self.provider.as_deref()?,
            self.model.as_deref()?
        ))
    }
}

/// A locally configured provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInventoryEntry {
    /// Provider name.
    pub name: String,
    /// Declared adapter kind, when present.
    pub kind: Option<String>,
    /// Whether the declaration is valid for an adapter in this build.
    pub adapter_available: bool,
    /// Whether at least one complete model makes it runnable immediately.
    pub selectable: bool,
    /// Number of selectable models scoped to this provider.
    pub model_count: usize,
    /// Whether this is the active provider.
    pub active: bool,
    /// Where its adapter kind was declared.
    pub source: Option<Source>,
}

/// Where one model limit comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelLimitOrigin {
    /// Explicit Smith configuration.
    Configured(Source),
    /// Versioned Smith-owned trusted data.
    Trusted {
        /// Stable catalog name.
        catalog: String,
        /// Catalog revision.
        revision: u32,
    },
    /// A schema-validated Models.dev snapshot.
    Catalog {
        /// Stable public catalog name.
        catalog: String,
        /// Frozen source revision.
        revision: String,
        /// Retrieval time in Unix milliseconds.
        retrieved_at_ms: u64,
    },
}

/// One enforceable model limit and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryLimit {
    /// Token value.
    pub value: u32,
    /// Why Smith trusts it.
    pub origin: ModelLimitOrigin,
}

/// A provider-qualified model that can pass model-profile preflight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInventoryEntry {
    /// Provider identity.
    pub provider: String,
    /// Provider model ID.
    pub model: String,
    /// Human-readable catalog display name.
    pub label: String,
    /// Total context window.
    pub context_tokens: Option<InventoryLimit>,
    /// Maximum enforceable input.
    pub max_input_tokens: Option<InventoryLimit>,
    /// Maximum model output.
    pub max_output_tokens: Option<InventoryLimit>,
    /// Whether the catalog advertises tool calling, when catalog-backed.
    pub tool_call: Option<bool>,
    /// Whether the catalog advertises reasoning, when catalog-backed.
    pub reasoning: Option<bool>,
    /// Whether the catalog advertises structured output, when catalog-backed.
    pub structured_output: Option<bool>,
    /// Models.dev provider identity, when catalog-backed.
    pub catalog_provider: Option<String>,
    /// Frozen catalog revision, when catalog-backed.
    pub catalog_revision: Option<String>,
    /// Catalog retrieval time in Unix milliseconds, when catalog-backed.
    pub catalog_retrieved_at_ms: Option<u64>,
    /// Profiles that point at this pair.
    pub profiles: Vec<String>,
    /// Whether this entry can be applied.
    pub selectable: bool,
    /// Why a visible entry cannot be selected.
    pub disabled_reason: Option<String>,
    /// Whether this exact pair is active.
    pub active: bool,
}

impl ModelInventoryEntry {
    /// Stable provider-qualified identity.
    pub fn id(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

/// Every local runtime selection surface needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionInventory {
    /// Profiles, sorted by name.
    pub profiles: Vec<ProfileInventoryEntry>,
    /// Providers, sorted by name.
    pub providers: Vec<ProviderInventoryEntry>,
    /// Valid provider/model pairs, sorted by provider then model.
    pub models: Vec<ModelInventoryEntry>,
}

impl SelectionInventory {
    /// Models scoped to `provider`.
    pub fn models_for(&self, provider: &str) -> Vec<&ModelInventoryEntry> {
        self.models
            .iter()
            .filter(|entry| entry.provider == provider)
            .collect()
    }

    /// Resolves a provider-qualified or unique unqualified model choice.
    ///
    /// An unqualified model in the active provider wins only when it exists
    /// there; otherwise global uniqueness is required.
    pub fn resolve_model(
        &self,
        value: &str,
        active_provider: Option<&str>,
    ) -> Result<&ModelInventoryEntry, ModelSelectionError> {
        if let Some(exact) = self
            .models
            .iter()
            .find(|entry| entry.selectable && entry.id() == value)
        {
            return Ok(exact);
        }
        if let Some(provider) = active_provider
            && let Some(local) = self.models.iter().find(|entry| {
                entry.selectable && entry.provider == provider && entry.model == value
            })
        {
            return Ok(local);
        }
        let matches: Vec<&ModelInventoryEntry> = self
            .models
            .iter()
            .filter(|entry| entry.selectable && entry.model == value)
            .collect();
        match matches.as_slice() {
            [only] => Ok(*only),
            [] => Err(ModelSelectionError::Unknown {
                value: value.to_owned(),
            }),
            many => Err(ModelSelectionError::Ambiguous {
                value: value.to_owned(),
                choices: many.iter().map(|entry| entry.id()).collect(),
            }),
        }
    }
}

/// Why a direct model shortcut cannot resolve locally.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelSelectionError {
    /// No selectable pair has this identity.
    #[error("model `{value}` is not locally selectable; run `smith setup add-model`")]
    Unknown {
        /// Supplied value.
        value: String,
    },
    /// Several providers serve the same unqualified model.
    #[error(
        "model `{value}` is ambiguous; choose a provider-qualified model from: {}",
        choices.join(", ")
    )]
    Ambiguous {
        /// Supplied value.
        value: String,
        /// Provider-qualified choices.
        choices: Vec<String>,
    },
}

/// Builds a deterministic local inventory from an already-ready resolution.
pub fn local_inventory(
    resolution: &Resolution,
    available_adapter_kinds: &[&str],
) -> Result<SelectionInventory, ConfigError> {
    local_inventory_with_catalog(resolution, available_adapter_kinds, None)
}

/// Builds a deterministic local inventory augmented by one frozen snapshot.
///
/// This remains an I/O-free operation with respect to catalogs: the host has
/// already loaded and validated `catalog`, and no provider or credential is
/// consulted here.
pub fn local_inventory_with_catalog(
    resolution: &Resolution,
    available_adapter_kinds: &[&str],
    catalog: Option<&CatalogSnapshot>,
) -> Result<SelectionInventory, ConfigError> {
    let mut providers = BTreeMap::<String, ProviderSection>::new();
    let mut models = BTreeMap::<String, ModelSection>::new();

    for loaded in &resolution.layout.files {
        let text = fs::read_to_string(&loaded.path).map_err(|error| ConfigError::Unreadable {
            path: loaded.path.clone(),
            message: error.to_string(),
        })?;
        let file = ConfigFile::parse(&text).map_err(|error| ConfigError::Malformed {
            path: loaded.path.clone(),
            location: error.span().map(|span| position(&text, span.start)),
            message: error.message().to_owned(),
        })?;
        for (name, section) in file.providers {
            merge_provider(providers.entry(name).or_default(), section);
        }
        for (name, section) in file.models {
            merge_model(models.entry(name).or_default(), section);
        }
    }

    let available: BTreeSet<&str> = available_adapter_kinds.iter().copied().collect();
    let provider_valid: BTreeMap<String, bool> = providers
        .iter()
        .map(|(name, section)| (name.clone(), provider_is_selectable(section, &available)))
        .collect();

    let mut candidate_pairs: BTreeSet<(String, String)> = models
        .keys()
        .filter_map(|identity| {
            let (provider, model) = identity.split_once('/')?;
            Some((provider.to_owned(), model.to_owned()))
        })
        .collect();
    for profile in resolution.config.agent.profiles.values() {
        if let (Some(provider), Some(model)) = (&profile.provider, &profile.model) {
            candidate_pairs.insert((provider.value.clone(), model.value.clone()));
        }
    }

    let mut catalog_models = BTreeMap::<(String, String), (&str, &CatalogModel)>::new();
    if let Some(snapshot) = catalog {
        for (provider, section) in &providers {
            if !provider_valid.get(provider).copied().unwrap_or(false) {
                continue;
            }
            let catalog_provider = catalog_provider_for(
                section.kind.as_deref().unwrap_or_default(),
                section.base_url.as_deref(),
            )
            .or_else(|| {
                (section.kind.as_deref() == Some(crate::model::KIND_GEMINI_INTERACTIONS))
                    .then_some(crate::catalog::GOOGLE_CATALOG_PROVIDER)
            });
            let Some(catalog_provider) = catalog_provider else {
                continue;
            };
            let Some(provider_catalog) = snapshot.provider(catalog_provider) else {
                continue;
            };
            for (model, entry) in &provider_catalog.models {
                candidate_pairs.insert((provider.clone(), model.clone()));
                catalog_models.insert((provider.clone(), model.clone()), (catalog_provider, entry));
            }
        }
    }

    let active_provider = &resolution.config.provider.name.value;
    let active_model = &resolution.config.model.value;
    let mut model_entries = Vec::new();
    for (provider, model) in candidate_pairs {
        if !provider_valid.get(&provider).copied().unwrap_or(false) {
            continue;
        }
        let identity = format!("{provider}/{model}");
        let explicit = models.get(&identity);
        let trusted = trusted_model(&provider, &model);
        let catalog_entry = catalog_models.get(&(provider.clone(), model.clone()));
        let catalog_model = catalog_entry.map(|(_, entry)| *entry);
        let catalog_limits = catalog_model.and_then(|entry| entry.limits);
        let context_tokens = inventory_limit(
            resolution,
            &identity,
            "context_tokens",
            explicit.and_then(|section| section.context_tokens),
            trusted.map(|record| record.context_tokens),
            trusted.map(|record| (record.catalog, record.revision)),
            CatalogLimit {
                value: catalog_limits.map(|limits| limits.context_tokens),
                snapshot: catalog,
            },
        );
        let max_input_tokens = inventory_limit(
            resolution,
            &identity,
            "max_input_tokens",
            explicit.and_then(|section| section.max_input_tokens),
            trusted.map(|record| record.max_input_tokens),
            trusted.map(|record| (record.catalog, record.revision)),
            CatalogLimit {
                value: catalog_limits.map(|limits| limits.max_input_tokens),
                snapshot: catalog,
            },
        );
        let max_output_tokens = inventory_limit(
            resolution,
            &identity,
            "max_output_tokens",
            explicit.and_then(|section| section.max_output_tokens),
            trusted.map(|record| record.max_output_tokens),
            trusted.map(|record| (record.catalog, record.revision)),
            CatalogLimit {
                value: catalog_limits.map(|limits| limits.max_output_tokens),
                snapshot: catalog,
            },
        );
        if catalog_model.is_none()
            && (context_tokens.is_none()
                || max_input_tokens.is_none()
                || max_output_tokens.is_none())
        {
            continue;
        }
        let mut disabled_reason = catalog_model.and_then(|entry| entry.disabled_reason.clone());
        if disabled_reason.is_none() && catalog_model.is_some_and(|entry| !entry.has_text_output())
        {
            disabled_reason = Some("catalog model does not produce text".to_owned());
        }
        if disabled_reason.is_none() && catalog_model.is_some_and(|entry| !entry.tool_call) {
            disabled_reason = Some("catalog model does not support tool calling".to_owned());
        }
        if disabled_reason.is_none()
            && (context_tokens.is_none()
                || max_input_tokens.is_none()
                || max_output_tokens.is_none())
        {
            disabled_reason =
                Some("catalog model has incomplete or invalid enforceable limits".to_owned());
        }
        if disabled_reason.is_none()
            && let (Some(context), Some(input), Some(output)) =
                (&context_tokens, &max_input_tokens, &max_output_tokens)
            && (input.value > context.value || output.value > context.value)
        {
            disabled_reason =
                Some("effective model input or output limit exceeds its context window".to_owned());
        }
        if disabled_reason.is_none()
            && let (Some(context), Some(output)) = (&context_tokens, &max_output_tokens)
        {
            let output_reserve = resolution
                .config
                .context
                .output_reserve
                .as_ref()
                .or(resolution.config.max_output_tokens.as_ref())
                .map_or(output.value, |sourced| sourced.value);
            let reasoning_reserve = resolution.config.context.reasoning_reserve.value;
            if output_reserve.saturating_add(reasoning_reserve) >= context.value {
                disabled_reason = Some(format!(
                    "output reserve {output_reserve} + reasoning reserve {reasoning_reserve} \
                     leaves no input budget"
                ));
            }
        }
        let selectable = disabled_reason.is_none()
            && context_tokens.is_some()
            && max_input_tokens.is_some()
            && max_output_tokens.is_some();
        let associated_profiles = resolution
            .config
            .agent
            .profiles
            .iter()
            .filter(|(_, profile)| {
                profile.provider.as_ref().map(|value| value.value.as_str())
                    == Some(provider.as_str())
                    && profile.model.as_ref().map(|value| value.value.as_str())
                        == Some(model.as_str())
            })
            .map(|(name, _)| name.clone())
            .collect();
        model_entries.push(ModelInventoryEntry {
            active: provider == *active_provider && model == *active_model,
            label: catalog_model
                .map(|entry| entry.name.clone())
                .unwrap_or_else(|| model.clone()),
            tool_call: catalog_model.map(|entry| entry.tool_call),
            reasoning: catalog_model.map(|entry| entry.reasoning),
            structured_output: catalog_model.map(|entry| entry.structured_output),
            catalog_provider: catalog_entry
                .map(|(catalog_provider, _)| (*catalog_provider).to_owned()),
            catalog_revision: catalog_entry
                .and(catalog.map(|snapshot| snapshot.revision().to_owned())),
            catalog_retrieved_at_ms: catalog_entry
                .and(catalog.map(|snapshot| snapshot.retrieved_at_ms)),
            provider,
            model,
            context_tokens,
            max_input_tokens,
            max_output_tokens,
            profiles: associated_profiles,
            selectable,
            disabled_reason,
        });
    }

    let model_ids: BTreeSet<String> = model_entries
        .iter()
        .filter(|model| model.selectable)
        .map(ModelInventoryEntry::id)
        .collect();
    let profile_entries = resolution
        .config
        .agent
        .profiles
        .iter()
        .map(|(name, profile)| {
            let provider = profile
                .provider
                .as_ref()
                .map(|value| value.value.clone())
                .or_else(|| profile.legacy.then(|| active_provider.clone()));
            let model = profile
                .model
                .as_ref()
                .map(|value| value.value.clone())
                .or_else(|| profile.legacy.then(|| active_model.clone()));
            let pair = provider
                .as_ref()
                .zip(model.as_ref())
                .map(|(provider, model)| format!("{provider}/{model}"));
            ProfileInventoryEntry {
                name: name.clone(),
                provider,
                model,
                posture: profile.posture.value,
                description: profile
                    .description
                    .as_ref()
                    .map(|value| value.value.clone()),
                uses: profile.uses.value.clone(),
                revision: profile.revision.clone(),
                legacy: profile.legacy,
                selectable: pair.as_ref().is_some_and(|pair| model_ids.contains(pair)),
                active: resolution.config.agent.profile.name == *name,
                source: Some(profile.posture.source.clone()),
            }
        })
        .collect::<Vec<_>>();

    let provider_entries = providers
        .iter()
        .map(|(name, section)| ProviderInventoryEntry {
            name: name.clone(),
            kind: section.kind.clone(),
            adapter_available: provider_valid.get(name).copied().unwrap_or(false),
            selectable: provider_valid.get(name).copied().unwrap_or(false)
                && model_entries
                    .iter()
                    .any(|model| model.provider == *name && model.selectable),
            model_count: model_entries
                .iter()
                .filter(|model| model.provider == *name && model.selectable)
                .count(),
            active: *name == *active_provider,
            source: resolution
                .provenance
                .source(&format!("providers.{}.kind", quote(name)))
                .cloned(),
        })
        .collect::<Vec<_>>();

    Ok(SelectionInventory {
        profiles: profile_entries,
        providers: provider_entries,
        models: model_entries,
    })
}

fn provider_is_selectable(section: &ProviderSection, available: &BTreeSet<&str>) -> bool {
    let Some(kind) = section.kind.as_deref() else {
        return false;
    };
    if !available.contains(kind)
        || kind == KIND_COMMAND_JSONL
            && section.command.as_ref().is_none_or(|command| {
                command.executable.is_empty()
                    || !Path::new(&command.executable).is_absolute()
                    || command
                        .cwd
                        .as_deref()
                        .is_some_and(|cwd| cwd != "workspace" && !Path::new(cwd).is_absolute())
            })
        || kind != KIND_COMMAND_JSONL && section.command.is_some()
        || matches!(
            kind,
            KIND_OPENAI_COMPATIBLE | KIND_CHATGPT_RESPONSES | KIND_XAI_RESPONSES
        ) && section.base_url.is_none()
        || section
            .credential
            .as_deref()
            .is_some_and(|reference| CredentialRef::parse(reference).is_err())
        || section
            .api_key
            .as_ref()
            .is_some_and(|secret| secret.is_empty())
        || section.credential.is_some() && section.api_key.is_some()
        || section.headers.keys().any(|header| {
            matches!(
                header.to_ascii_lowercase().as_str(),
                "authorization" | "proxy-authorization" | "x-api-key" | "api-key"
            )
        })
    {
        return false;
    }
    section.response.as_ref().is_none_or(|response| {
        response.reasoning_only.is_none()
            || matches!(
                kind,
                KIND_OPENAI_COMPATIBLE | crate::model::KIND_GEMINI_INTERACTIONS
            )
    })
}

struct CatalogLimit<'a> {
    value: Option<u32>,
    snapshot: Option<&'a CatalogSnapshot>,
}

fn inventory_limit(
    resolution: &Resolution,
    identity: &str,
    field: &str,
    explicit: Option<u32>,
    trusted: Option<u32>,
    trusted_source: Option<(&str, u32)>,
    catalog: CatalogLimit<'_>,
) -> Option<InventoryLimit> {
    if let Some(value) = explicit {
        let key = format!("models.{}.{}", quote(identity), field);
        let source = resolution.provenance.source(&key)?.clone();
        return Some(InventoryLimit {
            value,
            origin: ModelLimitOrigin::Configured(source),
        });
    }
    if let (Some(value), Some((catalog, revision))) = (trusted, trusted_source) {
        return Some(InventoryLimit {
            value,
            origin: ModelLimitOrigin::Trusted {
                catalog: catalog.to_owned(),
                revision,
            },
        });
    }
    let snapshot = catalog.snapshot?;
    Some(InventoryLimit {
        value: catalog.value?,
        origin: ModelLimitOrigin::Catalog {
            catalog: "models.dev".to_owned(),
            revision: snapshot.revision().to_owned(),
            retrieved_at_ms: snapshot.retrieved_at_ms,
        },
    })
}

fn merge_provider(target: &mut ProviderSection, source: ProviderSection) {
    overlay(&mut target.kind, source.kind);
    overlay(&mut target.base_url, source.base_url);
    overlay(&mut target.credential, source.credential);
    overlay(&mut target.api_key, source.api_key);
    overlay(&mut target.command, source.command);
    target.headers.extend(source.headers);
    match (&mut target.response, source.response) {
        (Some(target), Some(source)) => merge_response(target, source),
        (slot @ None, Some(source)) => *slot = Some(source),
        _ => {}
    }
}

fn merge_response(target: &mut ProviderResponseSection, source: ProviderResponseSection) {
    overlay(&mut target.reasoning_only, source.reasoning_only);
}

fn merge_model(target: &mut ModelSection, source: ModelSection) {
    overlay(&mut target.context_tokens, source.context_tokens);
    overlay(&mut target.max_input_tokens, source.max_input_tokens);
    overlay(&mut target.max_output_tokens, source.max_output_tokens);
}

fn overlay<T>(target: &mut Option<T>, source: Option<T>) {
    if source.is_some() {
        *target = source;
    }
}

fn quote(part: &str) -> String {
    if !part.is_empty()
        && part
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
    {
        part.to_owned()
    } else {
        format!("\"{}\"", part.replace('"', "\\\""))
    }
}

fn position(text: &str, offset: usize) -> Position {
    let consumed = &text[..offset.min(text.len())];
    let line = consumed.matches('\n').count() + 1;
    let column = consumed
        .rfind('\n')
        .map_or(consumed.len(), |index| consumed.len() - index - 1)
        + 1;
    Position {
        line: u32::try_from(line).unwrap_or(u32::MAX),
        column: u32::try_from(column).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_reasoning_behavior_is_not_part_of_model_identity() {
        let mut target = ProviderResponseSection::default();
        merge_response(
            &mut target,
            ProviderResponseSection {
                reasoning_only: Some(ReasoningOnlyBehavior::Text),
            },
        );
        assert_eq!(target.reasoning_only, Some(ReasoningOnlyBehavior::Text));
    }
}
