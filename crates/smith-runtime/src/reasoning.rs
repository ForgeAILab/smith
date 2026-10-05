//! Typed reasoning capability resolution and exact provider-wire adaptation.
//!
//! A catalog boolean proves only that a model reasons. Adjustable controls
//! require an exact dialect plus bounded metadata for the selected binding.

use std::sync::Arc;

use agent_runtime::harness::{ComponentDescriptor, ModelInterceptor, ModelRequestPatch, ModelView};
use agent_runtime::registry::RegistryRevision;
use agent_runtime_core::catalog::ResolvedModelProfile;
use agent_runtime_core::error::RuntimeError;
use agent_runtime_core::provider::{
    Capabilities, ModelDescriptor, ModelId, Provider, ProviderCallContext, ProviderError,
    ProviderErrorKind, ProviderRequest, ProviderStream, ReasoningConfig, ReasoningSupport,
};
use agent_runtime_core::store::{SessionStateSensitivity, VersionedSessionState};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use smith_config::catalog::{
    CatalogReasoningControls, GEMINI_ENDPOINT, OPENAI_ENDPOINT, OPENROUTER_ENDPOINT,
    XAI_CATALOG_ENDPOINT, ZAI_CODING_PLAN_ENDPOINT,
};
use smith_config::model::ReasoningDialect;
use smith_config::resolve::{Layer, ResolvedConfig, ResolvedModelReasoning, Source, Sourced};
use smith_config::setup::trusted_model;

const INTERCEPTOR_REVISION: &str = "smith-reasoning-selection-1";
/// Stable redaction-safe session-state namespace.
pub const SESSION_STATE_NAMESPACE: &str = "smith.reasoning.override";
const SESSION_STATE_REVISION: &str = "smith-reasoning-override-1";
const SENTINEL_ENABLED: &str = "__smith_reasoning_enabled";
const SENTINEL_DISABLED: &str = "__smith_reasoning_disabled";

/// OpenRouter's vendor-normalized effort buckets, valid for every
/// catalog-advertised reasoning model behind the exact OpenRouter endpoint.
const OPENROUTER_EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// The `reasoning_effort` values every OpenAI reasoning model family accepts.
/// Family-specific extremes (`none`, `minimal`) are deliberately absent; a
/// model that supports one can advertise it through explicit
/// `[models."provider/model".reasoning]` metadata.
const OPENAI_EFFORTS: [&str; 3] = ["low", "medium", "high"];

/// Additive redaction-safe reasoning override stored with a session.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PersistedReasoningOverride {
    /// Explicit thinking state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Provider-advertised effort name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Session-local named context window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<String>,
}

impl PersistedReasoningOverride {
    /// Captures only explicit session-layer values, not profile defaults.
    pub fn from_config(config: &ResolvedConfig) -> Self {
        Self {
            enabled: config.reasoning.enabled.as_ref().and_then(|value| {
                (value.source.layer == Layer::SessionOverride).then_some(value.value)
            }),
            effort: config.reasoning.effort.as_ref().and_then(|value| {
                (value.source.layer == Layer::SessionOverride).then(|| value.value.clone())
            }),
            context_window: config.context_window.as_ref().and_then(|value| {
                (value.source.layer == Layer::SessionOverride).then(|| value.value.clone())
            }),
        }
    }

