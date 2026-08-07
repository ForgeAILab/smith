# configuration Specification

## Purpose
TBD - created by archiving change add-smith-agent-harness. Update Purpose after archive.
## Requirements
### Requirement: Layered explainable configuration

Smith SHALL resolve configuration in this low-to-high order: built-in defaults,
`~/.smith/config.toml`, project `.smith/config.toml`, project
`.smith/config.local.toml`, selected profile, `SMITH_*` environment variables,
CLI flags, and explicit per-session overrides. Every resolved value MUST retain
source provenance.

#### Scenario: CLI overrides project profile

- **GIVEN** a project profile selects one model
- **AND** the CLI explicitly selects another model
- **WHEN** Smith resolves the session configuration
- **THEN** the CLI model wins
- **AND** `smith config explain model` identifies the CLI as its source

#### Scenario: Unknown setting is present

- **GIVEN** a project config contains an unknown key
- **WHEN** Smith validates configuration
- **THEN** it reports the file, key, and nearest known alternatives
- **AND** does not silently discard the key

### Requirement: Typed resolved run configuration

Smith SHALL resolve file, profile, environment, CLI, and session inputs into one
typed immutable run configuration before constructing Agent Runtime. Raw TOML,
environment strings, and CLI parser values MUST NOT be passed directly to
runtime or provider builders.

#### Scenario: Profile resolves completely

- **GIVEN** the selected profile references a known provider, credential,
  model, limits, and context policy
- **WHEN** Smith resolves the run configuration
- **THEN** every runtime-facing field has a validated typed value
- **AND** every field retains the source that supplied it

#### Scenario: Profile references an unknown provider

- **GIVEN** a selected profile names a provider that is not registered
- **WHEN** Smith resolves the run configuration
- **THEN** resolution fails with the profile key and source file
- **AND** no runtime or terminal session starts

### Requirement: Enforceable model profile

Every selected provider/model pair MUST resolve Agent Runtime's immutable model
profile through explicit Smith configuration or a layered model catalog. Smith
MUST NOT guess context, input, or output limits for an unknown model.

#### Scenario: Explicit model limits override catalog metadata

- **GIVEN** a validated cached catalog contains model limits
- **AND** an explicit CLI or session layer supplies different safe limits
- **WHEN** Smith resolves the model profile
- **THEN** the explicit values win according to runtime catalog precedence
- **AND** `smith config explain` retains the provenance of both sources

#### Scenario: No source supplies safe limits

- **GIVEN** the selected model is absent from every configured catalog source
- **WHEN** Smith prepares the runtime
- **THEN** it returns a missing-model-profile diagnostic before network I/O
- **AND** it does not substitute a default context window

### Requirement: Provider configuration maps to shared adapters

Provider configuration SHALL identify a shared adapter kind, endpoint, model
selection, one credential source, and only options supported by that adapter.
A credential source is either a validated reference or an inline `api_key`
from owner-only user configuration. Smith SHALL validate adapter-specific keys
before constructing the shared provider.

#### Scenario: Configure an OpenAI-compatible provider

- **GIVEN** a profile selects an OpenAI-compatible provider with a base URL,
  model, one credential source, and enforceable limits
- **WHEN** Smith builds the configured runtime
- **THEN** it constructs the shared OpenAI-compatible adapter over Smith's
  production HTTP transport
- **AND** the secret value is exposed only at that construction boundary

#### Scenario: Provider declares two credential sources

- **GIVEN** one provider declares both `credential` and `api_key`
- **WHEN** Smith resolves configuration
- **THEN** resolution fails before any credential-service or provider I/O
- **AND** diagnostics name the conflicting setting names without either value

#### Scenario: Configure an unavailable adapter

- **GIVEN** a profile selects an adapter not present in the pinned Agent Runtime
- **WHEN** Smith validates configuration
- **THEN** it reports the adapter as unavailable
- **AND** it does not silently route the request through another endpoint family

### Requirement: Project-local Smith directory

Smith SHALL discover repository customization under `.smith/`. Repository-safe
declarative config, instructions, profiles, extension manifests, and extension
source MAY live there, while sessions, trust decisions, monitor output, and
credential material MUST remain in user state under `~/.smith/`.

#### Scenario: Open a configured project

- **GIVEN** the project root contains `.smith/config.toml` and
  `.smith/extensions/review.ts`
- **WHEN** Smith opens the project
- **THEN** it may read and validate the declarative config
- **AND** it keeps session and secret state outside the repository

### Requirement: Hash-bound project execution trust

Smith MUST obtain user confirmation before executing project extensions, hooks,
shell-valued settings, or credential helpers. Trust SHALL bind the canonical
project path to the exact executable-content hash, and a content change MUST
invalidate the prior decision.

