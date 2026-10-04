//! Pure guided-setup state and rendering.
//!
//! This reducer owns no filesystem, keychain, runtime, or terminal handle.
//! Secret input stays in a private masked buffer and crosses the effect
//! boundary only as Agent Runtime's redaction-safe [`Secret`] wrapper.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use agent_runtime_core::store::Secret;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use smith_client::compact_tokens;

use crate::picker::{
    PickerOutcome, ResourceEntry, ResourcePicker, ScreenFooter, draw_inline_surface,
    draw_picker_with_context, indented_words,
};
use crate::screen::{Screen, ScreenEvent, Step as ScreenStep};
use crate::theme::{Theme, Tone};

/// Why setup was entered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupMode {
    /// Automatic empty-install setup.
    FirstRun,
    /// Explicit `smith setup` action menu.
    Menu,
    /// Direct `smith setup add-provider`.
    AddProvider,
    /// Direct connection using a flow supplied by the CLI.
    Provider {
        /// Reviewed setup flow.
        flow: SetupFlow,
    },
    /// Direct `smith setup add-model`.
    AddModel {
        /// Preselected provider, or a picker when absent.
        provider: Option<String>,
    },
    /// Change only one existing provider's credential source.
    Credential {
        /// Existing provider whose authentication is being changed.
        provider: String,
    },
}

/// Complete explicit model limits collected by setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupModelLimits {
    /// Total model context.
    pub context_tokens: u32,
    /// Maximum enforceable input.
    pub max_input_tokens: u32,
    /// Maximum model output.
    pub max_output_tokens: u32,
}

/// Reviewed quick-start values supplied by the configuration owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupQuickStart {
    /// Provider identity written by the quick start.
    pub provider: String,
    /// Reviewed API base URL.
    pub endpoint: String,
    /// Exact model ID written by the quick start.
    pub model: String,
    /// Human-facing model name for the action menu.
    pub model_label: String,
    /// Enforceable model limits.
    pub limits: SetupModelLimits,
    /// Default request output budget.
    pub request_output_tokens: u32,
    /// Default context output reserve.
    pub output_reserve: u32,
    /// Default profile written by the quick start.
    pub profile: String,
    /// Revision of the trusted metadata used by the quick start.
    pub catalog_revision: u32,
}

/// Wire protocol for a provider collected by the shared setup wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupProviderKind {
    /// Custom OpenAI-compatible endpoint.
    OpenAiCompatible,
    /// Native Anthropic Messages endpoint.
    AnthropicMessages,
}

/// One offered setup action, with its required executable flow.
///
/// ```compile_fail
/// use smith_tui::setup::SetupEntry;
/// let entry = SetupEntry {
///     id: "provider".into(),
///     label: "Provider".into(),
///     detail: "Connect a provider".into(),
/// }; // A flow is required.
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupEntry {
    /// Stable picker identity.
    pub id: String,
    /// Human-facing picker label.
    pub label: String,
    /// Short picker explanation.
    pub detail: String,
    /// Required flow dispatched on confirmation.
    pub flow: SetupFlow,
}

/// Reviewed built-in key plans understood by the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupQuickKey {
    /// Trusted quick-start model.
    Glm,
    /// Responses catalog model.
    Xai,
    /// Native interactions catalog model.
    Google,
}

/// Provider-specific review text supplied by the configuration owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupKeyReview {
    /// Complete action review line.
    pub action: String,
    /// Complete provider and adapter review line.
    pub provider: String,
    /// Complete endpoint review line.
    pub endpoint: String,
    /// Complete default-profile review line.
    pub profile: String,
    /// Additional reasoning review line, when applicable.
    pub reasoning: Option<String>,
}

/// Shared prompt wording supplied by the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupPrompts {
    /// Help for a collected provider name.
    pub provider_name_help: String,
    /// Help for a collected endpoint.
    pub endpoint_help: String,
    /// Error for an invalid environment-variable name.
    pub environment_variable_error: String,
}

/// Plain setup data crossing from the CLI into the pure reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupFlow {
    /// Enter a credential for a reviewed built-in plan.
    QuickKey {
        /// Plan returned to the CLI on submission.
        kind: SetupQuickKey,
        /// Fixed provider identity.
        provider: String,
        /// Fixed API base URL.
        endpoint: String,
        /// Provider-specific review wording.
        review: SetupKeyReview,
        /// Whether the direct connection selects a frozen catalog model.
        catalog_models: bool,
    },
    /// Collect a provider/model using the shared wizard.
    CustomEndpoint {
        /// Provider wire protocol.
        kind: SetupProviderKind,
        /// Fixed identity, or collect one when absent.
        provider: Option<String>,
        /// Fixed endpoint, or collect one when absent.
        endpoint: Option<String>,
        /// Complete action review line.
        review_action: String,
        /// Adapter name shown in the review.
        adapter: String,
        /// Whether the direct connection selects a frozen catalog model.
        catalog_models: bool,
    },
    /// Hand off through the host's runner so browser sign-in can return to setup.
    OAuth {
        /// Progress wording supplied by the host.
        busy_note: String,
    },
    /// Attach a model to a configured provider.
    AddModel,
    /// Select a configured provider/model pair as default.
    ChangeDefault,
}

/// Authentication choice crossing from the pure reducer to CLI effects.
#[derive(Clone)]
pub enum SetupCredential {
    /// Store a newly-entered key in the platform service.
    StoreInKeychain(Secret),
    /// Store a newly-entered key in owner-only user configuration.
    StoreInConfig(Secret),
    /// Use the reviewed keychain location without replacing it.
    ExistingKeychain,
    /// Record an environment reference without reading its value.
    Environment(String),
}

impl fmt::Debug for SetupCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoreInKeychain(secret) => formatter
                .debug_tuple("StoreInKeychain")
                .field(secret)
                .finish(),
            Self::StoreInConfig(secret) => formatter
                .debug_tuple("StoreInConfig")
                .field(secret)
                .finish(),
            Self::ExistingKeychain => formatter.write_str("ExistingKeychain"),
            Self::Environment(variable) => formatter
                .debug_tuple("Environment")
                .field(variable)
                .finish(),
        }
    }
}

/// Reviewed setup operation for the CLI to persist and preflight.
#[derive(Debug, Clone)]
pub enum SetupSubmission {
    /// Smith's trusted Z.AI / GLM quick start.
    QuickGlm {
        /// Reviewed authentication choice.
        credential: SetupCredential,
    },
    /// xAI connection and its reviewed catalog model.
    QuickXai {
        /// Reviewed authentication choice.
        credential: SetupCredential,
        /// Exact model selected from the frozen xAI catalog.
        model: String,
    },
    /// Native Google Gemini connection and its reviewed catalog model.
    QuickGoogle {
        /// Reviewed authentication choice.
        credential: SetupCredential,
        /// Exact model selected from the frozen Google catalog.
        model: String,
    },
    /// A provider and its first model collected by the shared wizard.
    AddProvider {
        /// Provider wire protocol.
        kind: SetupProviderKind,
        /// Provider identity.
        provider: String,
        /// API base URL.
        endpoint: String,
        /// Reviewed authentication choice.
        credential: SetupCredential,
        /// Provider model ID.
        model: String,
        /// Explicit enforceable limits.
        limits: SetupModelLimits,
        /// Whether reasoning-only successful output becomes visible text.
        reasoning_only_text: bool,
        /// Whether this pair becomes the default.
        make_default: bool,
    },
    /// A model added beneath an existing provider.
    AddModel {
        /// Existing provider identity.
        provider: String,
        /// Provider model ID.
        model: String,
        /// Explicit enforceable limits.
        limits: SetupModelLimits,
        /// Whether this pair becomes the default.
        make_default: bool,
    },
    /// Make one already-configured pair the default.
    ChangeDefault {
        /// Provider identity.
        provider: String,
        /// Provider model ID.
        model: String,
    },
    /// Replace only one existing provider's credential source.
    ChangeCredential {
        /// Existing provider identity.
        provider: String,
        /// Reviewed authentication choice.
        credential: SetupCredential,
    },
}

/// Complete limits one automatic source resolved, with review provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelLimits {
    /// Total context window.
    pub context_tokens: u32,
    /// Enforced input ceiling.
    pub max_input_tokens: u32,
    /// Enforced output ceiling.
    pub max_output_tokens: u32,
    /// Bounded provenance shown in review, e.g. `endpoint /models listing`.
    pub source: String,
}

/// A best-effort automatic limit resolution the driver must complete.
#[derive(Clone)]
pub struct ResolveModelLimits {
    /// Whether this adapter supports the reviewed OpenAI-compatible listing.
    pub use_endpoint_listing: bool,
    /// The flow's known OpenAI-compatible base URL, when it has one.
    pub endpoint: Option<String>,
    /// A bearer the flow already holds, when it holds one.
    pub bearer: Option<Secret>,
    /// Chosen environment variable holding the bearer, when chosen.
    pub environment_variable: Option<String>,
    /// Locally configured provider name the driver can resolve instead.
    pub provider: Option<String>,
    /// The model ID just entered.
    pub model: String,
}

impl fmt::Debug for ResolveModelLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The bearer is a secret on its way to a probe; effects reach test
        // output and must not carry it, even redacted-by-convention.
        formatter
            .debug_struct("ResolveModelLimits")
            .field("use_endpoint_listing", &self.use_endpoint_listing)
            .field("endpoint", &self.endpoint)
            .field("bearer", &self.bearer.as_ref().map(|_| "[redacted]"))
            .field("environment_variable", &self.environment_variable)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .finish()
    }
}