    /// Whether there is anything to persist.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.effort.is_none() && self.context_window.is_none()
    }

    /// Encodes the additive state for ordinary redaction-safe persistence.
    pub fn versioned(&self) -> Result<VersionedSessionState, RuntimeError> {
        let value = serde_json::to_value(self).map_err(|error| {
            RuntimeError::config(format!(
                "reasoning session override could not be serialized: {error}"
            ))
        })?;
        Ok(VersionedSessionState {
            revision: RegistryRevision::new(SESSION_STATE_REVISION),
            sensitivity: SessionStateSensitivity::RedactionSafe,
            value,
        })
    }

    /// Decodes a compatible saved override. Older sessions omit the namespace.
    pub fn restore(state: &VersionedSessionState) -> Result<Self, RuntimeError> {
        if state.revision.as_str() != SESSION_STATE_REVISION {
            return Err(RuntimeError::config(format!(
                "saved reasoning override uses incompatible revision `{}`",
                state.revision
            )));
        }
        serde_json::from_value(state.value.clone()).map_err(|error| {
            RuntimeError::config(format!("saved reasoning override is invalid: {error}"))
        })
    }

    /// Applies saved values as the highest-precedence session layer.
    pub fn apply(&self, config: &mut ResolvedConfig, reset_enabled: bool, reset_effort: bool) {
        self.apply_with_context_window(config, reset_enabled, reset_effort, false, false);
    }

    /// Applies saved reasoning and context selections while respecting this
    /// invocation's reset and shadow layers.
    pub fn apply_with_context_window(
        &self,
        config: &mut ResolvedConfig,
        reset_enabled: bool,
        reset_effort: bool,
        reset_context_window: bool,
        context_window_shadowed: bool,
    ) {
        let explicit_enabled = config
            .reasoning
            .enabled
            .as_ref()
            .is_some_and(|value| value.source.layer == Layer::SessionOverride);
        let explicit_effort = config
            .reasoning
            .effort
            .as_ref()
            .is_some_and(|value| value.source.layer == Layer::SessionOverride);
        if !reset_enabled
            && !explicit_enabled
            && let Some(enabled) = self.enabled
        {
            config.reasoning.enabled =
                Some(Sourced::new(enabled, Source::session("reasoning.enabled")));
        }
        if !reset_effort
            && !explicit_effort
            && let Some(effort) = &self.effort
        {
            config.reasoning.effort = Some(Sourced::new(
                effort.clone(),
                Source::session("reasoning.effort"),
            ));
        }
        let explicit_context_window = config
            .context_window
            .as_ref()
            .is_some_and(|value| value.source.layer == Layer::SessionOverride);
        if !reset_context_window
            && !context_window_shadowed
            && !explicit_context_window
            && let Some(context_window) = &self.context_window
        {
            config.context_window = Some(Sourced::new(
                context_window.clone(),
                Source::session("context_window"),
            ));
        }
    }
}

/// Whether the resolved dialect exposes an explicit thinking switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningSwitch {
    /// No explicit state can be represented.
    Unavailable,
    /// Both on and off are supported.
    Optional,
    /// Reasoning may be configured but cannot be turned off.
    MandatoryOn,
}

impl ReasoningSwitch {
    /// Stable status spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Optional => "optional",
            Self::MandatoryOn => "mandatory-on",
        }
    }
}

/// Redaction-safe reasoning controls and effective request selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReasoningRuntimePolicy {
    /// Presence/control support after exact binding resolution.
    pub support: ReasoningSupport,
    /// Explicit switch behavior.
    pub switch: ReasoningSwitch,
    /// Ordered supported effort names.
    pub efforts: Vec<String>,
    /// Provider/model default state, when documented.
    pub default_enabled: Option<bool>,
    /// Provider/model default effort, when documented.
    pub default_effort: Option<String>,
    /// Selected state. `None` preserves provider behavior.
    pub selected_enabled: Option<bool>,
    /// Selected effort. `None` preserves provider behavior.
    pub selected_effort: Option<String>,
    /// Exact wire dialect, present only for controllable bindings.
    pub dialect: Option<ReasoningDialect>,
    /// Bounded source of the capability facts.
    pub capability_source: String,
    /// Bounded source of the active selection.
    pub selection_source: String,
}

struct ResolvedReasoningControls {
    support: ReasoningSupport,
    switch: ReasoningSwitch,
    efforts: Vec<String>,
    default_enabled: Option<bool>,
    default_effort: Option<String>,
    dialect: Option<ReasoningDialect>,
    capability_source: String,
}