#### Scenario: First project extension load

- **GIVEN** a project extension has no matching trust record
- **WHEN** Smith is about to start it
- **THEN** Smith displays its path, declared capabilities, and content identity
- **AND** does not execute it until the user confirms

#### Scenario: Trusted extension changes

- **GIVEN** the user trusted a project extension
- **WHEN** its executable content or manifest changes
- **THEN** the old trust record no longer authorizes execution
- **AND** Smith requests confirmation for the new hash

### Requirement: Repository configuration cannot self-authorize tools

Smith MUST NOT treat repository-controlled `approval.mode = "allow-all"` or
`approval.auto_approve` as execution authority merely because the project was
opened. Authority-bearing approval settings MUST come from user-controlled
configuration or an explicit higher-precedence invocation policy.

#### Scenario: Malicious project requests silent write authority

- **GIVEN** project or project-local configuration selects `allow-all` or
  auto-approves a mutating tool
- **WHEN** Smith preflights the session
- **THEN** startup fails before creating session state or entering the terminal
- **AND** the diagnostic says to move the policy to user configuration or pass
  an explicit CLI policy

### Requirement: Repository configuration cannot redirect user state

Repository-controlled configuration MUST NOT redirect, enable/disable, or
disable journaling for user-scoped session persistence. Persistence policy
MUST come from built-in defaults, user-controlled configuration, or an
explicit higher-precedence invocation policy.

#### Scenario: Project selects an external session directory

- **GIVEN** project configuration sets `persistence.sessions_dir`
- **WHEN** Smith preflights the session
- **THEN** startup fails before creating the selected directory
- **AND** the diagnostic identifies persistence as user-scoped policy

### Requirement: Encrypted user-scope credentials

Smith MUST default provider credential enrollment to the macOS Keychain or
Linux Secret Service when available. An encrypted-file fallback MAY be enabled
explicitly with its passphrase or master key supplied from outside the
ciphertext file. A user MAY explicitly store an API key as plaintext in
owner-only `~/.smith/config.toml` to avoid credential-service prompts. Project
configuration SHALL contain references only.

#### Scenario: Save a provider API key

- **GIVEN** a supported OS credential service is available
- **WHEN** the user accepts the protected-storage choice
- **THEN** Smith stores the secret in that service
- **AND** writes only its reference into Smith configuration

#### Scenario: User explicitly chooses no-prompt config storage

- **GIVEN** setup has warned that the user config is plaintext and may be read
  by same-user processes or backups
- **WHEN** the user selects config storage and enters a key
- **THEN** Smith writes it only as the provider's `api_key` in mode-`0600`
  `~/.smith/config.toml`
- **AND** subsequent startup resolves it without opening a password or
  credential-service prompt

#### Scenario: User config containing a key is not private

- **GIVEN** `~/.smith/config.toml` contains `api_key`
- **WHEN** it is a symlink, is not a regular file, belongs to another user, or
  has group or world permissions
- **THEN** startup fails before provider or terminal I/O
- **AND** no diagnostic includes the key

#### Scenario: Project supplies an inline key

- **GIVEN** project or project-local configuration contains `api_key`
- **WHEN** Smith loads the configuration layers
- **THEN** it refuses the setting before terminal, provider, or
  credential-service I/O
- **AND** does not copy it into user configuration

### Requirement: Secret redaction

Smith SHALL redact known secrets from logs, events, command previews,
extension payloads, diagnostic metadata, and persisted provider error details.

#### Scenario: Provider error echoes a key

- **GIVEN** a provider error body contains a configured credential value
- **WHEN** Smith records and displays the failure
- **THEN** the stored and visible forms replace the credential with a redaction
  marker

### Requirement: Setup readiness classification

Smith SHALL distinguish a genuinely unconfigured installation from ready and
invalid configuration before extracting every required run field. Ready
configuration MUST produce the same typed resolution and provenance as the
ordinary resolver. Malformed files, unknown keys, partial provider/model
intent, unusable references, and invalid limits MUST remain configuration
errors and MUST NOT be reclassified as first-run setup.

#### Scenario: No layer contains setup intent

- **GIVEN** no user or project config supplies a provider or model
- **AND** no environment, CLI, or session override supplies setup intent
- **WHEN** Smith inspects startup readiness
- **THEN** it returns an unconfigured state with the discovered user and
  project locations
- **AND** it does not invent a provider, model, endpoint, credential, or limit

#### Scenario: Partial provider declaration exists

- **GIVEN** a user file selects a provider but omits its required declaration
  or model
- **WHEN** Smith inspects startup readiness
- **THEN** it returns the existing actionable configuration error
- **AND** automatic setup MUST NOT overwrite or reinterpret the file

#### Scenario: Configuration is ready

