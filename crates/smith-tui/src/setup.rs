//! Pure guided-setup state and rendering.
//!
//! This reducer owns no filesystem, keychain, runtime, or terminal handle.
//! Secret input stays in a private masked buffer and crosses the effect
//! boundary only as Agent Runtime's redaction-safe [`Secret`] wrapper.

use std::fmt;

use agent_runtime_core::store::Secret;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::picker::{PickerOutcome, ResourceEntry, ResourcePicker, draw_resource_picker};
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
    /// A custom OpenAI-compatible provider and its first model.
    AddProvider {
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

/// Effect requested by one setup key.
#[derive(Debug, Clone)]
pub enum SetupEffect {
    /// Continue rendering.
    None,
    /// Exit successfully without writing or starting a session.
    Cancel,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    MaxInputTokens,
    MaxOutputTokens,
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
    picker: Option<ResourcePicker>,
    provider_actions: Vec<ResourceEntry>,
    provider_entries: Vec<ResourceEntry>,
    model_entries: Vec<ResourceEntry>,
    action: Option<SetupAction>,
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
    collision_preview: Option<String>,
    allow_collisions: bool,
    destination: String,
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
    ) -> Self {
        let mut app = Self {
            mode: mode.clone(),
            step: Step::Action,
            history: Vec::new(),
            picker: None,
            provider_actions: vec![
                ResourceEntry::new("glm", "Quick start with GLM", "Z.AI · GLM-4.7"),
                ResourceEntry::new(
                    "add-provider",
                    "Add provider",
                    "custom OpenAI-compatible endpoint",
                ),
            ],
            provider_entries,
            model_entries,
            action: None,
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
            collision_preview: None,
            allow_collisions: false,
            destination: "~/.smith/config.toml".into(),
        };
        match mode {
            SetupMode::FirstRun | SetupMode::Menu => app.enter(Step::Action, false),
            SetupMode::AddProvider => {
                app.action = Some(SetupAction::AddProvider);
                app.enter(Step::ProviderName, false);
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

    /// Replaces provider actions with descriptors supported by this runtime
    /// build. IDs remain the reducer's stable `glm`/`add-provider` vocabulary.
    #[must_use]
    pub fn with_provider_actions(mut self, actions: Vec<ResourceEntry>) -> Self {
        self.provider_actions = actions;
        self.configure_picker();
        self
    }

    /// Whether setup is waiting for an external persistence/preflight effect.
    pub fn is_busy(&self) -> bool {
        self.step == Step::Busy
    }

    /// Returns setup to an actionable step with a bounded external error.
    pub fn fail(&mut self, message: impl Into<String>, authentication: bool) {
        self.error = Some(bound(message.into(), 1_024));
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
        self.allow_collisions = true;
        self.error = Some(
            "Existing values differ. Review the additional lines, then press Enter again to replace only those values."
                .into(),
        );
        self.step = Step::Review;
        self.configure_picker();
    }

    /// Non-secret review lines.
    pub fn review_lines(&self) -> Vec<String> {
        let mut lines = match self.action {
            Some(SetupAction::QuickGlm) => vec![
                "action: Quick start with GLM".into(),
                "provider: zai (openai-compatible)".into(),
                "endpoint: https://api.z.ai/api/coding/paas/v4".into(),
                format!("credential: {}", self.credential_reference("zai")),
                "model: glm-5.2".into(),
                "limits: context 1000000 · max input 1000000 · max output 131072 (trusted catalog v2)"
                    .into(),
                "request/output reserve: 32768".into(),
                "response: reasoning-only success becomes visible text; thinking stays enabled"
                    .into(),
                "default profile: glm".into(),
            ],
            Some(SetupAction::AddProvider) => vec![
                "action: Add OpenAI-compatible provider".into(),
                format!("provider: {}", self.provider),
                format!("endpoint: {}", self.endpoint),
                format!("credential: {}", self.credential_reference(&self.provider)),
                format!("model: {}/{}", self.provider, self.model),
                self.limits_review(),
                format!(
                    "response: {}",
                    if self.reasoning_only_text {
                        "reasoning-only success becomes visible text"
                    } else {
                        "preserve provider classifications"
                    }
                ),
                format!("make default: {}", yes_no(self.make_default)),
            ],
            Some(SetupAction::AddModel) => vec![
                "action: Add model".into(),
                format!("provider: {}", self.provider),
                format!("model: {}/{}", self.provider, self.model),
                self.limits_review(),
                format!("make default: {}", yes_no(self.make_default)),
            ],
            Some(SetupAction::ChangeDefault) => vec![
                "action: Change default model".into(),
                format!("provider/model: {}/{}", self.provider, self.model),
            ],
            Some(SetupAction::ChangeCredential) => vec![
                "action: Change provider credential".into(),
                format!("provider: {}", self.provider),
                format!("credential: {}", self.credential_reference(&self.provider)),
            ],
            None => vec!["Choose a setup action.".into()],
        };
        if self.credential_method == Some(CredentialMethod::Config) {
            lines.push("warning: plaintext at rest; same-user processes can read this key".into());
            lines.push("warning: backups may retain this key after rotation".into());
        }
        if let Some(preview) = &self.collision_preview {
            lines.push("configuration merge preview:".into());
            lines.extend(preview.lines().map(|line| format!("  {line}")));
        }
        lines.push(format!("destination: {}", self.destination));
        lines.push("pending action: write user config, then run local preflight".into());
        lines
    }

    /// Reduces one setup key.
    pub fn on_key(&mut self, key: KeyEvent) -> SetupEffect {
        if key.kind == KeyEventKind::Release || self.step == Step::Busy {
            return SetupEffect::None;
        }
        if matches!(
            (key.code, key.modifiers),
            (KeyCode::Esc, _) | (KeyCode::Char('c'), KeyModifiers::CONTROL)
        ) {
            return SetupEffect::Cancel;
        }
        if key.code == KeyCode::BackTab {
            self.back();
            return SetupEffect::None;
        }
        self.error = None;

        if let Some(picker) = &mut self.picker {
            return match picker.on_key(key) {
                PickerOutcome::Pending => SetupEffect::None,
                PickerOutcome::Cancelled => SetupEffect::Cancel,
                PickerOutcome::Selected(id) => {
                    self.select_picker(id);
                    SetupEffect::None
                }
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
            self.input.clear();
            self.error = None;
            // Leaving the review invalidates a collision approval: anything
            // edited on the way back must be re-reviewed before it can
            // replace existing values.
            self.collision_preview = None;
            self.allow_collisions = false;
            self.configure_picker();
        }
    }

    fn configure_picker(&mut self) {
        self.picker = match self.step {
            Step::Action => {
                let mut entries = self.provider_actions.clone();
                if matches!(self.mode, SetupMode::Menu) {
                    entries.push(ResourceEntry::new(
                        "add-model",
                        "Add model",
                        "attach explicit limits to an existing provider",
                    ));
                    entries.push(ResourceEntry::new(
                        "change-default",
                        "Change default",
                        "choose a configured provider/model pair",
                    ));
                }
                Some(ResourcePicker::new(
                    "Smith setup",
                    entries,
                    "No setup actions are available.",
                ))
            }
            Step::ProviderChoice => Some(ResourcePicker::new(
                "Choose provider",
                self.provider_entries.clone(),
                "No configured provider · run smith setup add-provider",
            )),
            Step::CredentialMethod => Some(ResourcePicker::new(
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
                        "keychain:smith/<provider>",
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
                "No selectable model · run smith setup add-model",
            )),
            Step::ResponseBehavior => Some(ResourcePicker::new(
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
            Step::DefaultChoice => Some(ResourcePicker::new(
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
    }

    fn select_picker(&mut self, id: String) {
        match self.step {
            Step::Action => match id.as_str() {
                "glm" => {
                    self.action = Some(SetupAction::QuickGlm);
                    self.provider = "zai".into();
                    self.endpoint = "https://api.z.ai/api/coding/paas/v4".into();
                    self.model = "glm-5.2".into();
                    self.context_tokens = Some(1_000_000);
                    self.max_input_tokens = Some(1_000_000);
                    self.max_output_tokens = Some(131_072);
                    self.reasoning_only_text = true;
                    self.make_default = true;
                    self.enter(Step::CredentialMethod, true);
                }
                "add-provider" => {
                    self.action = Some(SetupAction::AddProvider);
                    self.enter(Step::ProviderName, true);
                }
                "add-model" => {
                    self.action = Some(SetupAction::AddModel);
                    self.enter(Step::ProviderChoice, true);
                }
                "change-default" => {
                    self.action = Some(SetupAction::ChangeDefault);
                    self.enter(Step::ModelChoice, true);
                }
                _ => {}
            },
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
                if let Some((provider, model)) = id.split_once('/') {
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
    }

    fn after_credential_step(&self) -> Step {
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
                    self.error = Some(
                        "Use an environment variable such as ZAI_API_KEY (letters, digits, underscore)."
                            .into(),
                    );
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
                    self.enter(Step::ContextTokens, true);
                }
            }
            Step::ContextTokens => match positive_u32(&value) {
                Ok(value) => {
                    self.context_tokens = Some(value);
                    self.enter(Step::MaxInputTokens, true);
                }
                Err(error) => self.error = Some(error),
            },
            Step::MaxInputTokens => match positive_u32(&value) {
                Ok(value) if Some(value) <= self.context_tokens => {
                    self.max_input_tokens = Some(value);
                    self.enter(Step::MaxOutputTokens, true);
                }
                Ok(_) => self.error = Some("Maximum input cannot exceed context tokens.".into()),
                Err(error) => self.error = Some(error),
            },
            Step::MaxOutputTokens => match positive_u32(&value) {
                Ok(value) if Some(value) <= self.context_tokens => {
                    self.max_output_tokens = Some(value);
                    if self.action == Some(SetupAction::AddProvider) {
                        self.enter(Step::ResponseBehavior, true);
                    } else {
                        self.enter(Step::DefaultChoice, true);
                    }
                }
                Ok(_) => self.error = Some("Maximum output cannot exceed context tokens.".into()),
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
            SetupAction::AddProvider => Some(SetupSubmission::AddProvider {
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
            "limits: context {} · max input {} · max output {} (explicit)",
            self.context_tokens.unwrap_or_default(),
            self.max_input_tokens.unwrap_or_default(),
            self.max_output_tokens.unwrap_or_default()
        )
    }

    fn prompt(&self) -> (&'static str, &'static str, bool) {
        match self.step {
            Step::ProviderName => (
                "Provider name",
                "A stable local name, for example openrouter",
                false,
            ),
            Step::Endpoint => (
                "API base URL",
                "OpenAI-compatible base, for example https://openrouter.ai/api/v1",
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
                    },
                    true,
                )
            }
            Step::CredentialValue => (
                "Environment variable",
                "Smith records the name only and does not read its value during setup",
                false,
            ),
            Step::ModelName => (
                "Model ID",
                "Use the provider's exact identifier; Smith will not guess limits",
                false,
            ),
            Step::ContextTokens => ("Context tokens", "Total context window", false),
            Step::MaxInputTokens => ("Maximum input tokens", "Enforced input ceiling", false),
            Step::MaxOutputTokens => ("Maximum output tokens", "Provider output ceiling", false),
            _ => ("", "", false),
        }
    }
}

/// Draws the complete setup surface.
pub fn draw_setup(frame: &mut Frame<'_>, app: &SetupApp, theme: Theme) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let outer = centered(area, 88, 30);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Smith setup · no agent session or provider request exists yet ")
        .border_style(theme.style(Tone::Accent));
    let inner = block.inner(outer);
    frame.render_widget(block, outer);

    if let Some(picker) = &app.picker {
        // A failure that returns to a picker step still explains itself: the
        // error renders above the picker instead of being silently dropped.
        let picker_area = if let Some(error) = &app.error {
            let [message, rest] =
                Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(inner);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("error: {error}"),
                    theme.style(Tone::Danger),
                )))
                .wrap(Wrap { trim: false }),
                message,
            );
            rest
        } else {
            inner
        };
        draw_resource_picker(frame, picker_area, picker, theme);
        return;
    }

    let [body, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(inner);
    let mut lines = Vec::new();
    match app.step {
        Step::Review | Step::Busy => {
            lines.push(Line::from(Span::styled(
                if app.step == Step::Busy {
                    "Applying reviewed setup and running local preflight…"
                } else {
                    "Review the complete non-secret setup change:"
                },
                theme.style(Tone::Accent),
            )));
            lines.push(Line::default());
            for line in app.review_lines() {
                lines.push(Line::from(format!("  {line}")));
            }
        }
        _ => {
            let (label, help, masked) = app.prompt();
            lines.push(Line::from(Span::styled(label, theme.style(Tone::Accent))));
            if app.error.is_none() {
                lines.push(Line::from(Span::styled(help, theme.style(Tone::Dim))));
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
        lines.push(Line::from(Span::styled(
            format!("error: {error}"),
            theme.style(Tone::Danger),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    let footer_text = if inner.width < 60 {
        if app.step == Step::Review {
            " Enter confirm · Back: Shift+Tab\n Esc Cancel"
        } else if app.step == Step::Busy {
            " Validating locally\n Esc is disabled while applying"
        } else {
            " Enter continue · Back: Shift+Tab\n Esc Cancel"
        }
    } else if app.step == Step::Review {
        " Enter confirm · Shift+Tab Back · Esc Cancel"
    } else if app.step == Step::Busy {
        " Validating without a paid inference request"
    } else {
        " Enter continue · Shift+Tab Back · Esc Cancel"
    };
    frame.render_widget(
        Paragraph::new(footer_text).style(theme.style(Tone::Dim)),
        footer,
    );
}

fn centered(area: Rect, preferred_width: u16, preferred_height: u16) -> Rect {
    let width = preferred_width.min(area.width.saturating_sub(2)).max(1);
    let height = preferred_height.min(area.height.saturating_sub(2)).max(1);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
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

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn bound(mut value: String, limit: usize) -> String {
    if value.len() > limit {
        value.truncate(limit);
        value.push('…');
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn choose(app: &mut SetupApp, id: &str) {
        app.select_picker(id.to_owned());
    }

    fn render_setup(app: &SetupApp, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|frame| {
                draw_setup(
                    frame,
                    app,
                    Theme::from_env().without_color().without_motion(),
                );
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn glm_environment_review() -> SetupApp {
        let mut app = SetupApp::new(SetupMode::FirstRun, Vec::new(), Vec::new())
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
    fn glm_funnel_reaches_a_non_secret_review_and_submission() {
        let mut app = glm_environment_review();
        assert_eq!(app.step, Step::Review);
        let review = app.review_lines().join("\n");
        assert!(review.contains("glm-5.2"));
        assert!(review.contains("env:ZAI_API_KEY"));
        assert!(review.contains("reasoning-only"));
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
    fn masked_key_never_appears_in_debug_or_review() {
        let secret = "sk-do-not-render";
        let mut app = SetupApp::new(SetupMode::FirstRun, Vec::new(), Vec::new());
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
        let mut app = SetupApp::new(
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
        assert!(review.contains("backups"), "{review}");
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
    fn custom_limits_are_required_and_cross_field_validated() {
        let mut app = SetupApp::new(SetupMode::AddProvider, Vec::new(), Vec::new());
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
        app.on_key(key(KeyCode::Enter));
        for character in "100".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        for character in "101".chars() {
            app.on_key(key(KeyCode::Char(character)));
        }
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.step, Step::MaxInputTokens);
        assert!(
            app.error
                .as_deref()
                .is_some_and(|error| error.contains("exceed"))
        );
    }

    #[test]
    fn escape_cancels_without_an_effectful_submission() {
        let mut app = SetupApp::new(SetupMode::FirstRun, Vec::new(), Vec::new());
        assert!(matches!(app.on_key(key(KeyCode::Esc)), SetupEffect::Cancel));
    }

    #[test]
    fn credential_service_failure_returns_to_authentication_with_environment_available() {
        let secret = "sk-must-be-forgotten";
        let mut app = SetupApp::new(SetupMode::FirstRun, Vec::new(), Vec::new());
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
            "provider: zai",
            "api.z.ai/api/coding/paas/v4",
            "env:ZAI_API_KEY",
            "glm-5.2",
            "context 1000000",
            "trusted catalog v2",
            "/tmp/smith-home/.smith/config.toml",
            "pending action:",
            "Shift+Tab Back",
            "Esc Cancel",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected:?}\n{rendered}"
            );
        }
    }

    #[test]
    fn narrow_validation_keeps_field_error_and_navigation_visible() {
        let mut app = SetupApp::new(SetupMode::AddProvider, Vec::new(), Vec::new());
        app.on_key(key(KeyCode::Enter));
        let rendered = render_setup(&app, 40, 10);
        assert!(rendered.contains("Provider name"), "{rendered}");
        assert!(rendered.contains("error:"), "{rendered}");
        assert!(rendered.contains("Back: Shift+Tab"), "{rendered}");
        assert!(rendered.contains("Esc Cancel"), "{rendered}");
    }

    #[test]
    fn masked_input_and_collision_retry_remain_secret_free() {
        let secret = "sk-render-never";
        let mut masked = SetupApp::new(SetupMode::FirstRun, Vec::new(), Vec::new());
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
        for character in "ZAI_API_KEY".chars() {
            review.on_key(key(KeyCode::Char(character)));
        }
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
        let mut app = SetupApp::new(SetupMode::FirstRun, Vec::new(), Vec::new());
        choose(&mut app, "glm");
        app.fail("keychain unavailable: locked", true);
        let rendered = render_setup(&app, 72, 18);
        assert!(
            rendered.contains("error: keychain unavailable: locked"),
            "{rendered}"
        );
    }
}