impl Default for ReasoningRuntimePolicy {
    fn default() -> Self {
        Self {
            support: ReasoningSupport::Unsupported,
            switch: ReasoningSwitch::Unavailable,
            efforts: Vec::new(),
            default_enabled: None,
            default_effort: None,
            selected_enabled: None,
            selected_effort: None,
            dialect: None,
            capability_source: "no trusted reasoning capability metadata".to_owned(),
            selection_source: "provider/model default".to_owned(),
        }
    }
}

impl ReasoningRuntimePolicy {
    /// Whether Smith will emit a reasoning request option.
    pub fn has_override(&self) -> bool {
        self.selected_enabled.is_some() || self.selected_effort.is_some()
    }

    /// Human-readable effective state without pretending an unknown provider
    /// default is known.
    pub fn effective_state(&self) -> &'static str {
        let selected_effort_state = self
            .selected_effort
            .as_deref()
            .map(|effort| effort != "none");
        match self
            .selected_enabled
            .or(selected_effort_state)
            .or(self.default_enabled)
        {
            Some(true) => "on",
            Some(false) => "off",
            None => "provider default",
        }
    }

    /// Human-readable effective effort.
    pub fn effective_effort(&self) -> &str {
        self.selected_effort
            .as_deref()
            .or(self.default_effort.as_deref())
            .unwrap_or("provider default")
    }

    pub(crate) fn request_config(&self) -> Option<ReasoningConfig> {
        let dialect = self.dialect?;
        let effort = match dialect {
            ReasoningDialect::OpenaiEffort => {
                if self.selected_enabled == Some(false) {
                    Some("none".to_owned())
                } else {
                    self.selected_effort.clone().or_else(|| {
                        (self.selected_enabled == Some(true))
                            .then(|| self.default_effort.clone())
                            .flatten()
                    })
                }
            }
            ReasoningDialect::Openrouter | ReasoningDialect::ZaiThinking => {
                match self.selected_enabled {
                    Some(false) => Some(SENTINEL_DISABLED.to_owned()),
                    Some(true) if self.selected_effort.is_none() => {
                        Some(SENTINEL_ENABLED.to_owned())
                    }
                    _ => self.selected_effort.clone(),
                }
            }
            ReasoningDialect::GeminiThinking | ReasoningDialect::AnthropicEffort => {
                self.selected_effort.clone().or_else(|| {
                    (self.selected_enabled == Some(true))
                        .then(|| self.default_effort.clone())
                        .flatten()
                })
            }
        }?;
        Some(ReasoningConfig {
            effort: Some(effort),
            max_tokens: None,
        })
    }
}