- **GIVEN** the effective layers fully configure a usable run
- **WHEN** Smith inspects startup readiness
- **THEN** it returns the same typed configuration and provenance as full
  resolution
- **AND** setup is not entered automatically

### Requirement: Derived setup readiness

Smith MUST derive setup readiness from the current effective configuration on
every launch and MUST NOT rely on an independent onboarding-complete flag.

#### Scenario: Previously configured user file is removed

- **GIVEN** a prior launch completed setup successfully
- **AND** the only user configuration supplying provider/model setup is later
  removed
- **WHEN** Smith starts interactively again
- **THEN** readiness is unconfigured
- **AND** no stale completion marker suppresses setup

### Requirement: Trusted setup descriptors and model limits

Provider choices exposed by setup SHALL map to Smith-owned descriptors whose
adapter kind is present in the pinned Agent Runtime. A selected model MUST
resolve all enforceable limits from explicit input or a versioned trusted
catalog; setup MUST NOT guess limits or route an unavailable adapter through a
different provider family.

#### Scenario: Catalog-backed model is selected

- **GIVEN** a setup descriptor offers a model with complete trusted catalog
  metadata
- **WHEN** the user selects it
- **THEN** review shows the model and catalog provenance of its context,
  maximum-input, and maximum-output limits
- **AND** full runtime preflight resolves the same immutable profile

#### Scenario: GLM quick start is selected

- **GIVEN** the pinned runtime includes the OpenAI-compatible adapter
- **WHEN** the user selects the GLM quick start
- **THEN** setup proposes provider `zai` at
  `https://api.z.ai/api/coding/paas/v4` and model `glm-4.7`
- **AND** the proposed model profile declares 200000 context tokens, 196000
  maximum input tokens, and 131072 maximum output tokens
- **AND** the selected profile requests and reserves at most 8192 output tokens
- **AND** the proposed provider response policy treats a non-redacted
  reasoning-only completion as visible assistant text without disabling GLM
  thinking

#### Scenario: Unknown custom model is selected

- **GIVEN** the custom model is absent from every trusted catalog source
- **WHEN** the user tries to continue from model setup
- **THEN** Smith requires explicit context, maximum-input, and maximum-output
  limits
- **AND** it cannot finish setup while any enforceable limit is absent or
  invalid

#### Scenario: Descriptor names an unavailable adapter

- **GIVEN** a setup descriptor's adapter is not present in the pinned runtime
- **WHEN** Smith builds the provider choices
- **THEN** that descriptor is not selectable
- **AND** Smith does not silently substitute an OpenAI-compatible or other
  adapter

### Requirement: Typed reasoning-only response compatibility

Smith SHALL support a provenance-aware provider response policy that can
promote a successful non-redacted reasoning-only attempt to ordinary visible
text. The default and an omitted policy MUST preserve reasoning classification.
The policy MUST NOT promote redacted reasoning or reasoning associated with an
ordinary text response or tool call.

#### Scenario: GLM returns only non-redacted reasoning content

- **GIVEN** provider `zai` declares `reasoning_only = "text"`
- **AND** one successful attempt returns non-redacted reasoning without
  ordinary text or a tool call
- **WHEN** Smith normalizes the completed attempt
- **THEN** the accumulated content is emitted exactly once as visible text
- **AND** the turn is recorded as having visible output

#### Scenario: Reasoning precedes ordinary text

- **GIVEN** provider `zai` declares `reasoning_only = "text"`
- **WHEN** one attempt emits reasoning and then ordinary text
- **THEN** the earlier content remains reasoning
- **AND** only the ordinary text is the visible answer

#### Scenario: Reasoning precedes a tool call

- **GIVEN** provider `zai` declares `reasoning_only = "text"`
- **WHEN** one attempt emits reasoning and then requests a tool
- **THEN** the earlier content remains reasoning
- **AND** tool execution and continuation semantics are unchanged

#### Scenario: Reasoning is redacted

- **GIVEN** a configured provider returns only redacted reasoning
- **WHEN** the attempt completes
- **THEN** Smith does not promote or reveal the redacted content
- **AND** the turn remains without visible output

#### Scenario: Response policy is omitted

- **GIVEN** an OpenAI-compatible provider has no reasoning-only policy
- **WHEN** it emits ordinary OpenRouter-style text or reasoning-only output
- **THEN** Smith preserves the adapter's existing event classifications

#### Scenario: Response policy is invalid or incompatible

- **GIVEN** the configured value is not `reasoning` or `text`, or the selected
  adapter cannot expose compatible reasoning events
- **WHEN** Smith resolves configuration
- **THEN** resolution fails before credential lookup or network access
- **AND** the diagnostic identifies the non-secret incompatible setting

### Requirement: Secret-safe credential enrollment

