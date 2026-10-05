use super::*;

/// Why a resolved configuration could not become a runtime.
///
/// Every variant names what the user has to change, and none can carry a
/// credential: the payloads are provider names, references, and classified
/// failures from types that redact themselves.
#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    /// Declarative modules, trust, contributions, or grants did not resolve.
    #[error(transparent)]
    Harness(#[from] crate::harness::HarnessResolutionError),

    /// Two tools attempted to register the same stable ability name.
    #[error("Smith could not seal its ability catalog: {0}")]
    AbilityRegistry(#[source] agent_runtime::registry::NameConflict),

    /// The configured adapter kind is not one the pinned runtime ships.
    #[error(
        "provider `{provider}` selects the `{kind}` adapter, which this build of Agent Runtime \
         does not ship; the available kinds are `{KIND_OPENAI_COMPATIBLE}`, \
         `{KIND_OPENAI_RESPONSES}`, `{KIND_ANTHROPIC_MESSAGES}`, \
         `{KIND_CHATGPT_RESPONSES}`, `{KIND_XAI_RESPONSES}`, `{KIND_GEMINI_INTERACTIONS}`, and \
         `{KIND_COMMAND_JSONL}`, and `{KIND_FAKE}`"
    )]
    AdapterUnavailable {
        /// The provider that selected it.
        provider: String,
        /// The adapter kind it selected.
        kind: String,
    },
    /// Static command process settings did not resolve to an executable target.
    #[error("the command provider process declaration is unusable: {0}")]
    CommandConfiguration(#[source] CommandConfigError),
    /// The selected model cannot participate in the revision-1 command protocol.
    #[error("the command provider adapter is unusable: {0}")]
    CommandAdapter(#[source] CommandAdapterConfigError),
    /// The command adapter's fixed local model declaration was contradictory.
    #[error("the command provider model declaration is unusable: {0}")]
    CommandProviderBuild(#[source] CommandProviderBuildError),
    /// One named command environment credential could not cross the secret boundary.
    #[error("command environment variable `{variable}` could not be resolved: {source}")]
    CommandEnvironment {
        /// Environment variable named by configuration; never its value.
        variable: String,
        /// Existing redaction-safe credential failure.
        #[source]
        source: Box<FactoryError>,
    },
    /// The explicit compatibility probe failed before runtime construction.
    #[error("the command provider compatibility probe failed: {0}")]
    CommandPreflight(#[source] CommandPreflightError),
    /// The executable answered the probe but did not accept this exact contract.
    #[error("the command provider executable is incompatible with protocol revision 1")]
    CommandIncompatible,
    /// The configured endpoint cannot be used as written.
    #[error("provider `{provider}` has an unusable `base_url`: {message}")]
    Endpoint {
        /// The provider whose endpoint is unusable.
        provider: String,
        /// What is wrong with it. Never the URL itself, which may carry a key.
        message: String,
    },
    /// A configured credential is not a usable reference.
    #[error("provider `{provider}` has an unusable `credential`: {source}")]
    CredentialReference {
        /// The provider whose credential is unusable.
        provider: String,
        /// Why the reference could not be parsed.
        source: CredentialRefError,
    },
    /// A credential reference did not resolve to a secret.
    #[error(transparent)]
    Credential(#[from] CredentialError),
    /// The credential lookup did not finish.
    #[error("the provider credential lookup did not complete")]
    CredentialTask,
    /// The platform credential service did not answer within the startup
    /// boundary.
    #[error(
        "the provider credential lookup did not complete within {timeout_ms} ms; \
         unlock or allow the platform credential service, or use an `env:<VAR>` reference"
    )]
    CredentialTimeout {
        /// Configured lookup boundary.
        timeout_ms: u64,
    },
    /// Smith's protected ChatGPT token bundle or OAuth client is unusable.
    #[error("the experimental ChatGPT connection is unusable: {0}")]
    ChatGptAuth(#[source] crate::chatgpt::ChatGptAuthError),
    /// The stored xAI login could not be read or renewed.
    #[error("the configured xAI session is unusable")]
    XaiAuth(#[source] crate::xai::XaiAuthError),
    /// No layer supplied enforceable limits for the selected model.
    #[error(
        "provider `{provider}` cannot plan against model `{model}`: {source}. Declare \
         `[models.\"{provider}/{model}\"]` with `context_tokens`, `max_input_tokens`, and \
         `max_output_tokens`, or register a catalog source that does"
    )]
    ModelProfile {
        /// The provider serving the model.
        provider: String,
        /// The model that could not be resolved.
        model: ModelId,
        /// The shared resolver's structured failure.
        source: ModelProfileError,
    },
    /// A requested named context window is unavailable or pinned by a flat limit.
    #[error("provider `{provider}` cannot select a context window for model `{model}`: {message}")]
    ContextWindow {
        /// Provider serving the model.
        provider: String,
        /// Selected model.
        model: ModelId,
        /// Safe explanation and valid alternatives.
        message: String,
    },
    /// A reasoning request cannot be represented by the exact binding.
    #[error("provider `{provider}` cannot apply reasoning controls to model `{model}`: {message}")]
    Reasoning {
        /// Serving provider identity.
        provider: String,
        /// Selected model.
        model: ModelId,
        /// Redaction-safe validation detail and alternatives.
        message: String,
    },
    /// The configured context reserves leave no room to plan in.
    #[error("the configured context reserves cannot be planned against: {message}")]
    ContextReserve {
        /// Which reserves conflict with which limits.
        message: String,
    },
    /// The selected installed coding agent is not installed on this machine.
    #[error(
        "model `{model}` runs turns on the installed agent `{kind}`, but `{program}` is not on \
         PATH; install it, or declare `[harness.{kind}]` with an absolute `executable`"
    )]
    AgentNotInstalled {
        /// The `cli/<kind>/<model>` id that selected it.
        model: String,
        /// The installed agent kind, which is also its `[harness.<kind>]` key.
        kind: String,
        /// The program that was looked for.
        program: String,
    },
    /// A host adapter the composition requires was not supplied.
    #[error("this run needs a {what}: {message}")]
    MissingHostPolicy {
        /// The adapter that is missing.
        what: &'static str,
        /// Why the run cannot proceed without it.
        message: String,
    },
    /// The production transport could not be built.
    #[error("the provider transport could not be built: {0}")]
    Transport(ProviderError),
    /// The shared runtime refused the composition.
    #[error("the shared runtime refused this composition: {0}")]
    Runtime(RuntimeError),
}
