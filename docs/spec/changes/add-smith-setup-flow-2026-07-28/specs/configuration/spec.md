## ADDED Requirements

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

Setup SHALL enroll entered API keys through an injectable user-scoped
credential writer separate from the runtime's read-only resolver. Configuration
and setup previews MUST contain only a credential reference. Secret input MUST
NOT appear in normal render state, logs, diagnostics, events, journals, or
temporary config files.

#### Scenario: Store an API key in the platform service

- **GIVEN** the operating-system credential service is available
- **WHEN** the user chooses protected storage and enters an API key
- **THEN** Smith stores it under the reviewed service/account identity
- **AND** user configuration records only a
  `keychain:smith/<provider>` reference

#### Scenario: Use an environment-managed credential

- **GIVEN** the user chooses to manage the credential through the environment
- **WHEN** they enter a valid variable name
- **THEN** Smith records only `env:<NAME>`
- **AND** setup neither reads nor copies the environment value into Smith state

#### Scenario: Credential service is unavailable

- **GIVEN** protected credential storage is unavailable or denied
- **WHEN** enrollment fails
- **THEN** setup remains at the authentication step with an actionable error
- **AND** offers the environment-reference path
- **AND** does not fall back to plaintext configuration

#### Scenario: Credential lookup waits on a platform prompt

- **GIVEN** a configured keychain reference waits indefinitely for an unlock
  or access-control decision
- **WHEN** runtime construction or setup preflight resolves that reference
- **THEN** Smith stops waiting at a bounded startup deadline
- **AND** reports how to unlock or allow the service or use an `env:<VAR>`
  reference
- **AND** no provider request or session is created

### Requirement: Reviewed user-scope setup transaction

Setup SHALL write only user-controlled configuration under `~/.smith/`; it
MUST NOT modify project `.smith/` files. The proposed non-secret edit MUST be
reviewed before commit, published with a same-directory atomic replace, and
preserve unrelated existing user configuration. Setup is complete only after
the committed result passes full configuration, credential, model-profile,
workspace, and runtime-factory preflight.

#### Scenario: Commit fresh setup

- **GIVEN** the user config does not exist and every setup choice validates
- **WHEN** the user confirms the review
- **THEN** Smith creates the user directory and config with restrictive
  permissions and atomically publishes the reviewed content
- **AND** starts no session until full preflight succeeds

#### Scenario: Explicit setup encounters existing entries

- **GIVEN** `smith setup` proposes a profile, provider, or model key that
  already exists in user configuration
- **WHEN** the existing value differs
- **THEN** Smith preserves unrelated content and shows the exact non-secret
  collision in the review
- **AND** does not replace it without explicit confirmation

#### Scenario: Setup is cancelled or preflight fails

- **GIVEN** setup has not completed full preflight
- **WHEN** the user cancels or an enrollment, write, or preflight operation
  fails
- **THEN** Smith does not record setup as complete
- **AND** restores the prior config file when it had published a candidate
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