Setup SHALL enroll entered API keys through a secret-bearing path separate from
ordinary display-safe settings. Configuration and setup previews MUST redact an
inline `api_key`. Secret input MUST NOT appear in normal render state, logs,
diagnostics, events, journals, or failed-transaction artifacts.

#### Scenario: Store an API key in the platform service

- **GIVEN** the operating-system credential service is available
- **WHEN** the user chooses protected storage and enters an API key
- **THEN** Smith stores it under the reviewed service/account identity
- **AND** user configuration records only a
  `keychain:smith/<provider>` reference

#### Scenario: Store an API key in user config

- **GIVEN** the user chooses no-prompt config storage
- **WHEN** they enter and confirm an API key
- **THEN** setup writes the key only to the provider's `api_key` field
- **AND** every review, collision, success, and error surface renders
  `[redacted]` instead of the value

#### Scenario: Use an environment-managed credential

- **GIVEN** the user chooses to manage the credential through the environment
- **WHEN** they enter a valid variable name
- **THEN** Smith records only `env:<NAME>`
- **AND** setup neither reads nor copies the environment value into Smith state

#### Scenario: Credential service is unavailable

- **GIVEN** protected credential storage is unavailable or denied
- **WHEN** enrollment fails
- **THEN** setup remains at the authentication step with an actionable error
- **AND** offers the environment and plaintext user-config paths
- **AND** does not choose either fallback automatically

### Requirement: Reviewed user-scope setup transaction

Setup SHALL write only user-controlled configuration under `~/.smith/`; it
MUST NOT modify project `.smith/` files. The proposed edit MUST be reviewed
with secret values redacted, published through a mode-`0600` same-directory
atomic replace, and preserve unrelated existing user configuration. Setup is
complete only after full local preflight.

#### Scenario: Commit fresh inline-key setup

- **GIVEN** the user config does not exist and every setup choice validates
- **WHEN** the user confirms an inline-key review
- **THEN** Smith creates the user directory and config with restrictive
  permissions and atomically publishes the reviewed content
- **AND** no temporary file is group or world accessible

#### Scenario: Explicit setup encounters an existing inline key

- **GIVEN** setup proposes replacing a provider `api_key`
- **WHEN** an existing value differs
- **THEN** review identifies the credential field but renders both values as
  `[redacted]`
- **AND** does not replace it without explicit confirmation

#### Scenario: Setup is cancelled or preflight fails

- **GIVEN** setup has not completed full preflight
- **WHEN** the user cancels or a write or preflight operation fails
- **THEN** Smith restores the exact prior config bytes
- **AND** removes secret-bearing temporary artifacts
- **AND** reports how to retry without revealing secret material

### Requirement: Additive provider and model setup

Explicit setup SHALL support adding a provider with its first model and adding
another model to an existing named provider. It MUST preserve existing
provider/model records, validate each model against that provider, and change
the default profile or model only when the user explicitly confirms that
selection.

#### Scenario: Add a second model to an existing provider

- **GIVEN** user configuration declares provider `acme`
- **WHEN** the user runs `smith setup add-model --provider acme` and confirms a
  valid model with complete trusted or explicit limits
- **THEN** Smith adds the `models."acme/<model>"` record
- **AND** preserves the provider credential, endpoint, existing models, and
  unrelated configuration

#### Scenario: Add a model for another provider

- **GIVEN** multiple providers are configured
- **WHEN** the user chooses Add model without a provider argument
- **THEN** setup requires an explicit provider selection before collecting the
  model
- **AND** writes the model only under the selected provider/model identity

#### Scenario: Keep the current default

- **GIVEN** setup successfully adds a provider or model
- **WHEN** the user declines to make it the default
- **THEN** the existing default profile and active model remain unchanged
- **AND** the new configuration is available for later `/provider`, `/model`,
  or `/profile` selection

### Requirement: Locally selectable runtime inventory

Smith SHALL enumerate effective profiles, configured providers, and
declaratively valid provider/model pairs for interactive selection. Every model
entry MUST retain its provider identity and limit provenance. Enumeration MUST
be deterministic and MUST NOT resolve a credential, open the platform
credential service, or make a network request.

#### Scenario: Models exist under multiple providers

- **GIVEN** effective configuration contains valid models for providers `zai`
  and `openrouter`
- **WHEN** Smith builds the model selection inventory
- **THEN** it returns every valid pair with a provider-qualified identity
- **AND** marks the active provider/model pair without attaching either
  provider's models to the other

#### Scenario: Duplicate model names are configured

- **GIVEN** two providers declare the same unqualified model name
- **WHEN** Smith builds or resolves a model selection
- **THEN** both provider-qualified entries remain distinguishable
- **AND** an unqualified direct selection is rejected as ambiguous unless the
  active provider resolves it uniquely