/// Resolves exact controls and validates layered/session defaults.
///
/// `catalog_controls` carries the frozen Models.dev-advertised controls for
/// the exact selected binding, when the endpoint is catalog-mapped and the
/// snapshot annotates the model. It refines the per-endpoint defaults; it
/// never creates controls on an endpoint whose wire dialect is unknown, and
/// explicit `[models."provider/model".reasoning]` metadata still wins.
pub fn resolve_reasoning_policy(
    config: &ResolvedConfig,
    profile: &ResolvedModelProfile,
    endpoint: Option<&str>,
    catalog_controls: Option<&CatalogReasoningControls>,
) -> Result<ReasoningRuntimePolicy, String> {
    let metadata = &config.model_reasoning;
    let explicit_controls = metadata.dialect.is_some()
        || metadata.toggle.as_ref().is_some_and(|value| value.value)
        || metadata
            .efforts
            .as_ref()
            .is_some_and(|value| !value.value.is_empty());

    let ResolvedReasoningControls {
        support,
        switch,
        efforts,
        default_enabled,
        default_effort,
        dialect,
        capability_source,
    } = if explicit_controls {
        resolve_explicit(metadata)?
    } else if endpoint == Some(ZAI_CODING_PLAN_ENDPOINT)
        && trusted_model("zai", &config.model.value).is_some()
    {
        ResolvedReasoningControls {
            support: ReasoningSupport::Controllable,
            switch: ReasoningSwitch::Optional,
            efforts: Vec::new(),
            default_enabled: Some(true),
            default_effort: None,
            dialect: Some(ReasoningDialect::ZaiThinking),
            capability_source: "Smith trusted Z.AI Coding Plan model binding".to_owned(),
        }
    } else if endpoint == Some(ZAI_CODING_PLAN_ENDPOINT)
        && profile.capabilities.reasoning != ReasoningSupport::Unsupported
    {
        // The Z.AI Coding Plan endpoint exposes one normalized on/off
        // `thinking` switch for every reasoning model it serves, so a
        // catalog-advertised reasoning model is controllable without
        // per-model metadata. The provider default stays unknown, and any
        // catalog-advertised effort ladder is deliberately unused: the
        // `thinking.type` wire dialect has no effort spelling.
        ResolvedReasoningControls {
            support: ReasoningSupport::Controllable,
            switch: ReasoningSwitch::Optional,
            efforts: Vec::new(),
            default_enabled: None,
            default_effort: None,
            dialect: Some(ReasoningDialect::ZaiThinking),
            capability_source:
                "Z.AI Coding Plan thinking switch (catalog-advertised reasoning model)".to_owned(),
        }
    } else if endpoint == Some(OPENAI_ENDPOINT)
        && profile.capabilities.reasoning != ReasoningSupport::Unsupported
    {
        // OpenAI reasoning models share one `reasoning_effort` control. The
        // exact ladder is model-family-specific, so the catalog-advertised
        // values win over the universal fallback; `off` exists only where
        // `none` is advertised.
        let (efforts, advertised) = match catalog_controls {
            Some(controls) if !controls.efforts.is_empty() => (controls.efforts.clone(), true),
            _ => (OPENAI_EFFORTS.map(str::to_owned).to_vec(), false),
        };
        let has_off = efforts.iter().any(|effort| effort == "none");
        ResolvedReasoningControls {
            support: ReasoningSupport::Controllable,
            switch: if has_off {
                ReasoningSwitch::Optional
            } else {
                ReasoningSwitch::MandatoryOn
            },
            efforts,
            default_enabled: (!has_off).then_some(true),
            default_effort: None,
            dialect: Some(ReasoningDialect::OpenaiEffort),
            capability_source: if advertised {
                "Models.dev advertised OpenAI reasoning controls".to_owned()
            } else {
                "OpenAI reasoning API (catalog-advertised reasoning model)".to_owned()
            },
        }
    } else if endpoint == Some(XAI_CATALOG_ENDPOINT)
        && profile.capabilities.reasoning != ReasoningSupport::Unsupported
    {
        // xAI's Responses endpoint speaks the same typed effort selection the
        // OpenAI-effort dialect already carries. Catalog ladders refine the
        // universal fallback; `off` stays unrepresentable without `none`.
        let (efforts, advertised) = match catalog_controls {
            Some(controls) if !controls.efforts.is_empty() => (controls.efforts.clone(), true),
            _ => (OPENAI_EFFORTS.map(str::to_owned).to_vec(), false),
        };
        let has_off = efforts.iter().any(|effort| effort == "none");
        ResolvedReasoningControls {
            support: ReasoningSupport::Controllable,
            switch: if has_off {
                ReasoningSwitch::Optional
            } else {
                ReasoningSwitch::MandatoryOn
            },
            efforts,
            default_enabled: (!has_off).then_some(true),
            default_effort: None,
            dialect: Some(ReasoningDialect::OpenaiEffort),
            capability_source: if advertised {
                "Models.dev advertised xAI reasoning controls".to_owned()
            } else {
                "xAI Responses reasoning API (catalog-advertised reasoning model)".to_owned()
            },
        }
    } else if endpoint == Some(OPENROUTER_ENDPOINT)
        && profile.capabilities.reasoning != ReasoningSupport::Unsupported
    {
        // OpenRouter normalizes reasoning across vendors: one `reasoning`
        // object carries an on/off `enabled` for every reasoning model plus
        // the model's advertised effort ladder, so the exact endpoint is
        // itself the dialect evidence. A toggle-only or budget-only model
        // keeps the switch and simply advertises no ladder.
        let (efforts, advertised) = match catalog_controls {
            Some(controls) => (controls.efforts.clone(), true),
            None => (OPENROUTER_EFFORTS.map(str::to_owned).to_vec(), false),
        };
        ResolvedReasoningControls {
            support: ReasoningSupport::Controllable,
            switch: ReasoningSwitch::Optional,
            efforts,
            default_enabled: None,
            default_effort: None,
            dialect: Some(ReasoningDialect::Openrouter),
            capability_source: if advertised {
                "Models.dev advertised OpenRouter reasoning controls".to_owned()
            } else {
                "OpenRouter unified reasoning API (catalog-advertised reasoning model)".to_owned()
            },
        }
    } else if endpoint == Some(GEMINI_ENDPOINT)
        && profile.capabilities.reasoning != ReasoningSupport::Unsupported
        && catalog_controls.is_some_and(|controls| !controls.efforts.is_empty())
    {
        // Native Gemini thinking levels are model-specific. Only the frozen
        // catalog's bounded effort list is enough evidence to expose the
        // native `thinking_level` field; a reasoning boolean alone remains
        // fixed and cannot be turned into a guessed request dialect.
        let efforts = catalog_controls
            .expect("the branch requires catalog thinking controls")
            .efforts
            .clone();
        ResolvedReasoningControls {
            support: ReasoningSupport::Controllable,
            switch: ReasoningSwitch::MandatoryOn,
            efforts,
            default_enabled: Some(true),
            default_effort: None,
            dialect: Some(ReasoningDialect::GeminiThinking),
            capability_source: "Models.dev advertised Google Gemini thinking levels".to_owned(),
        }
    } else {
        // On an unknown endpoint a boolean catalog record grants no control:
        // presence is not evidence of a wire dialect. Normalized endpoints
        // are recognized above; anything else needs per-model metadata.
        ResolvedReasoningControls {
            support: profile.capabilities.reasoning,
            switch: ReasoningSwitch::Unavailable,
            efforts: Vec::new(),
            default_enabled: None,
            default_effort: None,
            dialect: None,
            capability_source: "resolved model catalog (presence only)".to_owned(),
        }
    };

    let selected_enabled = config.reasoning.enabled.as_ref().map(|value| value.value);
    let selected_effort = config
        .reasoning
        .effort
        .as_ref()
        .map(|value| value.value.trim().to_ascii_lowercase());
    let selection_source = selection_source(config);

    if (selected_enabled.is_some() || selected_effort.is_some())
        && (support != ReasoningSupport::Controllable || dialect.is_none())
    {
        return Err(format!(
            "reasoning is not adjustable for provider `{}` model `{}`; capability source: {capability_source}",
            config.provider.name.value, config.model.value
        ));
    }
    if selected_enabled == Some(false) {
        match switch {
            ReasoningSwitch::Optional => {}
            ReasoningSwitch::MandatoryOn => {
                return Err(format!(
                    "reasoning cannot be turned off for provider `{}` model `{}`; it is mandatory-on ({capability_source})",
                    config.provider.name.value, config.model.value
                ));
            }
            ReasoningSwitch::Unavailable => {
                return Err(format!(
                    "reasoning has no adjustable on/off switch for provider `{}` model `{}` ({capability_source})",
                    config.provider.name.value, config.model.value
                ));
            }
        }
        // The OpenAI-effort dialect has no off switch of its own: off is sent
        // as the effort `none`, so it stays selectable only when the binding
        // advertises that effort. Mirrored by the `/think` picker entry in
        // `smith-cli`.
        if dialect == Some(ReasoningDialect::OpenaiEffort)
            && !efforts.iter().any(|effort| effort == "none")
        {
            return Err(format!(
                "`off` would send the non-advertised effort `none` for provider `{}` model `{}`; advertise `none` in the binding's reasoning efforts to allow it ({capability_source})",
                config.provider.name.value, config.model.value
            ));
        }
    }
    if let Some(effort) = &selected_effort
        && !efforts.iter().any(|supported| supported == effort)
    {
        let alternatives = if efforts.is_empty() {
            "no effort levels are advertised".to_owned()
        } else {
            format!("supported values: {}", efforts.join(", "))
        };
        return Err(format!(
            "reasoning effort `{effort}` is unavailable for provider `{}` model `{}`; {alternatives} ({capability_source})",
            config.provider.name.value, config.model.value
        ));
    }
    if selected_enabled == Some(true)
        && matches!(
            dialect,
            Some(
                ReasoningDialect::OpenaiEffort
                    | ReasoningDialect::GeminiThinking
                    | ReasoningDialect::AnthropicEffort,
            )
        )
        && selected_effort.is_none()
        && default_effort.is_none()
    {
        let binding = match dialect {
            Some(ReasoningDialect::GeminiThinking) => "Gemini-thinking",
            Some(ReasoningDialect::AnthropicEffort) => "Anthropic-effort",
            _ => "OpenAI-effort",
        };
        return Err(format!(
            "`reasoning.enabled = true` needs an explicit supported effort for this {binding} binding; choose one of {}",
            efforts
                .iter()
                .filter(|effort| effort.as_str() != "none")
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    Ok(ReasoningRuntimePolicy {
        support,
        switch,
        efforts,
        default_enabled,
        default_effort,
        selected_enabled,
        selected_effort,
        dialect,
        capability_source,
        selection_source,
    })
}

fn resolve_explicit(
    metadata: &ResolvedModelReasoning,
) -> Result<ResolvedReasoningControls, String> {
    let dialect = metadata
        .dialect
        .as_ref()
        .ok_or_else(|| "reasoning control metadata must declare an exact `dialect`".to_owned())?;
    let efforts = metadata
        .efforts
        .as_ref()
        .map(|value| {
            value
                .value
                .iter()
                .map(|effort| effort.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let toggle = metadata.toggle.as_ref().is_some_and(|value| value.value);
    let mandatory = metadata.mandatory.as_ref().is_some_and(|value| value.value);
    let switch = if mandatory {
        ReasoningSwitch::MandatoryOn
    } else if toggle
        || (dialect.value == ReasoningDialect::OpenaiEffort
            && efforts.iter().any(|effort| effort == "none"))
    {
        ReasoningSwitch::Optional
    } else {
        ReasoningSwitch::Unavailable
    };
    if dialect.value == ReasoningDialect::ZaiThinking && !efforts.is_empty() {
        return Err(
            "the Z.AI thinking dialect exposes an on/off switch, not effort levels".to_owned(),
        );
    }
    if mandatory
        && metadata
            .default_enabled
            .as_ref()
            .is_some_and(|value| !value.value)
    {
        return Err("mandatory reasoning cannot declare `default_enabled = false`".to_owned());
    }
    let default_effort = metadata
        .default_effort
        .as_ref()
        .map(|value| value.value.to_ascii_lowercase());
    if let Some(default) = &default_effort
        && !efforts.iter().any(|effort| effort == default)
    {
        return Err(format!(
            "default reasoning effort `{default}` is absent from the advertised effort list"
        ));
    }
    Ok(ResolvedReasoningControls {
        support: ReasoningSupport::Controllable,
        switch,
        efforts,
        default_enabled: metadata.default_enabled.as_ref().map(|value| value.value),
        default_effort,
        dialect: Some(dialect.value),
        capability_source: format!("configured model metadata from {}", dialect.source),
    })
}

fn selection_source(config: &ResolvedConfig) -> String {
    let sources = [
        config.reasoning.enabled.as_ref().map(|value| &value.source),
        config.reasoning.effort.as_ref().map(|value| &value.source),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<&Source>>();
    if sources.is_empty() {
        "provider/model default".to_owned()
    } else if sources
        .iter()
        .any(|source| source.layer == smith_config::resolve::Layer::SessionOverride)
    {
        "session override".to_owned()
    } else if sources
        .iter()
        .all(|source| source.layer == smith_config::resolve::Layer::Profile)
    {
        "profile".to_owned()
    } else {
        sources
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

mod provider;

pub(crate) use provider::{ReasoningDialectProvider, ReasoningInterceptor};

#[cfg(test)]
mod tests;
