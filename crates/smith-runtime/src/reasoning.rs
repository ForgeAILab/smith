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
use smith_config::catalog::ZAI_CODING_PLAN_ENDPOINT;
use smith_config::model::ReasoningDialect;
use smith_config::resolve::{Layer, ResolvedConfig, ResolvedModelReasoning, Source, Sourced};
use smith_config::setup::trusted_model;

const INTERCEPTOR_REVISION: &str = "smith-reasoning-selection-1";
/// Stable redaction-safe session-state namespace.
pub const SESSION_STATE_NAMESPACE: &str = "smith.reasoning.override";
const SESSION_STATE_REVISION: &str = "smith-reasoning-override-1";
const SENTINEL_ENABLED: &str = "__smith_reasoning_enabled";
const SENTINEL_DISABLED: &str = "__smith_reasoning_disabled";

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
        }
    }

    /// Whether there is anything to persist.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.effort.is_none()
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
        }?;
        Some(ReasoningConfig {
            effort: Some(effort),
            max_tokens: None,
        })
    }
}

/// Resolves exact controls and validates layered/session defaults.
pub fn resolve_reasoning_policy(
    config: &ResolvedConfig,
    profile: &ResolvedModelProfile,
    endpoint: Option<&str>,
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
    } else {
        // A boolean catalog record deliberately grants no control — including
        // on exact OpenRouter endpoints. Rich per-model metadata must opt
        // into its dialect.
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
        && dialect == Some(ReasoningDialect::OpenaiEffort)
        && selected_effort.is_none()
        && default_effort.is_none()
    {
        return Err(format!(
            "`reasoning.enabled = true` needs an explicit supported effort for this OpenAI-effort binding; choose one of {}",
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

/// Immutable per-turn/provider-request reasoning patch.
#[derive(Debug)]
pub(crate) struct ReasoningInterceptor {
    request: Option<ReasoningConfig>,
}

impl ReasoningInterceptor {
    pub(crate) fn new(policy: &ReasoningRuntimePolicy) -> Self {
        Self {
            request: policy.request_config(),
        }
    }
}

#[async_trait]
impl ModelInterceptor for ReasoningInterceptor {
    fn descriptor(&self) -> ComponentDescriptor {
        ComponentDescriptor::new(
            "smith.reasoning.selection",
            RegistryRevision::new(INTERCEPTOR_REVISION),
        )
    }

    async fn before_model(&self, _view: &ModelView) -> Result<ModelRequestPatch, RuntimeError> {
        Ok(ModelRequestPatch {
            reasoning: Some(self.request.clone()),
            ..ModelRequestPatch::default()
        })
    }
}

/// Provider wrapper translating normalized reasoning into one exact dialect.
#[derive(Debug)]
pub(crate) struct ReasoningDialectProvider {
    inner: Arc<dyn Provider>,
    dialect: ReasoningDialect,
}

impl ReasoningDialectProvider {
    pub(crate) fn new(inner: Arc<dyn Provider>, dialect: ReasoningDialect) -> Self {
        Self { inner, dialect }
    }
}

#[async_trait]
impl Provider for ReasoningDialectProvider {
    fn describe(&self) -> Vec<ModelDescriptor> {
        self.inner.describe()
    }

    fn capabilities(&self, model: &ModelId) -> Option<Capabilities> {
        self.inner.capabilities(model)
    }

    async fn stream(
        &self,
        mut request: ProviderRequest,
        ctx: ProviderCallContext,
    ) -> Result<ProviderStream, ProviderError> {
        adapt_request(&mut request, self.dialect)?;
        self.inner.stream(request, ctx).await
    }
}

fn adapt_request(
    request: &mut ProviderRequest,
    dialect: ReasoningDialect,
) -> Result<(), ProviderError> {
    match dialect {
        ReasoningDialect::OpenaiEffort => {}
        ReasoningDialect::Openrouter => {
            if let Some(reasoning) = request.reasoning.take() {
                let effort = reasoning.effort.ok_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::BadRequest,
                        "OpenRouter reasoning selection has no typed value",
                    )
                })?;
                let value = match effort.as_str() {
                    SENTINEL_ENABLED => json!({"enabled": true}),
                    SENTINEL_DISABLED => json!({"enabled": false}),
                    effort => json!({"effort": effort}),
                };
                insert_extension(&mut request.vendor_extensions, "reasoning", value)?;
            }
        }
        ReasoningDialect::ZaiThinking => {
            if let Some(reasoning) = request.reasoning.take() {
                let state = match reasoning.effort.as_deref() {
                    Some(SENTINEL_ENABLED) => "enabled",
                    Some(SENTINEL_DISABLED) => "disabled",
                    _ => {
                        return Err(ProviderError::new(
                            ProviderErrorKind::BadRequest,
                            "Z.AI thinking selection is not an enabled/disabled state",
                        ));
                    }
                };
                insert_extension(
                    &mut request.vendor_extensions,
                    "thinking",
                    json!({"type": state}),
                )?;
            }
        }
    }
    Ok(())
}

fn insert_extension(extensions: &mut Value, key: &str, value: Value) -> Result<(), ProviderError> {
    if extensions.is_null() {
        *extensions = Value::Object(Map::new());
    }
    let Value::Object(object) = extensions else {
        return Err(ProviderError::new(
            ProviderErrorKind::BadRequest,
            "provider extensions must be a JSON object",
        ));
    };
    if object.insert(key.to_owned(), value).is_some() {
        return Err(ProviderError::new(
            ProviderErrorKind::BadRequest,
            format!("provider extension `{key}` was already set"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_runtime_core::catalog::{ModelLimits, ResolvedModelProfile};
    use agent_runtime_core::content::Message;
    use smith_config::catalog::OPENROUTER_ENDPOINT;
    use smith_config::resolve::{ResolveRequest, resolve};

    fn request(effort: &str) -> ProviderRequest {
        let mut request = ProviderRequest::new(
            ModelId::new("example-model"),
            vec![Message::user("counted input")],
        );
        request.reasoning = Some(ReasoningConfig {
            effort: Some(effort.to_owned()),
            max_tokens: None,
        });
        request
    }

    fn resolved_config() -> ResolvedConfig {
        let home = tempfile::tempdir().expect("home");
        let project = tempfile::tempdir().expect("project");
        std::fs::create_dir_all(project.path().join(".smith")).expect("config directory");
        std::fs::write(
            project.path().join(".smith/config.toml"),
            r#"
default_profile = "dev"

[profiles.dev]
provider = "local"
model = "example-model"

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
"#,
        )
        .expect("config");
        resolve(&ResolveRequest::new(project.path()).with_home_dir(home.path()))
            .expect("resolved config")
            .config
    }

    fn model_profile(reasoning: ReasoningSupport) -> ResolvedModelProfile {
        let mut profile = ResolvedModelProfile::explicit(
            "local",
            ModelId::new("example-model"),
            ModelLimits::new(128_000, 124_000, 4_096),
        );
        profile.capabilities.reasoning = reasoning;
        profile
    }

    #[test]
    fn openai_effort_stays_normalized_for_the_openai_adapter() {
        let mut request = request("high");
        adapt_request(&mut request, ReasoningDialect::OpenaiEffort).expect("adapted");
        assert_eq!(
            request.reasoning.and_then(|reasoning| reasoning.effort),
            Some("high".to_owned())
        );
        assert!(request.vendor_extensions.is_null());
    }

    #[test]
    fn openrouter_effort_becomes_only_the_unified_reasoning_object() {
        let mut request = request("low");
        let messages = request.messages.clone();
        adapt_request(&mut request, ReasoningDialect::Openrouter).expect("adapted");
        assert!(request.reasoning.is_none());
        assert_eq!(
            request.vendor_extensions,
            json!({"reasoning": {"effort": "low"}})
        );
        assert_eq!(
            request.messages, messages,
            "context fields remain immutable"
        );
        assert_eq!(request.model.as_str(), "example-model");
    }

    #[test]
    fn openrouter_explicit_off_is_not_misrouted_as_reasoning_effort() {
        let mut request = request(SENTINEL_DISABLED);
        adapt_request(&mut request, ReasoningDialect::Openrouter).expect("adapted");
        assert!(request.reasoning.is_none());
        assert_eq!(
            request.vendor_extensions,
            json!({"reasoning": {"enabled": false}})
        );
    }

    #[test]
    fn explicit_off_takes_precedence_over_a_retained_effort() {
        let policy = ReasoningRuntimePolicy {
            selected_enabled: Some(false),
            selected_effort: Some("high".to_owned()),
            dialect: Some(ReasoningDialect::Openrouter),
            ..ReasoningRuntimePolicy::default()
        };

        assert_eq!(policy.effective_state(), "off");
        assert_eq!(
            policy
                .request_config()
                .and_then(|reasoning| reasoning.effort),
            Some(SENTINEL_DISABLED.to_owned())
        );
    }

    #[test]
    fn presence_only_or_unknown_models_reject_controls_instead_of_guessing_a_dialect() {
        for support in [ReasoningSupport::Fixed, ReasoningSupport::Unsupported] {
            let mut config = resolved_config();
            config.reasoning.effort = Some(Sourced::new(
                "high".to_owned(),
                Source::session("reasoning.effort"),
            ));
            let error = resolve_reasoning_policy(&config, &model_profile(support), None)
                .expect_err("presence-only metadata cannot expose controls");
            assert!(error.contains("not adjustable"), "{error}");
            assert!(error.contains("presence only"), "{error}");
        }
    }

    #[test]
    fn openai_effort_off_requires_an_advertised_none_effort() {
        let metadata_source = Source::built_in("test model metadata");
        let metadata = |efforts: Vec<String>| ResolvedModelReasoning {
            toggle: Some(Sourced::new(true, metadata_source.clone())),
            mandatory: None,
            efforts: Some(Sourced::new(efforts, metadata_source.clone())),
            default_enabled: None,
            default_effort: None,
            dialect: Some(Sourced::new(
                ReasoningDialect::OpenaiEffort,
                metadata_source.clone(),
            )),
        };

        let mut config = resolved_config();
        config.model_reasoning = metadata(vec!["low".to_owned(), "high".to_owned()]);
        config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));
        let error =
            resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Unsupported), None)
                .expect_err("off would emit a non-advertised `none` effort");
        assert!(error.contains("non-advertised effort `none`"), "{error}");

        config.model_reasoning = metadata(vec!["none".to_owned(), "high".to_owned()]);
        let policy =
            resolve_reasoning_policy(&config, &model_profile(ReasoningSupport::Unsupported), None)
                .expect("an advertised `none` makes off representable");
        assert_eq!(
            policy
                .request_config()
                .and_then(|reasoning| reasoning.effort),
            Some("none".to_owned())
        );
    }

    #[test]
    fn mandatory_reasoning_rejects_off_with_a_local_alternative() {
        let mut config = resolved_config();
        let metadata_source = Source::built_in("test model metadata");
        config.model_reasoning = ResolvedModelReasoning {
            toggle: Some(Sourced::new(false, metadata_source.clone())),
            mandatory: Some(Sourced::new(true, metadata_source.clone())),
            efforts: Some(Sourced::new(
                vec!["high".to_owned()],
                metadata_source.clone(),
            )),
            default_enabled: Some(Sourced::new(true, metadata_source.clone())),
            default_effort: Some(Sourced::new("high".to_owned(), metadata_source.clone())),
            dialect: Some(Sourced::new(ReasoningDialect::Openrouter, metadata_source)),
        };
        config.reasoning.enabled = Some(Sourced::new(false, Source::session("reasoning.enabled")));

        let error = resolve_reasoning_policy(
            &config,
            &model_profile(ReasoningSupport::Unsupported),
            Some(OPENROUTER_ENDPOINT),
        )
        .expect_err("mandatory reasoning cannot be disabled");
        assert!(error.contains("mandatory-on"), "{error}");
    }

    #[test]
    fn zai_toggle_becomes_thinking_type_without_reasoning_effort() {
        let mut request = request(SENTINEL_ENABLED);
        adapt_request(&mut request, ReasoningDialect::ZaiThinking).expect("adapted");
        assert!(request.reasoning.is_none());
        assert_eq!(
            request.vendor_extensions,
            json!({"thinking": {"type": "enabled"}})
        );
    }

    #[test]
    fn persisted_override_is_versioned_redaction_safe_and_additive() {
        let override_value = PersistedReasoningOverride {
            enabled: Some(false),
            effort: Some("high".to_owned()),
        };
        let state = override_value.versioned().expect("versioned state");
        assert_eq!(state.sensitivity, SessionStateSensitivity::RedactionSafe);
        assert_eq!(
            PersistedReasoningOverride::restore(&state).expect("restored"),
            override_value
        );
    }

    #[test]
    fn restored_override_is_session_precedence_and_explicit_reset_skips_it() {
        let saved = PersistedReasoningOverride {
            enabled: Some(false),
            effort: Some("high".to_owned()),
        };

        let mut restored = resolved_config();
        saved.apply(&mut restored, false, false);
        assert_eq!(
            restored.reasoning.enabled.as_ref().map(|value| value.value),
            Some(false)
        );
        assert_eq!(
            restored
                .reasoning
                .effort
                .as_ref()
                .map(|value| value.value.as_str()),
            Some("high")
        );
        assert_eq!(
            restored
                .reasoning
                .effort
                .as_ref()
                .map(|value| value.source.layer),
            Some(Layer::SessionOverride)
        );

        let mut reset = resolved_config();
        saved.apply(&mut reset, true, true);
        assert!(reset.reasoning.enabled.is_none());
        assert!(reset.reasoning.effort.is_none());
    }
}