#### Scenario: A configured model lacks enforceable metadata

- **GIVEN** a provider/model pair lacks explicit or trusted catalog limits
- **WHEN** Smith builds the selectable inventory
- **THEN** the pair is not offered as runnable
- **AND** local guidance points to `smith setup add-model` without guessing
  limits or querying the provider

#### Scenario: Inventory is built before credential use

- **GIVEN** a configured provider references a locked or missing credential
- **WHEN** Smith lists local profiles, providers, and models
- **THEN** listing succeeds from declarative configuration
- **AND** credential availability is checked only during full preflight after
  the user chooses a candidate

### Requirement: Catalog-augmented runtime inventory

Smith SHALL augment the pure local selection inventory with models from an
immutable, schema-validated catalog snapshot only for configured providers
whose available adapter and normalized endpoint match a Smith-owned catalog
binding. Catalog data MUST retain the configured provider's local identity and
MUST NOT create or modify a provider, endpoint, adapter, credential, header, or
profile.

#### Scenario: Configured OpenRouter exposes catalog models

- **GIVEN** effective configuration declares an available OpenAI-compatible
  provider at the normalized OpenRouter endpoint
- **AND** the local catalog snapshot contains valid OpenRouter models not named
  by local model records or profiles
- **WHEN** Smith builds the selection inventory
- **THEN** it includes those valid models under the configured provider's local
  identity
- **AND** it does not add any other Models.dev provider

#### Scenario: Z.AI quick start uses the Coding Plan catalog

- **GIVEN** effective configuration declares Smith's Z.AI Coding Plan endpoint
- **AND** the local catalog snapshot contains several valid
  `zai-coding-plan` models
- **WHEN** Smith builds the selection inventory
- **THEN** it includes those models as `zai/<model-id>` pairs
- **AND** retains the local provider identity `zai`

#### Scenario: Familiar provider name points elsewhere

- **GIVEN** a configured provider is named `openrouter`
- **BUT** its normalized endpoint is not the Smith-owned OpenRouter binding
- **WHEN** Smith builds the selection inventory
- **THEN** it does not attach the OpenRouter catalog to that provider
- **AND** only locally configured or otherwise trusted models remain candidates

#### Scenario: Inventory consumes a prepared snapshot

- **GIVEN** the host has prepared an immutable catalog snapshot
- **WHEN** Smith enumerates profiles, providers, and models or filters a picker
- **THEN** enumeration reads only configuration and that in-memory snapshot
- **AND** does not access a network, provider credential, keychain, or provider
  endpoint

### Requirement: Catalog model validation and precedence

Smith SHALL normalize only schema-valid catalog metadata into provider-scoped
model records. Explicit Smith model configuration MUST retain field-level
precedence over catalog metadata, and every winning catalog field MUST identify
its catalog revision and retrieval provenance. A catalog entry MAY carry an
optional per-counter price in USD per million tokens; an entry whose price
block is absent, incomplete, or ill-typed MUST remain a valid selectable model
record with no price rather than a rejected or partially-priced one.

#### Scenario: Complete catalog limits are normalized

- **GIVEN** a catalog model publishes positive context and output limits within
  Smith's integer bounds
- **AND** its optional separate input limit is absent
- **WHEN** Smith normalizes the model record
- **THEN** `context_tokens` is the published context limit
- **AND** `max_output_tokens` is the published output limit
- **AND** `max_input_tokens` is the published total context limit
- **AND** runtime context policy still holds back declared output and reasoning
  reserves before admitting input

#### Scenario: Separate input limit is published

- **GIVEN** a catalog model publishes a positive input limit no greater than its
  total context limit
- **WHEN** Smith normalizes the model record
- **THEN** `max_input_tokens` is that separate published input limit
- **AND** no larger input ceiling is inferred

#### Scenario: Invalid catalog limits

- **GIVEN** a catalog entry has a zero or out-of-range limit, output above
  context, or a separate input limit above context
- **WHEN** Smith validates the snapshot
- **THEN** that entry cannot become a selectable model record
- **AND** Smith does not clamp or guess a replacement limit

#### Scenario: A published price is normalized

- **GIVEN** a catalog model publishes finite non-negative input, output, cache
  read, and cache write costs
- **WHEN** Smith normalizes the model record
- **THEN** each cost is retained per counter in USD per million tokens
- **AND** it carries the same catalog revision and retrieval provenance as the
  entry's other fields

#### Scenario: A partial or ill-typed price

- **GIVEN** a catalog model publishes only an input cost, or publishes a
  negative, infinite, or non-numeric cost
- **WHEN** Smith normalizes the model record
- **THEN** the model stays selectable
- **AND** only the individually valid costs are retained, with no value
  inferred for a counter the source did not price

