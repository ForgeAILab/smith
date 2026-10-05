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
    PickerContext, PickerOutcome, ResourceEntry, ResourcePicker, ScreenFooter, draw_inline_screen,
    draw_picker_context, indented_words, picker_content_height,
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

struct MaskedInput(crate::line_input::LineInput);

impl Default for MaskedInput {
    fn default() -> Self {
        Self(crate::line_input::LineInput::masked())
    }
}

impl MaskedInput {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn secret(&self) -> Secret {
        Secret::new(self.0.text().to_owned())
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
    /// Re-entered steps need whole text even when their picker title is unchanged.
    step_generation: u64,
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
    input: crate::line_input::LineInput,
    error: Option<String>,
    /// Why the surface is busy, shown instead of the default applying note.
    busy_note: Option<String>,
    /// Provenance of automatically resolved limits, shown in review.
    limits_source: Option<String>,
    collision_preview: Option<String>,
    review_scroll: Cell<ReviewScroll>,
    allow_collisions: bool,
    destination: String,
    title: Option<String>,
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

mod flow;
mod lifecycle;
mod rendering;
mod review;

pub use rendering::draw_setup;

#[cfg(test)]
mod tests;