/// Effect requested by one setup key.
#[derive(Debug, Clone)]
pub enum SetupEffect {
    /// Continue rendering.
    None,
    /// Exit successfully without writing or starting a session.
    Cancel,
    /// Hand off to Smith's ChatGPT OAuth connection using the host's screen runner.
    ConnectChatGpt,
    /// Run the bounded automatic limit resolution and feed it back through
    /// [`SetupApp::apply_resolved_limits`].
    ResolveModelLimits {
        /// Everything the driver needs to try each source.
        request: ResolveModelLimits,
    },
    /// Persist and preflight the reviewed submission.
    Submit {
        /// Reviewed setup values.
        submission: SetupSubmission,
        /// Whether a second review explicitly accepted differing existing
        /// user-config leaves.
        allow_collisions: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupAction {
    QuickGlm,
    QuickXai,
    QuickGoogle,
    AddProvider,
    AddModel,
    ChangeDefault,
    ChangeCredential,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialMethod {
    Keychain,
    Config,
    ExistingKeychain,
    Environment,
}

impl CredentialMethod {
    fn takes_secret(self) -> bool {
        matches!(self, Self::Keychain | Self::Config)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Action,
    ProviderChoice,
    ProviderName,
    Endpoint,
    CredentialMethod,
    CredentialValue,
    ModelChoice,
    ModelName,
    ContextTokens,
    ResponseBehavior,
    DefaultChoice,
    Review,
    Busy,
}

#[derive(Default)]
struct MaskedInput(String);

impl MaskedInput {
    fn push(&mut self, character: char) {
        self.0.push(character);
    }

    fn push_str(&mut self, value: &str) {
        self.0.push_str(value);
    }

    fn pop(&mut self) {
        self.0.pop();
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn masked(&self) -> String {
        "•".repeat(self.0.chars().count())
    }

    fn secret(&self) -> Secret {
        Secret::new(self.0.clone())
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

impl fmt::Debug for MaskedInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MaskedInput([redacted])")
    }
}

/// Pure setup application state.
pub struct SetupApp {
    mode: SetupMode,
    step: Step,
    history: Vec<Step>,
    picker_selections: BTreeMap<Step, String>,
    picker: Option<ResourcePicker>,
    provider_actions: Vec<SetupEntry>,
    key_review: Option<SetupKeyReview>,
    review_action: String,
    adapter: String,
    prompts: SetupPrompts,
    catalog_models: bool,
    provider_entries: Vec<ResourceEntry>,
    model_entries: Vec<ResourceEntry>,
    catalog_model_limits: BTreeMap<String, SetupModelLimits>,
    quick_start: SetupQuickStart,
    action: Option<SetupAction>,
    provider_kind: SetupProviderKind,
    provider: String,
    endpoint: String,
    credential_method: Option<CredentialMethod>,
    environment_variable: String,
    secret: MaskedInput,
    model: String,
    context_tokens: Option<u32>,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
    reasoning_only_text: bool,
    make_default: bool,
    input: String,
    error: Option<String>,
    /// Why the surface is busy, shown instead of the default applying note.
    busy_note: Option<String>,
    /// Provenance of automatically resolved limits, shown in review.
    limits_source: Option<String>,
    collision_preview: Option<String>,
    review_scroll: Cell<ReviewScroll>,
    allow_collisions: bool,
    destination: String,
}

/// Wrapped-row viewport refreshed by drawing, including after a resize.
#[derive(Debug, Clone, Copy, Default)]
struct ReviewScroll {
    offset: usize,
    limit: usize,
    page: usize,
}

impl fmt::Debug for SetupApp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetupApp")
            .field("mode", &self.mode)
            .field("step", &self.step)
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("credential_method", &self.credential_method)
            .field("environment_variable", &self.environment_variable)
            .field("secret", &self.secret)
            .field("model", &self.model)
            .field("context_tokens", &self.context_tokens)
            .field("max_input_tokens", &self.max_input_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("destination", &self.destination)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl SetupApp {
    /// Creates setup with locally configured provider/model choices.
    pub fn new(
        mode: SetupMode,
        provider_entries: Vec<ResourceEntry>,
        model_entries: Vec<ResourceEntry>,
        quick_start: SetupQuickStart,
        provider_actions: Vec<SetupEntry>,
        prompts: SetupPrompts,
    ) -> Self {
        let mut app = Self {
            mode: mode.clone(),
            step: Step::Action,
            history: Vec::new(),
            picker_selections: BTreeMap::new(),
            picker: None,
            provider_actions,
            key_review: None,
            review_action: String::new(),
            adapter: String::new(),
            prompts,
            catalog_models: false,
            provider_entries,
            model_entries,
            catalog_model_limits: BTreeMap::new(),
            quick_start,
            action: None,
            provider_kind: SetupProviderKind::OpenAiCompatible,
            provider: String::new(),
            endpoint: String::new(),
            credential_method: None,
            environment_variable: String::new(),
            secret: MaskedInput::default(),
            model: String::new(),
            context_tokens: None,
            max_input_tokens: None,
            max_output_tokens: None,
            reasoning_only_text: false,
            make_default: true,
            input: String::new(),
            error: None,
            busy_note: None,
            limits_source: None,
            collision_preview: None,
            review_scroll: Cell::new(ReviewScroll::default()),
            allow_collisions: false,
            destination: "~/.smith/config.toml".into(),
        };
        match mode {
            SetupMode::FirstRun | SetupMode::Menu => app.enter(Step::Action, false),
            SetupMode::AddProvider => {
                let flow = app
                    .provider_actions
                    .iter()
                    .find_map(|entry| {
                        matches!(entry.flow, SetupFlow::CustomEndpoint { provider: None, .. })
                            .then(|| entry.flow.clone())
                    })
                    .expect("the CLI supplies the custom endpoint flow for add-provider");
                app.start_flow(flow, false);
            }
            SetupMode::Provider { flow } => {
                app.start_flow(flow, false);
            }
            SetupMode::AddModel {
                provider: Some(provider),
            } => {
                app.action = Some(SetupAction::AddModel);
                app.provider = provider;
                app.enter(Step::ModelName, false);
            }
            SetupMode::AddModel { provider: None } => {
                app.action = Some(SetupAction::AddModel);
                app.enter(Step::ProviderChoice, false);
            }
            SetupMode::Credential { provider } => {
                app.action = Some(SetupAction::ChangeCredential);
                app.provider = provider;
                app.enter(Step::CredentialMethod, false);
            }
        }
        app
    }

    /// Sets the exact user-scoped config destination shown during review.
    #[must_use]
    pub fn with_destination(mut self, destination: impl Into<String>) -> Self {
        self.destination = destination.into();
        self
    }

    /// Replaces offered actions with entries carrying executable flows.
    #[must_use]
    pub fn with_provider_actions(mut self, actions: Vec<SetupEntry>) -> Self {
        self.provider_actions = actions;
        self.configure_picker();
        self
    }

    /// Supplies reviewed catalog limits keyed by the exact model IDs shown by
    /// a built-in provider picker.
    #[must_use]
    pub fn with_catalog_model_limits(mut self, limits: BTreeMap<String, SetupModelLimits>) -> Self {
        self.catalog_model_limits = limits;
        self.configure_picker();
        self
    }

    /// Whether setup is waiting for an external persistence/preflight effect.
    pub fn is_busy(&self) -> bool {
        self.step == Step::Busy
    }

    /// Whether the initial setup action picker is still active.
    pub fn is_choosing_action(&self) -> bool {
        self.step == Step::Action
    }

    /// Why the busy step is busy, when a specific reason was recorded.
    pub fn busy_note(&self) -> Option<&str> {
        self.busy_note.as_deref()
    }

    /// Continues after the driver's bounded automatic limit resolution.
    ///
    /// `None` means no source knew the context window, so the user enters that
    /// one value. Smith derives both ceilings without presenting more numeric
    /// fields. Resolved limits skip numeric entry; Back still exposes the
    /// prefilled context window for an intentional override.
    pub fn apply_resolved_limits(&mut self, resolved: Option<ResolvedModelLimits>) {
        self.busy_note = None;
        // Restore the model step as the back target so a completed effect
        // cannot become an editable step when the user moves backward.
        self.step = self.history.pop().unwrap_or(Step::ModelName);
        match resolved {
            None => self.enter(Step::ContextTokens, true),
            Some(resolved) => {
                self.context_tokens = Some(resolved.context_tokens);
                self.max_input_tokens = Some(resolved.max_input_tokens);
                self.max_output_tokens = Some(resolved.max_output_tokens);
                self.limits_source = Some(resolved.source);
                let next = if self.needs_response_behavior() {
                    Step::ResponseBehavior
                } else {
                    Step::DefaultChoice
                };
                self.enter(next, true);
                self.history.push(Step::ContextTokens);
            }
        }
    }

    /// Returns setup to an actionable step with a bounded external error.
    pub fn fail(&mut self, message: impl Into<String>, authentication: bool) {
        self.error = Some(bound(message.into(), 1_024));
        self.review_scroll.set(ReviewScroll::default());
        self.step = if authentication {
            Step::CredentialMethod
        } else {
            Step::Review
        };
        self.configure_picker();
    }

    /// Shows the exact secret-safe merge preview and requires a second
    /// confirmation before replacing differing existing leaves.
    pub fn review_collisions(&mut self, preview: impl Into<String>) {
        self.collision_preview = Some(bound(preview.into(), 8_192));
        self.review_scroll.set(ReviewScroll::default());
        self.allow_collisions = true;
        self.error = Some(
            "Existing values differ. Review the additional lines, then press Enter again to replace only those values."
                .into(),
        );
        self.step = Step::Review;
        self.configure_picker();
    }

    /// Labels reviewed facts so color is never needed to distinguish what is written.
    pub fn review_lines(&self) -> Vec<String> {
        let rows = self.review_rows();
        let label_width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
        rows.into_iter()
            .map(|(label, value)| format!("{label:label_width$}  {value}"))
            .collect()
    }

    fn review_rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = match self.action {
            Some(SetupAction::QuickGlm) => vec![
                (
                    "Provider",
                    format!(
                        "{} · {}",
                        self.quick_start.provider, self.quick_start.endpoint
                    ),
                ),
                (
                    "API key",
                    self.credential_reference(&self.quick_start.provider),
                ),
                (
                    "Model",
                    format!(
                        "{} · {} · trusted catalog v{}",
                        self.quick_start.model,
                        compact_limits(self.quick_start.limits),
                        self.quick_start.catalog_revision,
                    ),
                ),
                (
                    "Requests",
                    format!(
                        "{} output · {} reserved",
                        compact_tokens(u64::from(self.quick_start.request_output_tokens)),
                        compact_tokens(u64::from(self.quick_start.output_reserve)),
                    ),
                ),
                (
                    "GLM replies",
                    "an answer sent only as reasoning is shown as the reply; thinking stays on"
                        .into(),
                ),
                ("Default", format!("profile {}", self.quick_start.profile)),
            ],
            Some(SetupAction::QuickXai | SetupAction::QuickGoogle) => {
                let review = self
                    .key_review
                    .as_ref()
                    .expect("a quick key flow supplies review text");
                let mut rows = vec![
                    ("Action", review_value(&review.action).to_owned()),
                    ("Provider", format!("{} · {}", self.provider, self.endpoint)),
                    (
                        "Connection",
                        if self.action == Some(SetupAction::QuickGoogle) {
                            "Gemini API"
                        } else {
                            "xAI API"
                        }
                        .into(),
                    ),
                    ("API key", self.credential_reference(&self.provider)),
                    (
                        "Model",
                        format!(
                            "{}/{} · {} · Models.dev frozen catalog",
                            self.provider,
                            self.model,
                            compact_limits(SetupModelLimits {
                                context_tokens: self.context_tokens.unwrap_or_default(),
                                max_input_tokens: self.max_input_tokens.unwrap_or_default(),
                                max_output_tokens: self.max_output_tokens.unwrap_or_default(),
                            })
                        ),
                    ),
                    (
                        "Requests",
                        "output and reserve derived from the selected catalog model".into(),
                    ),
                ];
                if let Some(reasoning) = &review.reasoning {
                    rows.push(("Thinking", review_value(reasoning).to_owned()));
                }
                rows.push((
                    "Default",
                    format!("profile {}", review_value(&review.profile)),
                ));
                rows
            }
            Some(SetupAction::AddProvider) => vec![
                ("Action", review_value(&self.review_action).to_owned()),
                (
                    "Connection",
                    match self.adapter.as_str() {
                        "anthropic-messages" => "Anthropic Messages API".into(),
                        "openai-compatible" => "OpenAI-compatible API".into(),
                        _ => self.adapter.clone(),
                    },
                ),
                ("Provider", format!("{} · {}", self.provider, self.endpoint)),
                ("API key", self.credential_reference(&self.provider)),
                (
                    "Model",
                    format!(
                        "{}/{} · {}",
                        self.provider,
                        self.model,
                        self.limits_review()
                    ),
                ),
                (
                    "Replies",
                    if self.reasoning_only_text {
                        "an answer sent only as reasoning is shown as the reply".into()
                    } else {
                        "keep the provider's answer and thinking separate".into()
                    },
                ),
                (
                    "Default",
                    if self.make_default {
                        "use this model"
                    } else {
                        "keep current selection"
                    }
                    .into(),
                ),
            ],
            Some(SetupAction::AddModel) => vec![
                ("Action", "Add model".into()),
                ("Provider", self.provider.clone()),
                (
                    "Model",
                    format!(
                        "{}/{} · {}",
                        self.provider,
                        self.model,
                        self.limits_review()
                    ),
                ),
                (
                    "Default",
                    if self.make_default {
                        "use this model"
                    } else {
                        "keep current selection"
                    }
                    .into(),
                ),
            ],
            Some(SetupAction::ChangeDefault) => vec![
                ("Action", "Change default model".into()),
                ("Default", format!("{}/{}", self.provider, self.model)),
            ],
            Some(SetupAction::ChangeCredential) => vec![
                ("Action", "Change provider credential".into()),
                ("Provider", self.provider.clone()),
                ("API key", self.credential_reference(&self.provider)),
            ],
            None => vec![("Action", "Choose how to connect a model.".into())],
        };
        if self.credential_method == Some(CredentialMethod::Config) {
            rows.push((
                "Warning",
                "plaintext at rest; same-user processes can read this key".into(),
            ));
            rows.push(("Backups", "may retain this key after rotation".into()));
        }
        if let Some(preview) = &self.collision_preview {
            rows.push(("Changes", "existing values to replace:".into()));
            rows.extend(preview.lines().map(|line| ("", line.to_owned())));
        }
        let home = std::env::var_os("HOME");
        let destination = review_destination(&self.destination, home.as_deref().map(Path::new));
        rows.push((
            "Writes",
            format!("{destination}, then checks the connection"),
        ));
        rows
    }

    fn wrapped_review_lines(&self, width: u16) -> Vec<Line<'static>> {
        let rows = self.review_rows();
        let label_width = rows.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
        let indent = label_width + 4;
        let value_width = width
            .saturating_sub(u16::try_from(indent).unwrap_or(u16::MAX))
            .max(1);
        rows.into_iter()
            .flat_map(|(label, value)| {
                // Collision previews retain their existing full-row wrapping.
                if label.is_empty() {
                    let mut lines = crate::render::wrap::wrap_lines(
                        &[Line::from(format!("{label:label_width$}  {value}"))],
                        width.saturating_sub(2),
                    );
                    for line in &mut lines {
                        line.spans.insert(0, Span::raw("  "));
                    }
                    return lines;
                }
                let mut lines = crate::render::wrap::wrap_lines(&[Line::from(value)], value_width);
                for (index, line) in lines.iter_mut().enumerate() {
                    let prefix = if index == 0 {
                        format!("  {label:label_width$}  ")
                    } else {
                        " ".repeat(indent)
                    };
                    line.spans.insert(0, Span::raw(prefix));
                }
                lines
            })
            .collect()
    }

    /// Reduces one setup key.
    pub fn on_key(&mut self, key: KeyEvent) -> SetupEffect {
        if key.kind == KeyEventKind::Release {
            return SetupEffect::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SetupEffect::Cancel;
        }
        if key.code == KeyCode::Esc || key.code == KeyCode::BackTab {
            if self.history.is_empty() {
                return if key.code == KeyCode::Esc {
                    SetupEffect::Cancel
                } else {
                    SetupEffect::None
                };
            }
            self.back();
            return SetupEffect::None;
        }
        if self.step == Step::Busy {
            return SetupEffect::None;
        }
        if self.step == Step::Review
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
            )
        {
            let mut scroll = self.review_scroll.get();
            let page = scroll.page.max(1);
            scroll.offset = match key.code {
                KeyCode::Up => scroll.offset.saturating_sub(1),
                KeyCode::Down => scroll.offset.saturating_add(1).min(scroll.limit),
                KeyCode::PageUp => scroll.offset.saturating_sub(page),
                KeyCode::PageDown => scroll.offset.saturating_add(page).min(scroll.limit),
                _ => unreachable!("review scrolling keys were checked above"),
            };
            self.review_scroll.set(scroll);
            return SetupEffect::None;
        }
        self.error = None;

        if let Some(picker) = &mut self.picker {
            return match picker.on_key(key) {
                PickerOutcome::Pending => SetupEffect::None,
                PickerOutcome::Cancelled => SetupEffect::Cancel,
                PickerOutcome::Back => {
                    self.back();
                    SetupEffect::None
                }
                PickerOutcome::Selected(id) => self.select_picker(id),
            };
        }

        match key.code {
            KeyCode::Backspace => {
                if self.step == Step::CredentialValue
                    && self
                        .credential_method
                        .is_some_and(CredentialMethod::takes_secret)
                {
                    self.secret.pop();
                } else {
                    self.input.pop();
                }
                SetupEffect::None
            }
            KeyCode::Enter => self.submit_input(),
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) =>
            {
                if self.step == Step::CredentialValue
                    && self
                        .credential_method
                        .is_some_and(CredentialMethod::takes_secret)
                {
                    self.secret.push(character);
                } else {
                    self.input.push(character);
                }
                SetupEffect::None
            }
            _ => SetupEffect::None,
        }
    }

    /// Folds one bracketed paste into the active text field.
    ///
    /// Pasting is how credentials usually arrive; without this, enabling
    /// bracketed paste would silently swallow them. Newlines and controls are
    /// dropped so a trailing newline cannot auto-submit a half-read form.
    pub fn on_paste(&mut self, text: &str) {
        if self.step == Step::Busy {
            return;
        }
        if let Some(picker) = &mut self.picker {
            picker.paste(text);
            return;
        }
        self.error = None;
        let cleaned = text
            .chars()
            .filter(|character| !character.is_control())
            .collect::<String>();
        if cleaned.is_empty() {
            return;
        }
        if self.step == Step::CredentialValue
            && self
                .credential_method
                .is_some_and(CredentialMethod::takes_secret)
        {
            self.secret.push_str(&cleaned);
        } else {
            self.input.push_str(&cleaned);
        }
    }

    fn enter(&mut self, step: Step, remember: bool) {
        if remember {
            self.history.push(self.step);
        }
        self.step = step;
        self.review_scroll.set(ReviewScroll::default());
        self.input.clear();
        self.error = None;
        if step != Step::Review {
            self.collision_preview = None;
            self.allow_collisions = false;
        }
        self.configure_picker();
    }

    fn back(&mut self) {
        if let Some(step) = self.history.pop() {
            self.step = step;
            self.review_scroll.set(ReviewScroll::default());
            self.input = match step {
                Step::ProviderName => self.provider.clone(),
                Step::Endpoint => self.endpoint.clone(),
                Step::ModelName => self.model.clone(),
                Step::CredentialValue
                    if self.credential_method == Some(CredentialMethod::Environment) =>
                {
                    self.environment_variable.clone()
                }
                // A context window already chosen by resolution is what Back
                // is there to edit, so it is what the only numeric field
                // shows.
                Step::ContextTokens => self
                    .context_tokens
                    .map_or_else(String::new, |value| value.to_string()),
                _ => String::new(),
            };
            // Secret input is deliberately never restored by navigation. If
            // Back reaches authentication again, require a fresh key rather
            // than retaining the previously entered credential.
            if matches!(step, Step::CredentialMethod | Step::CredentialValue) {
                self.secret.clear();
            }
            self.error = None;
            // Leaving the review invalidates a collision approval: anything
            // edited on the way back must be re-reviewed before it can
            // replace existing values.
            self.collision_preview = None;
            self.allow_collisions = false;
            self.configure_picker();
        }
    }

    /// Resumes setup after a nested login backs out without dropping its selected action.
    pub fn back_from_chatgpt(&mut self) {
        self.busy_note = None;
        self.back();
    }

    fn configure_picker(&mut self) {
        self.picker = match self.step {
            Step::Action => {
                let entries = self
                    .provider_actions
                    .iter()
                    .map(|entry| ResourceEntry::new(&entry.id, &entry.label, &entry.detail))
                    .collect();
                Some(ResourcePicker::choices(
                    "Smith setup",
                    entries,
                    "No setup actions are available.",
                ))
            }
            Step::ProviderChoice => Some(ResourcePicker::new(
                "Choose provider",
                self.provider_entries.clone(),
                "No configured provider · go back to add one",
            )),
            Step::CredentialMethod => Some(ResourcePicker::choices(
                "Authentication",
                vec![
                    ResourceEntry::new(
                        "keychain",
                        "Store API key securely",
                        "macOS Keychain / Linux Secret Service",
                    ),
                    ResourceEntry::new(
                        "existing-keychain",
                        "Use existing secure entry",
                        format!("keychain:smith/{}", self.provider),
                    ),
                    ResourceEntry::new(
                        "config",
                        "Store in config (no prompts)",
                        "plaintext at rest · readable by same-user processes and backups",
                    ),
                    ResourceEntry::new(
                        "environment",
                        "Use environment variable",
                        "record a reference; Smith does not read or copy it now",
                    ),
                ],
                "Choose a credential method.",
            )),
            Step::ModelChoice => Some(ResourcePicker::new(
                "Choose default model",
                self.model_entries.clone(),
                "No selectable model · go back to add one",
            )),
            Step::ResponseBehavior => Some(ResourcePicker::choices(
                "Response compatibility",
                vec![
                    ResourceEntry::new(
                        "normal",
                        "Preserve response fields",
                        "recommended for ordinary OpenAI-compatible endpoints",
                    ),
                    ResourceEntry::new(
                        "reasoning-text",
                        "Reasoning-only success is visible text",
                        "for endpoints that put final answers only in reasoning_content",
                    ),
                ],
                "Choose response behavior.",
            )),
            Step::DefaultChoice => Some(ResourcePicker::choices(
                "Default selection",
                vec![
                    ResourceEntry::new("yes", "Make this the default", "used by plain `smith`"),
                    ResourceEntry::new(
                        "no",
                        "Keep current default",
                        "new choice remains selectable",
                    ),
                ],
                "Choose whether to change the default.",
            )),
            _ => None,
        };
        if let Some(picker) = self.picker.take() {
            let mut picker = picker.with_back(!self.history.is_empty());
            if let Some(id) = self.picker_selections.get(&self.step)
                && let Some(index) = picker.entries.iter().position(|entry| &entry.id == id)
            {
                picker.selected = index;
            }
            self.picker = Some(picker);
        }
    }

    fn start_flow(&mut self, flow: SetupFlow, remember: bool) -> SetupEffect {
        match flow {
            SetupFlow::QuickKey {
                kind,
                provider,
                endpoint,
                review,
                catalog_models,
            } => {
                self.action = Some(match kind {
                    SetupQuickKey::Glm => SetupAction::QuickGlm,
                    SetupQuickKey::Xai => SetupAction::QuickXai,
                    SetupQuickKey::Google => SetupAction::QuickGoogle,
                });
                self.provider = provider;
                self.endpoint = endpoint;
                self.key_review = Some(review);
                self.catalog_models = catalog_models;
                if kind == SetupQuickKey::Glm {
                    self.model = self.quick_start.model.clone();
                    self.context_tokens = Some(self.quick_start.limits.context_tokens);
                    self.max_input_tokens = Some(self.quick_start.limits.max_input_tokens);
                    self.max_output_tokens = Some(self.quick_start.limits.max_output_tokens);
                    self.reasoning_only_text = true;
                    self.make_default = true;
                }
                self.enter(Step::CredentialMethod, remember);
            }
            SetupFlow::CustomEndpoint {
                kind,
                provider,
                endpoint,
                review_action,
                adapter,
                catalog_models,
            } => {
                self.action = Some(SetupAction::AddProvider);
                self.provider_kind = kind;
                self.review_action = review_action;
                self.adapter = adapter;
                self.catalog_models = catalog_models;
                if let Some(provider) = provider {
                    self.provider = provider;
                    self.endpoint = endpoint.unwrap_or_default();
                    if kind == SetupProviderKind::AnthropicMessages {
                        self.reasoning_only_text = false;
                    }
                    self.enter(Step::CredentialMethod, remember);
                } else {
                    self.enter(Step::ProviderName, remember);
                }
            }
            SetupFlow::OAuth { busy_note } => {
                self.enter(Step::Busy, remember);
                self.busy_note = Some(busy_note);
                return SetupEffect::ConnectChatGpt;
            }
            SetupFlow::AddModel => {
                self.action = Some(SetupAction::AddModel);
                self.enter(Step::ProviderChoice, remember);
            }
            SetupFlow::ChangeDefault => {
                self.action = Some(SetupAction::ChangeDefault);
                self.enter(Step::ModelChoice, remember);
            }
        }
        SetupEffect::None
    }

    fn select_picker(&mut self, id: String) -> SetupEffect {
        self.picker_selections.insert(self.step, id.clone());
        match self.step {
            Step::Action => {
                let flow = self
                    .provider_actions
                    .iter()
                    .find(|entry| entry.id == id)
                    .expect("the picker only confirms offered setup entries")
                    .flow
                    .clone();
                return self.start_flow(flow, true);
            }
            Step::ProviderChoice => {
                self.provider = id;
                self.enter(Step::ModelName, true);
            }
            Step::CredentialMethod => match id.as_str() {
                "keychain" => {
                    self.credential_method = Some(CredentialMethod::Keychain);
                    self.secret.clear();
                    self.enter(Step::CredentialValue, true);
                }
                "existing-keychain" => {
                    self.secret.clear();
                    self.credential_method = Some(CredentialMethod::ExistingKeychain);
                    self.enter(self.after_credential_step(), true);
                }
                "config" => {
                    self.secret.clear();
                    self.credential_method = Some(CredentialMethod::Config);
                    self.enter(Step::CredentialValue, true);
                }
                "environment" => {
                    self.secret.clear();
                    self.credential_method = Some(CredentialMethod::Environment);
                    self.enter(Step::CredentialValue, true);
                }
                _ => {}
            },
            Step::ModelChoice => {
                if self.catalog_models {
                    let Some(limits) = self.catalog_model_limits.get(&id).copied() else {
                        self.error =
                            Some("the selected catalog model has no enforceable limits".to_owned());
                        return SetupEffect::None;
                    };
                    self.model = id;
                    self.context_tokens = Some(limits.context_tokens);
                    self.max_input_tokens = Some(limits.max_input_tokens);
                    self.max_output_tokens = Some(limits.max_output_tokens);
                    self.enter(Step::DefaultChoice, true);
                } else if let Some((provider, model)) = id.split_once('/') {
                    self.provider = provider.to_owned();
                    self.model = model.to_owned();
                    self.enter(Step::Review, true);
                }
            }
            Step::ResponseBehavior => {
                self.reasoning_only_text = id == "reasoning-text";
                if matches!(self.mode, SetupMode::FirstRun) {
                    self.make_default = true;
                    self.enter(Step::Review, true);
                } else {
                    self.enter(Step::DefaultChoice, true);
                }
            }
            Step::DefaultChoice => {
                self.make_default = id == "yes";
                self.enter(Step::Review, true);
            }
            _ => {}
        }
        SetupEffect::None
    }

    fn needs_response_behavior(&self) -> bool {
        self.action == Some(SetupAction::AddProvider)
            && self.provider_kind == SetupProviderKind::OpenAiCompatible
    }

    fn after_credential_step(&self) -> Step {
        if self.catalog_models {
            return Step::ModelChoice;
        }
        if matches!(
            self.action,
            Some(SetupAction::QuickGlm | SetupAction::ChangeCredential)
        ) {
            Step::Review
        } else {
            Step::ModelName
        }
    }

    fn submit_input(&mut self) -> SetupEffect {
        let value = self.input.trim().to_owned();
        match self.step {
            Step::ProviderName => {
                if value.is_empty()
                    || value.contains(['/', '\\'])
                    || value.chars().any(char::is_whitespace)
                {
                    self.error = Some(
                        "Use a non-empty provider name without spaces or path separators.".into(),
                    );
                } else {
                    self.provider = value;
                    self.enter(Step::Endpoint, true);
                }
            }
            Step::Endpoint => {
                if !(value.starts_with("https://") || value.starts_with("http://")) {
                    self.error = Some("Enter a complete http:// or https:// API base URL.".into());
                } else {
                    self.endpoint = value;
                    self.enter(Step::CredentialMethod, true);
                }
            }
            Step::CredentialValue => match self.credential_method {
                Some(method) if method.takes_secret() && self.secret.is_empty() => {
                    self.error =
                        Some("Enter an API key or go Back to choose another method.".into());
                }
                Some(method) if method.takes_secret() => {
                    self.enter(self.after_credential_step(), true);
                }
                Some(CredentialMethod::Environment) if !valid_variable(&value) => {
                    self.error = Some(self.prompts.environment_variable_error.clone());
                }
                Some(CredentialMethod::Environment) => {
                    self.environment_variable = value;
                    self.enter(self.after_credential_step(), true);
                }
                _ => {}
            },
            Step::ModelName => {
                if value.is_empty() || value.chars().any(char::is_control) {
                    self.error = Some("Enter the provider's exact model ID.".into());
                } else {
                    self.model = value;
                    if matches!(
                        self.action,
                        Some(SetupAction::AddProvider | SetupAction::AddModel)
                    ) {
                        // Custom models are the only ones whose limits nobody
                        // has reviewed yet, so they are the only ones worth a
                        // bounded read of the endpoint's own advertisement.
                        self.busy_note = Some(
                            "Resolving model limits from the endpoint and trusted catalog…".into(),
                        );
                        self.enter(Step::Busy, true);
                        return SetupEffect::ResolveModelLimits {
                            request: ResolveModelLimits {
                                use_endpoint_listing: self.action != Some(SetupAction::AddProvider)
                                    || self.provider_kind == SetupProviderKind::OpenAiCompatible,
                                endpoint: if self.action == Some(SetupAction::AddProvider)
                                    && self.provider_kind == SetupProviderKind::OpenAiCompatible
                                    && !self.endpoint.is_empty()
                                {
                                    Some(self.endpoint.clone())
                                } else {
                                    None
                                },
                                bearer: match self.credential_method {
                                    Some(method)
                                        if method.takes_secret() && !self.secret.is_empty() =>
                                    {
                                        Some(self.secret.secret())
                                    }
                                    _ => None,
                                },
                                environment_variable: if self.credential_method
                                    == Some(CredentialMethod::Environment)
                                {
                                    Some(self.environment_variable.clone())
                                } else {
                                    None
                                },
                                provider: (!self.provider.is_empty())
                                    .then(|| self.provider.clone()),
                                model: self.model.clone(),
                            },
                        };
                    }
                    self.enter(Step::ContextTokens, true);
                }
            }
            Step::ContextTokens => match positive_u32(&value) {
                Ok(value) => {
                    let unchanged_resolution =
                        self.context_tokens == Some(value) && self.limits_source.is_some();
                    self.context_tokens = Some(value);
                    if !unchanged_resolution {
                        self.max_input_tokens = Some(value);
                        self.max_output_tokens =
                            Some(smith_runtime::probe::derived_output_ceiling(value));
                        self.limits_source =
                            Some("context entered manually · ceilings derived".into());
                    }
                    if self.needs_response_behavior() {
                        self.enter(Step::ResponseBehavior, true);
                    } else {
                        self.enter(Step::DefaultChoice, true);
                    }
                }
                Err(error) => self.error = Some(error),
            },
            Step::Review => {
                let Some(submission) = self.submission() else {
                    self.error =
                        Some("Setup choices are incomplete; go Back and review them.".into());
                    return SetupEffect::None;
                };
                self.step = Step::Busy;
                return SetupEffect::Submit {
                    submission,
                    allow_collisions: self.allow_collisions,
                };
            }
            _ => {}
        }
        SetupEffect::None
    }

    fn submission(&self) -> Option<SetupSubmission> {
        let credential = || match self.credential_method? {
            CredentialMethod::Keychain => {
                Some(SetupCredential::StoreInKeychain(self.secret.secret()))
            }
            CredentialMethod::Config => Some(SetupCredential::StoreInConfig(self.secret.secret())),
            CredentialMethod::ExistingKeychain => Some(SetupCredential::ExistingKeychain),
            CredentialMethod::Environment => Some(SetupCredential::Environment(
                self.environment_variable.clone(),
            )),
        };
        let limits = || {
            Some(SetupModelLimits {
                context_tokens: self.context_tokens?,
                max_input_tokens: self.max_input_tokens?,
                max_output_tokens: self.max_output_tokens?,
            })
        };
        match self.action? {
            SetupAction::QuickGlm => Some(SetupSubmission::QuickGlm {
                credential: credential()?,
            }),
            SetupAction::QuickXai => Some(SetupSubmission::QuickXai {
                credential: credential()?,
                model: self.model.clone(),
            }),
            SetupAction::QuickGoogle => Some(SetupSubmission::QuickGoogle {
                credential: credential()?,
                model: self.model.clone(),
            }),
            SetupAction::AddProvider => Some(SetupSubmission::AddProvider {
                kind: self.provider_kind,
                provider: self.provider.clone(),
                endpoint: self.endpoint.clone(),
                credential: credential()?,
                model: self.model.clone(),
                limits: limits()?,
                reasoning_only_text: self.reasoning_only_text,
                make_default: self.make_default,
            }),
            SetupAction::AddModel => Some(SetupSubmission::AddModel {
                provider: self.provider.clone(),
                model: self.model.clone(),
                limits: limits()?,
                make_default: self.make_default,
            }),
            SetupAction::ChangeDefault => Some(SetupSubmission::ChangeDefault {
                provider: self.provider.clone(),
                model: self.model.clone(),
            }),
            SetupAction::ChangeCredential => Some(SetupSubmission::ChangeCredential {
                provider: self.provider.clone(),
                credential: credential()?,
            }),
        }
    }

    fn credential_reference(&self, provider: &str) -> String {
        match self.credential_method {
            Some(CredentialMethod::Environment) => {
                format!("env:{}", self.environment_variable)
            }
            Some(CredentialMethod::Config) => "api_key = [redacted]".to_owned(),
            _ => format!("keychain:smith/{provider}"),
        }
    }

    fn limits_review(&self) -> String {
        format!(
            "{} ({})",
            compact_limits(SetupModelLimits {
                context_tokens: self.context_tokens.unwrap_or_default(),
                max_input_tokens: self.max_input_tokens.unwrap_or_default(),
                max_output_tokens: self.max_output_tokens.unwrap_or_default(),
            }),
            self.limits_source.as_deref().unwrap_or("explicit limits")
        )
    }

    fn prompt(&self) -> (&'static str, String, bool) {
        match self.step {
            Step::ProviderName => (
                "Provider name",
                self.prompts.provider_name_help.clone(),
                false,
            ),
            Step::Endpoint => (
                "API base URL",
                self.prompts.endpoint_help.clone(),
                false,
            ),
            Step::CredentialValue
                if self
                    .credential_method
                    .is_some_and(CredentialMethod::takes_secret) =>
            {
                (
                    "API key",
                    if self.credential_method == Some(CredentialMethod::Config) {
                        "Plaintext in owner-only config; readable by same-user processes and backups"
                    } else {
                        "Stored only in the platform credential service"
                    }
                    .to_owned(),
                    true,
                )
            }
            Step::CredentialValue => (
                "Environment variable",
                "Smith records the name only and does not read its value during setup".to_owned(),
                false,
            ),
            Step::ModelName => (
                "Model ID",
                "Exact identifier; limits resolve automatically when the endpoint or catalog publishes them"
                    .to_owned(),
                false,
            ),
            Step::ContextTokens => (
                "Model context window",
                match self.limits_source.as_deref() {
                    Some(source) => format!(
                        "Resolved from {source} · edit only to override; input/output ceilings follow automatically"
                    ),
                    None => {
                        "Not published by the endpoint or catalog · input/output ceilings are derived"
                            .to_owned()
                    }
                },
                false,
            ),
            _ => ("", String::new(), false),
        }
    }
}

impl Screen for SetupApp {
    type Outcome = ();
    type Effect = SetupEffect;

    fn draw(&self, frame: &mut Frame<'_>, area: Rect, theme: Theme) {
        draw_setup_in_area(frame, area, self, theme);
    }

    fn on_event(&mut self, event: ScreenEvent) -> ScreenStep<Self::Outcome, Self::Effect> {
        match event {
            ScreenEvent::Key(key) => match self.on_key(key) {
                SetupEffect::None => ScreenStep::Pending,
                SetupEffect::Cancel => ScreenStep::Outcome(()),
                effect => ScreenStep::Effect(effect),
            },
            ScreenEvent::Paste(text) => {
                self.on_paste(&text);
                ScreenStep::Pending
            }
            ScreenEvent::Resize(_, _) | ScreenEvent::Tick => ScreenStep::Pending,
        }
    }
}

/// Draws the complete setup surface.
pub fn draw_setup(frame: &mut Frame<'_>, app: &SetupApp, theme: Theme) {
    let area = frame.area();
    app.draw(frame, area, theme);
}

fn draw_setup_in_area(frame: &mut Frame<'_>, area: Rect, app: &SetupApp, theme: Theme) {
    let back = !app.history.is_empty();
    if let Some(picker) = &app.picker {
        let title = if app.step == Step::Action {
            "Smith setup · Welcome · choose how to connect a model".to_owned()
        } else {
            format!("Smith setup · {}", picker.title)
        };
        draw_picker_with_context(
            frame,
            area,
            picker,
            &title,
            (app.step == Step::Action)
                .then_some("Nothing is sent to a provider until setup finishes."),
            app.error.as_deref(),
            theme,
        );
        return;
    }
    let mut lines = Vec::new();
    let heading = match app.step {
        Step::Review => "Review · nothing is written until you confirm",
        Step::Busy => "Applying setup",
        _ => app.prompt().0,
    };
    match app.step {
        Step::Review | Step::Busy => {
            if app.step == Step::Busy {
                lines.extend(indented_words(
                    app.busy_note()
                        .unwrap_or("Writing your choices and checking the connection…"),
                    area.width,
                    2,
                    Tone::Accent,
                    theme,
                ));
                lines.push(Line::default());
            }
            lines.extend(app.wrapped_review_lines(area.width));
        }
        _ => {
            let (_, help, masked) = app.prompt();
            if app.error.is_none() {
                lines.extend(indented_words(&help, area.width, 2, Tone::Dim, theme));
                lines.push(Line::default());
            }
            let value = if masked {
                app.secret.masked()
            } else if app.input.is_empty() {
                "type a value".to_owned()
            } else {
                app.input.clone()
            };
            lines.push(Line::from(vec![
                Span::styled("› ", theme.style(Tone::Accent)),
                Span::styled(
                    value,
                    theme.style(if !masked && app.input.is_empty() {
                        Tone::Dim
                    } else {
                        Tone::Default
                    }),
                ),
            ]));
        }
    }
    if let Some(error) = &app.error {
        lines.push(Line::default());
        lines.extend(indented_words(
            &format!("error: {error}"),
            area.width,
            2,
            Tone::Danger,
            theme,
        ));
    }
    let rows = crate::render::wrap::wrap_lines(&lines, area.width);
    let mut footer = match app.step {
        Step::Review => ScreenFooter::Review { back, scroll: None },
        Step::Busy => ScreenFooter::Busy { back },
        _ => ScreenFooter::Field { back },
    };
    let base_page = usize::from(area.height).saturating_sub(3 + footer.rows(area.width).len());
    if app.step == Step::Review {
        if rows.len() > base_page {
            footer = ScreenFooter::Review {
                back,
                scroll: Some((1, 1)),
            };
        }
        let page = rows
            .len()
            .min(usize::from(area.height).saturating_sub(3 + footer.rows(area.width).len()));
        let limit = rows.len().saturating_sub(page);
        let offset = app.review_scroll.get().offset.min(limit);
        app.review_scroll.set(ReviewScroll {
            offset,
            limit,
            page,
        });
        footer = ScreenFooter::Review {
            back,
            scroll: (limit > 0).then_some((offset + 1, limit + 1)),
        };
        let body = draw_inline_surface(
            frame,
            area,
            &format!("Smith setup · {heading}"),
            rows.len(),
            footer,
            theme,
        );
        frame.render_widget(
            Paragraph::new(
                rows.into_iter()
                    .skip(offset)
                    .take(usize::from(body.height))
                    .collect::<Vec<_>>(),
            ),
            body,
        );
    } else {
        let body = draw_inline_surface(
            frame,
            area,
            &format!("Smith setup · {heading}"),
            rows.len(),
            footer,
            theme,
        );
        frame.render_widget(Paragraph::new(rows), body);
    }
}

fn valid_variable(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(|character: char| character.is_ascii_digit())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn positive_u32(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "Enter a positive whole token count.".to_owned())
}

/// Uses the same token precision as model pickers so setup and runtime agree.
fn compact_limits(limits: SetupModelLimits) -> String {
    format!(
        "{} context · {} input · {} output",
        compact_tokens(u64::from(limits.context_tokens)),
        compact_tokens(u64::from(limits.max_input_tokens)),
        compact_tokens(u64::from(limits.max_output_tokens)),
    )
}

/// Removes old label prefixes from supplied facts before applying aligned labels.
fn review_value(line: &str) -> &str {
    line.split_once(": ").map_or(line, |(_, value)| value)
}

/// Shortens only home descendants, keeping sibling names and outside paths accurate.
fn review_destination(destination: &str, home: Option<&Path>) -> String {
    let path = Path::new(destination);
    if let Some(home) = home.filter(|home| !home.as_os_str().is_empty())
        && let Ok(relative) = path.strip_prefix(home)
    {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    destination.to_owned()
}

fn bound(mut value: String, limit: usize) -> String {
    if value.len() > limit {
        // The budget is bytes, but the cut must not land inside a character:
        // `String::truncate` panics on a non-boundary offset, and a long
        // Chinese preview reaches the limit mid-character more often than not.
        let mut end = limit;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn setup_app(
        mode: SetupMode,
        providers: Vec<ResourceEntry>,
        models: Vec<ResourceEntry>,
    ) -> SetupApp {
        let quick_start = SetupQuickStart {
            provider: "zai".into(),
            endpoint: "https://api.z.ai/api/coding/paas/v4".into(),
            model: "glm-5.2".into(),
            model_label: "GLM-5.2".into(),
            limits: SetupModelLimits {
                context_tokens: 1_000_000,
                max_input_tokens: 1_000_000,
                max_output_tokens: 131_072,
            },
            request_output_tokens: 32_768,
            output_reserve: 32_768,
            profile: "glm".into(),
            catalog_revision: 5,
        };
        let entries = setup_entries(&quick_start);
        SetupApp::new(
            mode,
            providers,
            models,
            quick_start,
            entries,
            setup_prompts(),
        )
    }

    fn setup_prompts() -> SetupPrompts {
        SetupPrompts {
            provider_name_help: "A stable local name, for example openrouter".into(),
            endpoint_help: "OpenAI-compatible base, for example https://openrouter.ai/api/v1"
                .into(),
            environment_variable_error:
                "Use an environment variable such as ZAI_API_KEY (letters, digits, underscore)."
                    .into(),
        }
    }

    fn setup_entries(quick_start: &SetupQuickStart) -> Vec<SetupEntry> {
        vec![
            SetupEntry {
                id: "glm".into(),
                label: "Quick start with GLM".into(),
                detail: format!("Z.AI · {}", quick_start.model_label),
                flow: SetupFlow::QuickKey {
                    kind: SetupQuickKey::Glm,
                    provider: quick_start.provider.clone(),
                    endpoint: quick_start.endpoint.clone(),
                    review: SetupKeyReview {
                        action: "action: Quick start with GLM".into(),
                        provider: format!("provider: {} (openai-compatible)", quick_start.provider),
                        endpoint: format!("endpoint: {}", quick_start.endpoint),
                        profile: format!("default profile: {}", quick_start.profile),
                        reasoning: None,
                    },
                    catalog_models: false,
                },
            },
            custom_entry("add-provider", "Add provider", None, None, SetupProviderKind::OpenAiCompatible, false),
            SetupEntry {
                id: "google".into(),
                label: "Connect Google Gemini".into(),
                detail: "AI Studio API key · native Gemini Interactions".into(),
                flow: SetupFlow::QuickKey {
                    kind: SetupQuickKey::Google,
                    provider: "google".into(),
                    endpoint: "https://generativelanguage.googleapis.com/v1beta".into(),
                    review: SetupKeyReview {
                        action: "action: Connect Google Gemini".into(),
                        provider: "provider: google (native gemini-interactions)".into(),
                        endpoint: "endpoint: fixed Google Gemini Interactions endpoint".into(),
                        profile: "default profile: gemini".into(),
                        reasoning: Some("reasoning: native Gemini thinking levels from the selected catalog model".into()),
                    },
                    catalog_models: false,
                },
            },
            custom_entry("anthropic-messages", "Anthropic Messages API", Some("anthropic"), Some("https://api.anthropic.com/v1"), SetupProviderKind::AnthropicMessages, false),
            custom_entry("openrouter", "Connect OpenRouter", Some("openrouter"), Some("https://openrouter.ai/api/v1"), SetupProviderKind::OpenAiCompatible, false),
        ]
    }

    fn custom_entry(
        id: &str,
        label: &str,
        provider: Option<&str>,
        endpoint: Option<&str>,
        kind: SetupProviderKind,
        catalog_models: bool,
    ) -> SetupEntry {
        let (review_action, adapter) = match kind {
            SetupProviderKind::OpenAiCompatible => (
                "action: Add OpenAI-compatible provider",
                "openai-compatible",
            ),
            SetupProviderKind::AnthropicMessages => (
                "action: Add Anthropic Messages provider",
                "anthropic-messages",
            ),
        };
        SetupEntry {
            id: id.into(),
            label: label.into(),
            detail: String::new(),
            flow: SetupFlow::CustomEndpoint {
                kind,
                provider: provider.map(str::to_owned),
                endpoint: endpoint.map(str::to_owned),
                review_action: review_action.into(),
                adapter: adapter.into(),
                catalog_models,
            },
        }
    }

    fn direct_provider_mode(id: &str) -> SetupMode {
        let app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        let mut flow = app
            .provider_actions
            .into_iter()
            .find(|entry| entry.id == id)
            .expect("test entry")
            .flow;
        match &mut flow {
            SetupFlow::QuickKey { catalog_models, .. }
            | SetupFlow::CustomEndpoint { catalog_models, .. } => *catalog_models = true,
            _ => panic!("the direct test mode needs a catalog model"),
        }
        SetupMode::Provider { flow }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn escape_from_key_field_restores_the_chosen_credential_method() {
        for method in ["keychain", "config", "environment"] {
            let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
            choose(&mut app, "glm");
            choose(&mut app, method);
            assert_eq!(app.step, Step::CredentialValue);
            assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::None));
            assert_eq!(app.step, Step::CredentialMethod);
            assert_eq!(
                app.picker
                    .as_ref()
                    .and_then(ResourcePicker::selected_entry)
                    .map(|entry| entry.id.as_str()),
                Some(method)
            );
            assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::None));
            assert_eq!(app.step, Step::Action);
            assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::Cancel));
        }
    }

    #[test]
    fn ctrl_c_cancels_review_and_busy_steps_without_submitting() {
        let mut review = glm_environment_review();
        assert!(matches!(
            review.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            SetupEffect::Cancel
        ));
        review.step = Step::Busy;
        assert!(matches!(
            review.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            SetupEffect::Cancel
        ));
    }

    #[test]
    fn numbered_setup_choices_ignore_letters_and_select_with_digits() {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        app.on_paste("google");
        app.on_key(key(KeyCode::Char('g')));
        assert!(app.picker.as_ref().expect("action picker").query.is_empty());
        app.on_key(key(KeyCode::Char('1')));
        assert_eq!(app.step, Step::CredentialMethod);
        app.on_key(key(KeyCode::Char('3')));
        assert_eq!(app.step, Step::CredentialValue);
        assert_eq!(app.credential_method, Some(CredentialMethod::Config));
    }

    #[test]
    fn bound_cuts_on_a_character_boundary_instead_of_panicking() {
        // 重 is three bytes, so a 1_024-byte budget lands inside the 342nd
        // character — exactly where String::truncate would panic. Chinese
        // error text and collision previews reach this path for real.
        let long = "重".repeat(400);
        let bounded = bound(long, 1_024);
        assert!(bounded.ends_with('…'), "{bounded}");
        assert_eq!(bounded.trim_end_matches('…').chars().count(), 341);
    }

    fn choose(app: &mut SetupApp, id: &str) {
        app.select_picker(id.to_owned());
    }

    fn render_setup(app: &SetupApp, width: u16, height: u16) -> String {
        setup_screen(&render_setup_buffer(
            app,
            width,
            height,
            Theme::new().without_color().without_motion(),
        ))
    }

    fn render_setup_buffer(
        app: &SetupApp,
        width: u16,
        height: u16,
        theme: Theme,
    ) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| draw_setup(frame, app, theme))
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    fn setup_screen(buffer: &ratatui::buffer::Buffer) -> String {
        // Read glyphs, not cells: the trailing cell of a wide character is
        // stored blank, so cell-by-cell collection garbles Chinese content.
        (0..buffer.area.height)
            .map(|y| {
                crate::selection::glyph_bounds(buffer, buffer.area, y)
                    .map(|(x, _)| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn setup_body_rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
        let mut rows = setup_screen(buffer)
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        while rows.last().is_some_and(|row| row.trim().is_empty()) {
            rows.pop();
        }
        rows
    }

    #[test]
    fn setup_steps_start_at_the_left_gutter_without_a_frame() {
        let action = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        let mut authentication = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut authentication, "glm");
        let field = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
        let review = glm_environment_review();
        let mut busy = glm_environment_review();
        busy.on_key(key(KeyCode::Enter));
        for app in [action, authentication, field, review, busy] {
            for (width, height) in [(44, 16), (80, 24), (100, 32)] {
                let rendered = render_setup(&app, width, height);
                assert_eq!(rendered.matches("Smith setup").count(), 1, "{rendered}");
                assert!(
                    rendered
                        .lines()
                        .next()
                        .expect("title row")
                        .starts_with("  Smith setup"),
                    "{rendered}"
                );
                for frame_glyph in ['┌', '┐', '└', '┘', '│'] {
                    assert!(!rendered.contains(frame_glyph), "{rendered}");
                }
            }
        }
    }

    #[test]
    fn setup_descriptions_wrap_whole_words_beneath_the_name() {
        let description = "Review endpoint credentials and 中文 model limits before applying this connection. Every word remains available beneath the selected name.";
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        let mut entry = app.provider_actions[0].clone();
        entry.detail = description.into();
        app = app.with_provider_actions(vec![entry.clone()]);
        for width in [44, 80] {
            let buffer = render_setup_buffer(&app, width, 32, Theme::new().without_color());
            let rendered = setup_screen(&buffer);
            let rows = setup_body_rows(&buffer);
            let name = rows
                .iter()
                .position(|row| row.trim_end() == format!("❯ 1. {}", entry.label))
                .expect("the name occupies its own line");
            let description_rows = rows[name + 1..]
                .iter()
                .take_while(|row| row.starts_with("     "))
                .filter(|row| !row.trim().is_empty())
                .collect::<Vec<_>>();
            assert!(description_rows.len() > 1, "{rendered}");
            assert_eq!(
                description_rows
                    .iter()
                    .map(|row| row.trim())
                    .collect::<Vec<_>>()
                    .join(" "),
                description,
                "description was split or truncated:\n{rendered}"
            );
            for row in &description_rows {
                assert_eq!(
                    row.chars()
                        .take_while(|character| *character == ' ')
                        .count(),
                    5,
                    "descriptions start beneath the numbered label:\n{rendered}"
                );
                for word in row.split_whitespace() {
                    assert!(
                        description.split_whitespace().any(|whole| whole == word),
                        "{rendered}"
                    );
                }
            }
            let inner = buffer.area;
            for row in 0..description_rows.len() {
                let y = inner.y + u16::try_from(name + 1 + row).expect("description row");
                assert!(
                    buffer[(inner.x + 5, y)]
                        .modifier
                        .contains(ratatui::style::Modifier::DIM)
                );
            }
        }
    }

    #[test]
    fn overflowing_setup_entries_scroll_without_displacing_the_footer() {
        let description = "Keep the endpoint credentials and model available for local review before applying the change.";
        for (width, height, footer_rows) in [(44, 16, 2), (80, 24, 1)] {
            let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
            let prototype = app.provider_actions[0].clone();
            let entries = (1..=20)
                .map(|index| SetupEntry {
                    id: format!("choice-{index:02}"),
                    label: format!("Choice {index:02}"),
                    detail: description.into(),
                    flow: prototype.flow.clone(),
                })
                .collect::<Vec<_>>();
            app = app.with_provider_actions(entries);
            for selected in 1..=20 {
                let buffer = render_setup_buffer(&app, width, height, Theme::new().without_color());
                let rendered = setup_screen(&buffer);
                let rows = setup_body_rows(&buffer);
                let footer = rows[rows.len() - footer_rows..].join("\n");
                for control in ["↑↓ or 1–9 choose", "enter confirm", "esc cancel"] {
                    assert!(footer.contains(control), "{width}×{height}: {rendered}");
                }
                assert_eq!(rendered.matches('❯').count(), 1, "{rendered}");
                let name = rows
                    .iter()
                    .position(|row| row.trim_end() == format!("❯ {selected}. Choice {selected:02}"))
                    .expect("the selected entry remains visible");
                let description_rows = rows[name + 1..]
                    .iter()
                    .take_while(|row| row.starts_with("     "))
                    .filter(|row| !row.trim().is_empty())
                    .collect::<Vec<_>>();
                assert_eq!(
                    description_rows
                        .iter()
                        .map(|row| row.trim())
                        .collect::<Vec<_>>()
                        .join(" "),
                    description,
                    "{width}×{height} omitted part of the selected description:\n{rendered}"
                );
                let visible = rows
                    .iter()
                    .filter_map(|row| {
                        row.trim_start_matches("❯ ")
                            .trim()
                            .split_once(". ")
                            .and_then(|(_, label)| label.strip_prefix("Choice "))
                    })
                    .map(|number| number.parse::<usize>().expect("entry number"))
                    .collect::<Vec<_>>();
                assert!(visible.len() < 20, "{rendered}");
                assert!(
                    visible.windows(2).all(|pair| pair[0] < pair[1]),
                    "{rendered}"
                );
                app.on_key(key(KeyCode::Down));
            }
            let wrapped = render_setup(&app, width, height);
            assert!(wrapped.contains("❯ 1. Choice 01"), "{wrapped}");
            app.on_key(key(KeyCode::Up));
            let last = render_setup(&app, width, height);
            assert!(last.contains("❯ 20. Choice 20"), "{last}");
        }
    }

    #[test]
    fn setup_no_color_preserves_selection_description_and_controls() {
        use ratatui::style::{Color, Modifier};

        let app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        for (width, height) in [(44, 16), (80, 24)] {
            let colored = render_setup_buffer(&app, width, height, Theme::new());
            let plain = render_setup_buffer(&app, width, height, Theme::new().without_color());
            assert_eq!(setup_screen(&colored), setup_screen(&plain));
            assert!(
                plain
                    .content
                    .iter()
                    .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
            );
            let inner = plain.area;
            let rows = setup_body_rows(&plain);
            let name = rows
                .iter()
                .position(|row| row.starts_with("❯ "))
                .expect("selection marker");
            let y = inner.y + u16::try_from(name).expect("name row");
            assert!(plain[(inner.x, y)].modifier.contains(Modifier::BOLD));
            assert!(rows[name + 1].starts_with("    "));
            assert!(plain[(inner.x + 5, y + 1)].modifier.contains(Modifier::DIM));
            assert!(rows.iter().any(|row| row.contains("esc cancel")));
        }
    }

    #[test]
    fn setup_filtering_and_empty_guidance_keep_the_footer() {
        let mut app = setup_app(
            SetupMode::AddModel { provider: None },
            vec![
                ResourceEntry::new("google", "Google Gemini", "native endpoint"),
                ResourceEntry::new("zai", "Z.AI", "configured"),
            ],
            Vec::new(),
        );
        app.on_paste("google");
        let filtered = render_setup(&app, 44, 16);
        assert!(filtered.contains("❯ Google Gemini"), "{filtered}");
        assert!(!filtered.contains("Z.AI"), "{filtered}");
        app.on_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        app.on_paste("no-such-entry");
        let missing = render_setup(&app, 44, 16);
        for text in [
            "No matches",
            "Ctrl+U clear filter",
            "ctrl+u clear filter",
            "esc cancel",
        ] {
            assert!(missing.contains(text), "{missing}");
        }
        assert!(!missing.contains("enter confirm"), "{missing}");
        assert!(!missing.contains("↑↓ choose"), "{missing}");
        let empty = render_setup(
            &setup_app(SetupMode::FirstRun, Vec::new(), Vec::new())
                .with_provider_actions(Vec::new()),
            44,
            16,
        );
        assert!(empty.contains("No setup actions are available."), "{empty}");
        assert!(empty.contains("esc cancel"), "{empty}");
    }

    #[test]
    fn setup_picker_states_and_selected_detail_remain_visible() {
        let entries = vec![
            ResourceEntry::new("active", "Active provider", "Native endpoint").active(true),
            ResourceEntry::new(
                "unavailable",
                "Unavailable provider",
                "Full provider metadata",
            )
            .description("Custom endpoint")
            .active(true)
            .disabled("Adapter unavailable in this build"),
        ];
        let mut app = setup_app(SetupMode::AddModel { provider: None }, entries, Vec::new());
        app.on_key(key(KeyCode::Down));
        let rendered = render_setup(&app, 80, 24);
        for text in [
            "Active provider",
            "❯ Unavailable provider",
            "✓ current",
            "unavailable",
            "Custom endpoint",
            "Full provider metadata",
            "Adapter unavailable in this build",
        ] {
            assert!(rendered.contains(text), "{rendered}");
        }
        assert!(matches!(app.on_key(key(KeyCode::Enter)), SetupEffect::None));
        assert_eq!(app.step, Step::ProviderChoice);
    }

    fn glm_environment_review() -> SetupApp {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new())
            .with_destination("/tmp/smith-home/.smith/config.toml");
        choose(&mut app, "glm");
        choose(&mut app, "environment");
        for character in "ZAI_API_KEY".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        app
    }

    #[test]
    fn setup_welcome_and_intro_precede_the_list_at_every_width() {
        let app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        for width in [20, 44, 100] {
            let rendered = render_setup(&app, width, 24);
            let rows = rendered.lines().collect::<Vec<_>>();
            assert!(rows[0].starts_with("  Smith setup"), "{rendered}");
            assert!(rows[1].starts_with("  Nothing"), "{rendered}");
            let choice = rows
                .iter()
                .position(|row| row.starts_with("❯ "))
                .expect("first choice");
            let intro = rows[1..choice]
                .iter()
                .map(|row| row.trim())
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(intro, "Nothing is sent to a provider until setup finishes.");
            assert!(!rendered.contains("Shift+Tab"), "{rendered}");
            assert!(!rendered.contains("no agent session"), "{rendered}");
            let plain = render_setup_buffer(&app, width, 24, Theme::new().without_color());
            assert!(
                plain[(2, 1)]
                    .modifier
                    .contains(ratatui::style::Modifier::DIM)
            );
        }
        assert!(
            render_setup(&app, 100, 24)
                .contains("Smith setup · Welcome · choose how to connect a model")
        );
    }

    #[test]
    fn credential_methods_name_the_provider_and_field_help_starts_at_the_gutter() {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "glm");
        choose(&mut app, "existing-keychain");
        app.on_key(key(KeyCode::Esc));
        let rendered = render_setup(&app, 100, 24);
        assert!(rendered.contains("keychain:smith/zai"), "{rendered}");
        assert!(!rendered.contains("<provider>"), "{rendered}");
        choose(&mut app, "keychain");
        let rendered = render_setup(&app, 44, 16);
        let rows = rendered.lines().collect::<Vec<_>>();
        assert!(rows[2].starts_with("  Stored"), "{rendered}");
        assert!(rows[3].starts_with("  "), "{rendered}");
        assert!(
            !rows[3].trim().is_empty(),
            "help wraps at the gutter: {rendered}"
        );
    }

    #[test]
    fn review_labels_align_and_keep_compact_limits_and_all_reviewed_facts() {
        let mut app = glm_environment_review();
        let home = std::env::var_os("HOME").expect("test HOME");
        app = app.with_destination(
            Path::new(&home)
                .join(".smith/config.toml")
                .to_string_lossy(),
        );
        let rows = app.review_lines();
        let labels = [
            "Provider",
            "API key",
            "Model",
            "Requests",
            "GLM replies",
            "Default",
            "Writes",
        ];
        assert_eq!(rows.len(), labels.len());
        for (row, label) in rows.iter().zip(labels) {
            assert_eq!(&row[..13], format!("{label:11}  "));
        }
        assert_eq!(
            rows[0],
            "Provider     zai · https://api.z.ai/api/coding/paas/v4"
        );
        assert_eq!(rows[1], "API key      env:ZAI_API_KEY");
        assert_eq!(
            rows[2],
            "Model        glm-5.2 · 1M context · 1M input · 131k output · trusted catalog v5"
        );
        assert_eq!(rows[3], "Requests     32.7k output · 32.7k reserved");
        assert!(rows[4].ends_with(
            "an answer sent only as reasoning is shown as the reply; thinking stays on"
        ));
        assert_eq!(rows[5], "Default      profile glm");
        assert_eq!(
            rows[6],
            "Writes       ~/.smith/config.toml, then checks the connection"
        );
        let text = rows.join("\n");
        for old in [
            "1000000",
            "131072",
            "32768",
            "pending action",
            "local preflight",
        ] {
            assert!(!text.contains(old), "{text}");
        }
        assert_eq!(
            review_destination(
                "/tmp/injected-home/.smith/config.toml",
                Some(Path::new("/tmp/injected-home"))
            ),
            "~/.smith/config.toml"
        );
        assert_eq!(
            review_destination(
                "/tmp/injected-home-other/config.toml",
                Some(Path::new("/tmp/injected-home"))
            ),
            "/tmp/injected-home-other/config.toml"
        );
        assert_eq!(
            review_destination("~/.smith/config.toml", None),
            "~/.smith/config.toml"
        );
        let rendered = render_setup(&app, 100, 32);
        assert!(
            rendered.contains("Review · nothing is written until you confirm"),
            "{rendered}"
        );
        assert!(!rendered.contains("Review your choices"), "{rendered}");
    }

    #[test]
    fn setup_review_values_hang_indent_and_wrap_words_at_44_and_80_columns() {
        let destination = format!("/tmp/{}config.toml", "reviewed-directory/".repeat(12));
        let app = glm_environment_review().with_destination(destination.clone());
        let labels = [
            "Provider",
            "API key",
            "Model",
            "Requests",
            "GLM replies",
            "Default",
            "Writes",
        ];
        let value_column = 15;
        for width in [44, 80] {
            let rendered = render_setup(&app, width, 64);
            let rows = rendered.lines().collect::<Vec<_>>();
            assert!(!rendered.contains("Review your choices"), "{rendered}");
            assert!(rows[2].starts_with("  Provider"), "{rendered}");
            let starts = labels
                .iter()
                .map(|label| {
                    rows.iter()
                        .position(|row| row.starts_with(&format!("  {label:11}  ")))
                        .expect("labelled review row")
                })
                .collect::<Vec<_>>();
            for (index, start) in starts.iter().copied().enumerate() {
                let end = starts.get(index + 1).copied().unwrap_or(rows.len());
                for row in rows[start + 1..end]
                    .iter()
                    .take_while(|row| !row.trim().is_empty())
                {
                    assert!(row.starts_with(&" ".repeat(value_column)), "{rendered}");
                    assert!(!row[value_column..].starts_with(' '), "{rendered}");
                }
            }
            let model_rows = &rows[starts[2]..starts[3]];
            assert!(model_rows.len() > 1, "{rendered}");
            assert_eq!(
                model_rows
                    .iter()
                    .map(|row| row[value_column..].trim_end())
                    .collect::<Vec<_>>()
                    .join(" "),
                "glm-5.2 · 1M context · 1M input · 131k output · trusted catalog v5",
                "{rendered}"
            );
            if width == 80 {
                assert_eq!(model_rows.last().expect("model continuation").trim(), "v5");
            }
            let value_width = usize::from(width) - value_column;
            for chunk in 0..3 {
                let row = rows[starts[6] + chunk];
                assert_eq!(
                    &row[value_column..],
                    &destination[chunk * value_width..(chunk + 1) * value_width],
                    "long path must use the full value width: {rendered}"
                );
            }
            let writes = rows[starts[6]..]
                .iter()
                .take_while(|row| !row.trim().is_empty())
                .map(|row| row[value_column..].trim_end())
                .collect::<Vec<_>>();
            assert_eq!(
                writes
                    .concat()
                    .split_once(',')
                    .expect("destination comma")
                    .0,
                destination,
                "long path must retain every character: {rendered}"
            );
            assert!(
                writes.join(" ").contains("then checks the connection"),
                "{rendered}"
            );
        }
    }

    #[test]
    fn setup_review_scrolls_to_the_last_wrapped_row_with_fixed_footer_keys() {
        for (width, height) in [(44, 16), (80, 24)] {
            for collisions in [false, true] {
                for down in [KeyCode::Down, KeyCode::PageDown] {
                    let mut app = glm_environment_review();
                    if collisions {
                        let mut preview = (0..60)
                            .map(|line| format!("merge line {line}: replace reviewed value"))
                            .collect::<Vec<_>>();
                        preview.push("merge-final".to_owned());
                        app.review_collisions(preview.join("\n"));
                    } else {
                        app = app.with_destination(format!(
                            "/tmp/{}/config.toml",
                            "reviewed-directory/".repeat(90),
                        ));
                    }
                    let initial = render_setup(&app, width, height);
                    let initial_scroll = app.review_scroll.get();
                    assert!(initial_scroll.limit > 0, "{initial}");
                    assert!(initial.contains("↑↓/PgUp/PgDn review · 1/"), "{initial}");
                    let error = app.error.clone();
                    app.on_key(key(down));
                    let expected = if down == KeyCode::Down {
                        1
                    } else {
                        initial_scroll.page
                    };
                    assert_eq!(
                        app.review_scroll.get().offset,
                        expected.min(initial_scroll.limit)
                    );
                    for _ in 0..initial_scroll.limit {
                        let buffer =
                            render_setup_buffer(&app, width, height, Theme::new().without_color());
                        let rows = setup_body_rows(&buffer);
                        let footer = rows[rows.len() - 2..].join("\n");
                        assert!(footer.contains("enter confirm"), "{footer}");
                        assert!(footer.contains("esc back"), "{footer}");
                        app.on_key(key(down));
                    }
                    let last = render_setup(&app, width, height);
                    let scroll = app.review_scroll.get();
                    assert_eq!(scroll.offset, scroll.limit);
                    assert_eq!(app.error, error, "scrolling must preserve merge warnings");
                    assert!(
                        last.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .contains("then checks the connection"),
                        "{last}"
                    );
                    if collisions {
                        assert!(last.contains("merge-final"), "{last}");
                        assert!(last.contains("only those values."), "{last}");
                    }
                    assert!(last.contains(&format!("{}/{}", scroll.limit + 1, scroll.limit + 1)));
                    app.on_key(key(KeyCode::PageUp));
                    assert_eq!(
                        app.review_scroll.get().offset,
                        scroll.limit.saturating_sub(scroll.page)
                    );
                    let offset = app.review_scroll.get().offset;
                    app.on_key(key(KeyCode::Up));
                    assert_eq!(app.review_scroll.get().offset, offset.saturating_sub(1));
                    for _ in 0..scroll.limit {
                        app.on_key(key(KeyCode::PageUp));
                    }
                    assert_eq!(app.review_scroll.get().offset, 0);
                    let top = render_setup(&app, width, height);
                    assert!(
                        top.lines().any(|row| row.starts_with("  Provider")),
                        "{top}"
                    );
                    assert!(matches!(
                        app.on_key(key(KeyCode::Enter)),
                        SetupEffect::Submit { allow_collisions, .. } if allow_collisions == collisions
                    ));
                }
            }
        }
    }

    #[test]
    fn setup_review_scroll_clamps_on_resize_and_resets_after_back_or_new_preview() {
        let mut app = glm_environment_review();
        app.review_collisions("merge line\n".repeat(60));
        render_setup(&app, 44, 16);
        for _ in 0..100 {
            app.on_key(key(KeyCode::PageDown));
        }
        let narrow = app.review_scroll.get();
        render_setup(&app, 80, 24);
        let wide = app.review_scroll.get();
        assert!(wide.limit < narrow.limit);
        assert_eq!(wide.offset, wide.limit);
        app.review_collisions("replacement preview");
        assert_eq!(app.review_scroll.get().offset, 0);
        render_setup(&app, 44, 16);
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.review_scroll.get().offset, 0);
        assert!(app.collision_preview.is_none());
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            SetupEffect::Cancel
        ));
    }

    #[test]
    fn glm_funnel_reaches_a_non_secret_review_and_submission() {
        let mut app = glm_environment_review();
        assert_eq!(app.step, Step::Review);
        let review = app.review_lines().join("\n");
        assert!(review.contains("glm-5.2"));
        assert!(review.contains("env:ZAI_API_KEY"));
        assert!(review.contains("an answer sent only as reasoning is shown as the reply"));
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            SetupEffect::Submit {
                submission: SetupSubmission::QuickGlm {
                    credential: SetupCredential::Environment(variable)
                },
                allow_collisions: false,
            } if variable == "ZAI_API_KEY"
        ));
    }

    #[test]
    fn quick_start_menu_and_review_use_supplied_values() {
        let mut data = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new()).quick_start;
        data.model = "glm-reviewed".into();
        data.model_label = "GLM Reviewed".into();
        data.catalog_revision = 99;
        data.limits.context_tokens = 123_456;
        data.request_output_tokens = 4_000;
        data.output_reserve = 5_000;
        let entries = setup_entries(&data);
        let mut app = SetupApp::new(
            SetupMode::FirstRun,
            Vec::new(),
            Vec::new(),
            data,
            entries,
            setup_prompts(),
        );
        let menu = render_setup(&app, 110, 34);
        assert!(menu.contains("Z.AI · GLM Reviewed"), "{menu}");
        choose(&mut app, "glm");
        choose(&mut app, "existing-keychain");
        let review = app.review_lines().join("\n");
        for value in [
            "glm-reviewed",
            "trusted catalog v99",
            "123.4k context",
            "4k output",
            "5k reserved",
        ] {
            assert!(review.contains(value), "missing {value}: {review}");
        }
    }

    #[test]
    fn anthropic_reuses_credentials_limits_and_review_with_its_native_kind() {
        for resolved in [
            None,
            Some(ResolvedModelLimits {
                context_tokens: 200_000,
                max_input_tokens: 190_000,
                max_output_tokens: 10_000,
                source: "trusted catalog match anthropic/claude-reviewed".into(),
            }),
        ] {
            let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
            choose(&mut app, "anthropic-messages");
            assert_eq!(app.step, Step::CredentialMethod);
            assert_eq!(app.provider, "anthropic");
            choose(&mut app, "config");
            app.on_paste("sk-anthropic-test-only");
            assert!(!render_setup(&app, 110, 34).contains("sk-anthropic-test-only"));
            app.on_key(key(KeyCode::Enter));
            assert_eq!(app.step, Step::ModelName);
            app.on_paste("claude-reviewed");
            let SetupEffect::ResolveModelLimits { request } = app.on_key(key(KeyCode::Enter))
            else {
                panic!("Anthropic uses the shared model-limit resolver");
            };
            assert!(!request.use_endpoint_listing);
            app.apply_resolved_limits(resolved);
            if app.step == Step::ContextTokens {
                app.on_paste("64000");
                app.on_key(key(KeyCode::Enter));
            }
            assert_eq!(app.step, Step::DefaultChoice);
            choose(&mut app, "yes");
            let review = app.review_lines().join("\n");
            assert!(
                review.lines().any(|line| line.starts_with("Connection")
                    && line.contains("Anthropic Messages API")),
                "{review}"
            );
            assert!(review.contains("https://api.anthropic.com/v1"), "{review}");
            assert!(review.contains("anthropic/claude-reviewed"), "{review}");
            assert!(!review.contains("sk-anthropic-test-only"), "{review}");
            app.on_key(key(KeyCode::BackTab));
            app.on_key(key(KeyCode::BackTab));
            assert_eq!(app.step, Step::ContextTokens);
            assert!(!app.input.is_empty(), "Back retains non-secret context");
            app.on_key(key(KeyCode::Enter));
            choose(&mut app, "yes");
            assert!(matches!(
                app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
                SetupEffect::Cancel
            ));
            assert!(matches!(
                app.on_key(key(KeyCode::Enter)),
                SetupEffect::Submit {
                    submission: SetupSubmission::AddProvider {
                        kind: SetupProviderKind::AnthropicMessages,
                        credential: SetupCredential::StoreInConfig(_),
                        reasoning_only_text: false,
                        make_default: true,
                        ..
                    },
                    allow_collisions: false,
                }
            ));
        }
    }

    #[test]
    fn chatgpt_confirmation_requests_the_existing_connection_handoff() {
        let mut app =
            setup_app(SetupMode::FirstRun, Vec::new(), Vec::new()).with_provider_actions(vec![
                SetupEntry {
                    id: "chatgpt".into(),
                    label: "Connect ChatGPT".into(),
                    detail: "OAuth".into(),
                    flow: SetupFlow::OAuth {
                        busy_note: "Opening ChatGPT sign-in…".into(),
                    },
                },
            ]);
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            SetupEffect::ConnectChatGpt
        ));
        assert!(app.is_busy());
        assert!(!app.is_choosing_action());
        assert!(app.busy_note().is_some_and(|note| note.contains("ChatGPT")));
        let mut methods = ResourcePicker::choices(
            "Connect ChatGPT",
            vec![ResourceEntry::new("browser", "Browser login", "")],
            "empty",
        )
        .with_back(true);
        assert_eq!(
            methods.on_event(ScreenEvent::Key(key(KeyCode::Esc))),
            ScreenStep::Outcome(crate::screen::FlowOutcome::Back)
        );
        app.back_from_chatgpt();
        assert!(app.is_choosing_action());
        assert_eq!(
            app.picker
                .as_ref()
                .and_then(ResourcePicker::selected_entry)
                .map(|entry| entry.id.as_str()),
            Some("chatgpt")
        );
    }

    #[test]
    fn an_arbitrary_entry_id_dispatches_its_required_flow() {
        let mut app =
            setup_app(SetupMode::FirstRun, Vec::new(), Vec::new()).with_provider_actions(vec![
                SetupEntry {
                    id: "future-provider".into(),
                    label: "Future provider".into(),
                    detail: String::new(),
                    flow: SetupFlow::AddModel,
                },
            ]);
        assert!(matches!(app.on_key(key(KeyCode::Enter)), SetupEffect::None));
        assert!(!app.is_choosing_action());
        assert_eq!(app.step, Step::ProviderChoice);
        assert!(matches!(
            app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            SetupEffect::Cancel
        ));
    }

    #[test]
    fn openrouter_mode_fixes_identity_and_endpoint_before_authentication() {
        let model = "openai/gpt-reviewed";
        let limits = SetupModelLimits {
            context_tokens: 128_000,
            max_input_tokens: 120_000,
            max_output_tokens: 8_000,
        };
        let mut app = setup_app(
            direct_provider_mode("openrouter"),
            Vec::new(),
            vec![ResourceEntry::new(
                model,
                "Reviewed model",
                "catalog limits",
            )],
        )
        .with_catalog_model_limits(BTreeMap::from([(model.to_owned(), limits)]));
        assert_eq!(app.step, Step::CredentialMethod);
        assert_eq!(app.provider, "openrouter");
        assert_eq!(app.endpoint, "https://openrouter.ai/api/v1");

        choose(&mut app, "environment");
        for character in "OPENROUTER_API_KEY".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::ModelChoice);
        assert!(app.secret.is_empty());

        choose(&mut app, model);
        assert_eq!(app.step, Step::DefaultChoice);
        assert_eq!(app.model, model);
        assert_eq!(app.context_tokens, Some(limits.context_tokens));
        assert_eq!(app.max_input_tokens, Some(limits.max_input_tokens));
        assert_eq!(app.max_output_tokens, Some(limits.max_output_tokens));
    }

    #[test]
    fn google_mode_chooses_a_catalog_model_without_collecting_an_endpoint() {
        let model = "gemini-3.6-flash";
        let limits = SetupModelLimits {
            context_tokens: 1_048_576,
            max_input_tokens: 1_048_576,
            max_output_tokens: 65_536,
        };
        let mut app = setup_app(
            direct_provider_mode("google"),
            Vec::new(),
            vec![ResourceEntry::new(
                model,
                "Gemini 3.6 Flash",
                "catalog limits",
            )],
        )
        .with_catalog_model_limits(BTreeMap::from([(model.to_owned(), limits)]));
        assert_eq!(app.step, Step::CredentialMethod);
        assert_eq!(app.provider, "google");
        assert!(app.endpoint.ends_with("/v1beta"));

        choose(&mut app, "environment");
        for character in "GEMINI_API_KEY".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::ModelChoice);
        choose(&mut app, model);
        choose(&mut app, "yes");
        assert_eq!(app.step, Step::Review);
        let review = app.review_lines().join("\n");
        assert!(review.contains("Gemini API"));
        assert!(review.contains(&app.endpoint), "{review}");
        assert!(review.contains("Models.dev frozen catalog"));
        assert!(!review.contains("API base URL"));
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            SetupEffect::Submit {
                submission: SetupSubmission::QuickGoogle { model: selected, credential: SetupCredential::Environment(variable) },
                allow_collisions: false,
            } if selected == model && variable == "GEMINI_API_KEY"
        ));
    }

    #[test]
    fn masked_key_never_appears_in_debug_or_review() {
        let secret = "sk-do-not-render";
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "glm");
        choose(&mut app, "keychain");
        for character in secret.chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        let rendered = format!("{app:?}\n{}", app.review_lines().join("\n"));
        assert!(!rendered.contains(secret), "{rendered}");
        assert!(
            app.secret
                .masked()
                .chars()
                .all(|character| character == '•')
        );
    }

    #[test]
    fn config_storage_is_masked_warned_and_submitted_as_a_secret() {
        let secret = "sk-config-input-must-not-render";
        let mut app = setup_app(
            SetupMode::Credential {
                provider: "zai".into(),
            },
            Vec::new(),
            Vec::new(),
        )
        .with_destination("/tmp/smith-home/.smith/config.toml");
        choose(&mut app, "config");
        for character in secret.chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        let input_render = render_setup(&app, 92, 20);
        assert!(!input_render.contains(secret), "{input_render}");
        assert!(input_render.contains('•'), "{input_render}");

        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::Review);
        let review = app.review_lines().join("\n");
        assert!(review.contains("api_key = [redacted]"), "{review}");
        assert!(review.contains("plaintext at rest"), "{review}");
        assert!(review.contains("same-user processes"), "{review}");
        assert!(review.contains("Backups"), "{review}");
        assert!(!review.contains(secret), "{review}");
        assert!(!format!("{app:?}").contains(secret));

        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            SetupEffect::Submit {
                submission: SetupSubmission::ChangeCredential {
                    provider,
                    credential: SetupCredential::StoreInConfig(value),
                },
                allow_collisions: false,
            } if provider == "zai" && value.expose() == secret
        ));
    }

    #[test]
    fn an_unknown_custom_model_asks_only_for_context_and_derives_ceilings() {
        let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
        for character in "router".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        for character in "https://example.test/v1".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        choose(&mut app, "existing-keychain");
        for character in "model".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        // The driver feeds a failed resolution back before any numeric value
        // is requested.
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            SetupEffect::ResolveModelLimits { .. }
        ));
        assert_eq!(app.step, Step::Busy);
        app.apply_resolved_limits(None);
        assert_eq!(app.step, Step::ContextTokens);
        let rendered = render_setup(&app, 92, 20);
        assert!(rendered.contains("Model context window"), "{rendered}");
        assert!(
            rendered.contains("input/output ceilings are derived"),
            "{rendered}"
        );
        assert!(!rendered.contains("Maximum input tokens"), "{rendered}");
        assert!(!rendered.contains("Maximum output tokens"), "{rendered}");
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::ContextTokens);
        assert!(app.error.is_some(), "an empty context window was accepted");
        for character in "64000".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::ResponseBehavior);
        assert_eq!(app.context_tokens, Some(64_000));
        assert_eq!(app.max_input_tokens, Some(64_000));
        assert_eq!(app.max_output_tokens, Some(16_000));
        assert!(
            app.review_lines()
                .iter()
                .any(|line| line.contains("context entered manually · ceilings derived"))
        );
    }

    #[test]
    fn a_model_step_resolution_request_carries_the_flow_facts() {
        let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
        for character in "router".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        for character in "https://example.test/v1".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        choose(&mut app, "keychain");
        for character in "sk-test".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        for character in "model".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        let SetupEffect::ResolveModelLimits { request } = app.on_key(key(KeyCode::Enter)) else {
            panic!("the custom model step requests resolution");
        };
        assert_eq!(request.endpoint.as_deref(), Some("https://example.test/v1"));
        assert_eq!(request.provider.as_deref(), Some("router"));
        assert_eq!(request.model, "model");
        assert!(request.bearer.is_some(), "the typed key rides along");
        assert_eq!(request.environment_variable, None);
        // The busy note is the reason the surface is blocked, and no key
        // escapes it while the driver works.
        assert!(
            app.busy_note()
                .is_some_and(|note| note.contains("Resolving"))
        );
        assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::None));
    }

    #[test]
    fn resolved_limits_skip_input_and_only_context_is_editable() {
        let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
        app.action = Some(SetupAction::AddProvider);
        app.endpoint = "https://example.test/v1".into();
        app.enter(Step::ModelName, false);
        for character in "model".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        app.apply_resolved_limits(Some(ResolvedModelLimits {
            context_tokens: 200_000,
            max_input_tokens: 200_000,
            max_output_tokens: 32_768,
            source: "endpoint /models listing".to_owned(),
        }));
        assert_eq!(app.step, Step::ResponseBehavior);
        assert!(
            app.review_lines()
                .iter()
                .any(|line| line.contains("(endpoint /models listing)")),
            "{:?}",
            app.review_lines()
        );
        // Back walks into the one context-window fallback with its value
        // prefilled, not into the busy step the resolution passed through.
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::ContextTokens);
        assert_eq!(app.input, "200000");
        let rendered = render_setup(&app, 92, 20);
        assert!(
            rendered.contains("Resolved from endpoint /models listing"),
            "{rendered}"
        );
        app.input.clear();
        for character in "100000".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::ResponseBehavior);
        assert_eq!(app.max_input_tokens, Some(100_000));
        assert_eq!(app.max_output_tokens, Some(25_000));
        assert!(
            app.review_lines()
                .iter()
                .any(|line| line.contains("(context entered manually · ceilings derived)")),
            "{:?}",
            app.review_lines()
        );
    }

    #[test]
    fn unchanged_resolved_context_preserves_published_ceilings() {
        let mut app = setup_app(
            SetupMode::AddModel { provider: None },
            Vec::new(),
            Vec::new(),
        );
        app.action = Some(SetupAction::AddModel);
        app.provider = "local".into();
        app.enter(Step::ModelName, false);
        for character in "m".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        app.apply_resolved_limits(Some(ResolvedModelLimits {
            context_tokens: 64_000,
            max_input_tokens: 60_000,
            max_output_tokens: 4_000,
            source: "endpoint /models listing".to_owned(),
        }));
        assert_eq!(app.step, Step::DefaultChoice);
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::ContextTokens);
        assert_eq!(app.input, "64000");
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::DefaultChoice);
        assert_eq!(app.max_input_tokens, Some(60_000));
        assert_eq!(app.max_output_tokens, Some(4_000));
        assert_eq!(
            app.limits_source.as_deref(),
            Some("endpoint /models listing")
        );
    }

    #[test]
    fn back_restores_custom_non_secret_fields_and_clears_secret_input() {
        let provider = "router";
        let endpoint = "https://example.test/v1";
        let model = "custom-model";
        let secret = "sk-never-restored";
        let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());

        for character in provider.chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        for character in endpoint.chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        choose(&mut app, "keychain");
        for character in secret.chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        for character in model.chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        assert!(matches!(
            app.on_key(key(KeyCode::Enter)),
            SetupEffect::ResolveModelLimits { .. }
        ));
        app.apply_resolved_limits(None);
        for character in "64000".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        choose(&mut app, "normal");
        choose(&mut app, "yes");
        assert_eq!(app.step, Step::Review);

        app.review_collisions("[providers.router]\n- endpoint = \"old\"\n+ endpoint = \"new\"");
        assert!(app.allow_collisions);

        // Back immediately revokes the collision approval, then the actual
        // setup history exposes the retained non-secret fields in order.
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::DefaultChoice);
        assert!(app.collision_preview.is_none());
        assert!(!app.allow_collisions);
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::ResponseBehavior);
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::ContextTokens);
        assert_eq!(app.input, "64000");
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::ModelName);
        assert_eq!(app.input, model);
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::CredentialValue);
        assert!(app.input.is_empty());
        assert!(app.secret.is_empty());
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::CredentialMethod);
        assert!(app.secret.is_empty());
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::Endpoint);
        assert_eq!(app.input, endpoint);
        app.on_key(key(KeyCode::BackTab));
        assert_eq!(app.step, Step::ProviderName);
        assert_eq!(app.input, provider);
    }

    #[test]
    fn escape_on_first_step_cancels_without_an_effectful_submission() {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::Cancel));
    }

    #[test]
    fn credential_service_failure_returns_to_authentication_with_environment_available() {
        let secret = "sk-must-be-forgotten";
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "glm");
        choose(&mut app, "keychain");
        for character in secret.chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.fail(
            "protected storage unavailable; choose the environment-variable option",
            true,
        );
        assert_eq!(app.step, Step::CredentialMethod);
        let picker = app.picker.as_ref().expect("authentication picker");
        assert!(picker.entries.iter().any(|entry| entry.id == "environment"));
        assert!(picker.entries.iter().any(|entry| entry.id == "config"));

        choose(&mut app, "environment");
        assert!(app.secret.is_empty(), "stale key material was retained");
        let rendered = format!("{app:?}\n{}", app.review_lines().join("\n"));
        assert!(!rendered.contains(secret), "{rendered}");
    }

    #[test]
    fn wide_no_color_review_names_every_non_secret_boundary() {
        let app = glm_environment_review();
        let rendered = render_setup(&app, 110, 34);
        for expected in [
            "Provider     zai",
            "api.z.ai/api/coding/paas/v4",
            "env:ZAI_API_KEY",
            "glm-5.2",
            "1M context",
            "trusted catalog v5",
            "/tmp/smith-home/.smith/config.toml",
            "Writes",
            "enter confirm",
            "esc back",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected:?}\n{rendered}"
            );
        }
    }

    #[test]
    fn narrow_validation_keeps_field_error_and_navigation_visible() {
        let mut app = setup_app(SetupMode::AddProvider, Vec::new(), Vec::new());
        app.on_key(key(KeyCode::Enter));
        let rendered = render_setup(&app, 40, 10);
        assert!(rendered.contains("Provider name"), "{rendered}");
        assert!(rendered.contains("error:"), "{rendered}");
        assert!(rendered.contains("enter continue"), "{rendered}");
        assert!(rendered.contains("esc cancel"), "{rendered}");
    }

    #[test]
    fn masked_input_and_collision_retry_remain_secret_free() {
        let secret = "sk-render-never";
        let mut masked = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut masked, "glm");
        choose(&mut masked, "keychain");
        for character in secret.chars() {
            masked.on_key(key(KeyCode::Char(character)));
        }
        let rendered = render_setup(&masked, 72, 18);
        assert!(!rendered.contains(secret), "{rendered}");
        assert!(rendered.contains('•'), "{rendered}");

        let mut review = glm_environment_review();
        review.review_collisions(
            "[providers.zai]\n- credential = \"env:OLD\"\n+ credential = \"env:ZAI_API_KEY\"",
        );
        assert!(matches!(
            review.on_key(key(KeyCode::Enter)),
            SetupEffect::Submit {
                allow_collisions: true,
                ..
            }
        ));
    }

    #[test]
    fn backing_out_of_collision_review_revokes_the_stale_approval() {
        let mut review = glm_environment_review();
        review.review_collisions(
            "[providers.zai]\n- credential = \"env:OLD\"\n+ credential = \"env:ZAI_API_KEY\"",
        );

        // Back-editing invalidates the approval: a re-entered review submits
        // without collision consent until the merge preview is shown again.
        review.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(review.input, "ZAI_API_KEY");
        review.on_key(key(KeyCode::Enter));
        assert_eq!(review.step, Step::Review);
        assert!(matches!(
            review.on_key(key(KeyCode::Enter)),
            SetupEffect::Submit {
                allow_collisions: false,
                ..
            }
        ));
    }

    #[test]
    fn picker_step_failures_render_their_error_above_the_picker() {
        let mut app = setup_app(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "glm");
        app.fail("keychain unavailable: locked", true);
        for (width, height) in [(44, 16), (80, 24)] {
            let rendered = render_setup(&app, width, height);
            assert!(
                rendered.contains("error: keychain unavailable: locked"),
                "{rendered}"
            );
            assert!(
                rendered.contains("❯ 1. Store API key securely"),
                "{rendered}"
            );
            for control in ["enter confirm", "esc back"] {
                assert!(rendered.contains(control), "{rendered}");
            }
        }
    }
}