#### Scenario: A price cannot disable a model

- **GIVEN** a catalog model publishes no price at all
- **WHEN** Smith prepares runtime choices
- **THEN** the model is selectable exactly as it is today
- **AND** the absence of a price is not a validation diagnostic

#### Scenario: A snapshot at the prior schema revision

- **GIVEN** a cached catalog snapshot was written at the schema revision before
  prices existed
- **WHEN** Smith loads it
- **THEN** it is rejected as stale by the existing revision check
- **AND** Smith falls back to the embedded seed and schedules a refresh, as it
  does for any stale revision

#### Scenario: Effective reserves leave no input budget

- **GIVEN** a catalog model has internally valid published limits
- **BUT** the effective Smith output and reasoning reserves equal or exceed its
  context window
- **WHEN** Smith prepares runtime choices
- **THEN** the model is visible but disabled with a local reserve diagnostic
- **AND** Smith does not lower the published output ceiling or configured
  reserve to make it selectable

#### Scenario: Explicit limit overrides catalog metadata

- **GIVEN** a catalog record and an explicit
  `[models."<provider>/<model>"]` record both supply a limit field
- **WHEN** Smith resolves the model profile
- **THEN** the explicit field wins through the existing catalog precedence
- **AND** catalog and explicit contributions remain available for provenance
  diagnostics

#### Scenario: Catalog cannot grant provider entitlement

- **GIVEN** a valid catalog model is associated with a configured provider
- **WHEN** Smith adds it to the inventory
- **THEN** Smith labels it as catalog-advertised rather than account-verified
- **AND** does not claim that the credential, subscription, region, or account
  can use that model

### Requirement: Layered agent-mode configuration

Smith SHALL resolve root modes and child presets through typed user-controlled
configuration with deterministic built-in, user, trusted-project, and session
precedence. Mode definitions may narrow prompt/capability/model preferences but
MUST NOT grant permissions, trust, credentials, or approval authority.

#### Scenario: User reorders built-in modes
- **GIVEN** owner-controlled configuration orders `plan`, `build`, `review`
- **WHEN** the user cycles modes
- **THEN** Smith follows that validated order
- **AND** each effective view remains an intersection with run authority

#### Scenario: Project mode requests broader authority
- **GIVEN** a trusted or untrusted project mode declares shell or write grants
- **WHEN** configuration is resolved
- **THEN** the authority-bearing fields are rejected
- **AND** provenance identifies the project key without executing it

### Requirement: Secret-safe checkpoint-key configuration

Smith SHALL accept a checkpoint-protection key only from an explicit
higher-precedence environment value, an owner-only inline user-config value,
or a protected credential reference. Sources are mutually exclusive after
layer resolution, project configuration is forbidden from supplying or
redirecting them, and all observable forms MUST redact the key.

#### Scenario: Use an inline no-prompt key
- **GIVEN** mode-`0600` user configuration contains a valid inline checkpoint
  key
- **WHEN** Smith initializes persistence
- **THEN** it uses that key without calling Keychain or Secret Service
- **AND** config explanation names only the source and redacted setting

#### Scenario: Inline key file is not private
- **GIVEN** user configuration containing a checkpoint key is a symlink,
  non-regular, wrong-owner, group-readable, or world-readable
- **WHEN** Smith resolves configuration
- **THEN** startup fails before checkpoint, credential-service, provider, or
  terminal I/O
- **AND** no diagnostic includes any part of the key

#### Scenario: Project supplies a checkpoint key
- **GIVEN** project configuration defines a checkpoint key or key reference
- **WHEN** Smith loads layers
- **THEN** it rejects the setting as user-scoped security policy
- **AND** does not copy, query, or use the value

### Requirement: Reviewed checkpoint-key setup

`smith setup checkpoint-key` SHALL offer protected OS storage, environment
reference, and explicit `Store in config (no prompts)` choices. The local
choice MUST generate a cryptographically random key, warn about same-user and
backup exposure, redact review, publish atomically at mode `0600`, and roll
back exact prior bytes on failure.

#### Scenario: Enroll a local checkpoint key
- **GIVEN** the user explicitly accepts the plaintext-key warning
- **WHEN** setup completes local generation and full persistence preflight
- **THEN** later Smith startup opens no credential-service prompt
- **AND** exact checkpoints remain authenticated-encrypted

#### Scenario: Setup is cancelled
- **GIVEN** setup generated secret material but has not committed
- **WHEN** the user cancels or preflight fails
- **THEN** Smith zeroizes/removes temporary material and restores prior config
- **AND** emits only redacted recovery guidance

### Requirement: Source-explainable reasoning controls

Smith SHALL resolve reasoning presence separately from adjustable reasoning
controls. Control metadata SHALL identify switch behavior, supported efforts,
optional token-budget support, provider wire dialect, defaults, and provenance;
a boolean reasoning capability alone MUST NOT imply controllability.

#### Scenario: Boolean reasoning remains fixed

- **GIVEN** a catalog record declares only `reasoning = true`
- **WHEN** Smith resolves the model profile
- **THEN** reasoning is recorded as present but fixed
- **AND** no toggle, effort, budget, or wire dialect is inferred

#### Scenario: Rich trusted metadata declares controls

- **GIVEN** trusted metadata for an exact provider/model binding declares a
  toggle, supported effort levels, defaults, and a request dialect
- **WHEN** Smith resolves the model profile
- **THEN** the control profile retains every value and its source
- **AND** `/status` can explain where the effective control came from

### Requirement: Layered reasoning defaults

Smith SHALL allow profiles to declare typed reasoning enabled-state and effort
defaults. Smith MUST validate requested defaults against the selected provider/model controls
before constructing a runtime and MUST preserve provider defaults when no
Smith value is configured.

#### Scenario: Supported profile default resolves

- **GIVEN** a profile requests enabled reasoning at `high` effort
- **AND** the selected provider/model advertises that combination
- **WHEN** Smith resolves the configuration
- **THEN** the runtime policy carries the configured values and source

#### Scenario: Unsupported effort fails before runtime construction

- **GIVEN** a profile requests an effort absent from the capability snapshot
- **WHEN** Smith resolves the configuration
- **THEN** startup fails with the requested value and supported alternatives
- **AND** no credential lookup or provider request is performed

#### Scenario: Omitted reasoning configuration preserves provider behavior

- **GIVEN** no layer configures enabled state or effort
- **WHEN** Smith constructs the runtime
- **THEN** it preserves the provider/model default
- **AND** it does not synthesize `low`, enable reasoning, or disable reasoning

### Requirement: Compatible persisted reasoning override

Smith SHALL persist a session reasoning override additively and revalidate it
against the frozen capability snapshot during resume. Older sessions without
the field MUST preserve provider/model defaults.

#### Scenario: Compatible override resumes

- **GIVEN** a saved session contains a supported thinking and effort override
- **WHEN** the session resumes against a compatible capability snapshot
- **THEN** the override remains effective and source-labelled

#### Scenario: Legacy session has no override

- **GIVEN** a saved session predates the reasoning override field
- **WHEN** the session resumes
- **THEN** deserialization succeeds
- **AND** the provider/model default remains effective

### Requirement: Unified agent profile declarations

Smith SHALL use named profiles as the single declarative agent-preset type for
main-agent selection and explicit direct-child creation. A profile MAY contain
bounded description and instructions, an authority-narrowing posture,
main/child availability, provider/model preferences, and existing profile
policy fields; none of those fields may grant trust, credentials, permissions,
approval, workspace scope, or host capabilities.

#### Scenario: Use one profile on the main agent
- **GIVEN** a valid profile is available for `main`
- **WHEN** startup, `--profile`, or `/profile` selects it
- **THEN** Smith resolves its instructions, posture, provider/model, and limits
  through the normal typed profile-precedence layer
- **AND** applies the effective profile atomically at a safe runtime boundary

#### Scenario: Expose one profile to both placements
- **GIVEN** a valid profile declares `use = ["main", "child"]`
- **WHEN** Smith builds its local profile inventory
- **THEN** the same named declaration is eligible for main selection and
  explicit child invocation
- **AND** each placement independently intersects the profile with host policy

#### Scenario: Existing profile omits availability
- **GIVEN** a pre-change profile contains only existing runtime fields
- **WHEN** the transition release resolves it
- **THEN** Smith treats it as a main-enabled build profile
- **AND** does not expose it for child creation without an explicit child use

### Requirement: Deterministic agent profile inheritance

Smith SHALL allow a profile to extend at most one named profile and SHALL
resolve the effective fields with bounded, acyclic, source-explainable
inheritance before provider, credential, session, or terminal I/O. Child fields
replace inherited scalar or section fields, and instruction bodies MUST NOT be
implicitly concatenated.

#### Scenario: Reuse a provider and model baseline
- **GIVEN** `plan` extends a valid `work` profile and overrides posture and
  instructions
- **WHEN** Smith resolves `plan`
- **THEN** it inherits the provider/model and other unmodified fields from
  `work`
- **AND** provenance identifies the winning source for every effective field

#### Scenario: Inheritance contains a cycle
- **GIVEN** two or more profiles form an inheritance cycle
- **WHEN** configuration preflight resolves any affected profile
- **THEN** Smith fails before credential, provider, session, or terminal I/O
- **AND** the diagnostic identifies the bounded cycle and source declarations

### Requirement: Legacy agent configuration migration

Smith SHALL accept existing root-mode and child-preset declarations through an
explicit one-release compatibility adapter, emit source-explainable migration
guidance, and fail closed when a legacy declaration conflicts with a new
profile of the same effective name. Smith MUST NOT silently select a winner or
change a legacy run profile into a child-enabled profile.

#### Scenario: Load an existing child preset
- **GIVEN** configuration declares a valid read-only `[child_agents.inspect]`
- **WHEN** the transition release builds the profile inventory
- **THEN** it exposes an equivalent deprecated child-only preset
- **AND** reports the replacement profile shape without changing authority

#### Scenario: Legacy and new names collide
- **GIVEN** a legacy mode or child preset conflicts with a new profile of the
  same effective name
- **WHEN** Smith resolves declarations
- **THEN** resolution fails with both source locations
- **AND** no map order, file order, or precedence rule hides the collision

### Requirement: Trusted authentication descriptors

Smith SHALL define authentication methods in trusted product descriptors keyed
by provider. Project configuration MUST NOT define or
override OAuth issuers, client identities, scopes, redirect URIs, token
endpoints, callback listeners, or credential storage locations.

#### Scenario: Enumerate supported connection methods

- **GIVEN** a provider has a trusted descriptor and its required adapter or
  backend is available
- **WHEN** Smith builds the `/connect` inventory
- **THEN** it lists only the descriptor's supported authentication methods
- **AND** does not contact the provider, credential service, or OAuth issuer

#### Scenario: Project attempts to redirect OAuth

- **GIVEN** project-controlled configuration declares an OAuth endpoint,
  client identity, scope, callback, or storage override
- **WHEN** Smith resolves configuration
- **THEN** it rejects the setting before credential, terminal, or provider I/O
- **AND** does not include any supplied secret-shaped value in diagnostics

### Requirement: Renewable credential persistence

OAuth access and refresh material SHALL be treated as user-scope secrets.
Smith-managed ChatGPT refresh material MUST default to the fixed owner-only
plaintext `~/.smith/auth.json` store and MUST NOT initialize an OS credential
service. Configuration SHALL contain only a typed non-secret credential
reference.

#### Scenario: Persist a renewable provider credential

- **GIVEN** a trusted direct-provider OAuth integration completes successfully
- **WHEN** Smith commits the connection
- **THEN** refresh material is written to the reviewed user-scope auth-file
  backend with owner-only permissions and atomic replacement
- **AND** configuration stores only the non-secret credential-source identity

#### Scenario: Smith owns ChatGPT credentials

- **GIVEN** ChatGPT login completes through Smith's trusted OAuth integration
- **WHEN** Smith records connection readiness
- **THEN** Smith stores the versioned access, refresh, expiry, and account
  bundle only in the `chatgpt` entry of `~/.smith/auth.json`
- **AND** configuration records only `authfile:chatgpt`
- **AND** Smith neither reads nor mutates another client's auth cache

#### Scenario: Auth file is private but plaintext

- **GIVEN** Smith creates or replaces the ChatGPT auth-file entry
- **WHEN** it publishes the updated file
- **THEN** `~/.smith` is mode `0700` and `auth.json` is a regular mode `0600`
  file on supported Unix hosts
- **AND** Smith refuses symlink, non-regular, malformed, or oversized storage
- **AND** help and documentation warn that same-user processes and backups can
  read or retain the plaintext tokens

#### Scenario: ChatGPT lifecycle avoids the credential service

- **GIVEN** a developer Keychain entry exists or macOS would prompt for access
- **WHEN** Smith connects, resolves, refreshes, reconnects, or disconnects
  ChatGPT through `authfile:chatgpt`
- **THEN** it performs zero Keychain or Secret Service operations
- **AND** it neither imports nor deletes the legacy `keychain:smith/chatgpt`
  entry

### Requirement: Transactional connection changes

Connection, reconnection, and disconnection MUST preserve unrelated user
configuration and prior credential state until local preflight and any
required safe-boundary runtime replacement succeed. Failure or cancellation
MUST restore the exact prior Smith-owned state.

#### Scenario: Active-provider reconnection fails preflight

- **GIVEN** a replacement credential has been enrolled and the prior runtime is
  still restorable
- **WHEN** full local preflight or safe-boundary runtime replacement fails
- **THEN** Smith restores the prior config bytes and credential value
- **AND** keeps the prior runtime/session active when restoration succeeds

#### Scenario: OAuth ceremony fails before persistence

- **GIVEN** browser or device login has not reported success
- **WHEN** it times out, is denied, is cancelled, or reports a protocol error
- **THEN** Smith commits no connection state
- **AND** removes memory-only callback and PKCE material
