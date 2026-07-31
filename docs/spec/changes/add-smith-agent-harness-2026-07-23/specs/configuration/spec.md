## ADDED Requirements

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
selection, credential reference, and only the options supported by that
adapter. Smith SHALL validate adapter-specific keys before constructing the
shared provider.

#### Scenario: Configure an OpenAI-compatible provider

- **GIVEN** a profile selects an OpenAI-compatible provider with a base URL,
  model, credential reference, and enforceable limits
- **WHEN** Smith builds the configured runtime
- **THEN** it constructs the shared OpenAI-compatible adapter over Smith's
  production HTTP transport
- **AND** the secret value is resolved only at that construction boundary

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

Smith MUST store provider credentials outside project files using the macOS
Keychain or Linux Secret Service when available. An encrypted-file fallback
MAY be enabled explicitly, but its passphrase or master key MUST be supplied
from outside the ciphertext file. Config files SHALL contain secret references,
not plaintext keys.

#### Scenario: Save a provider API key

- **GIVEN** a supported OS credential service is available
- **WHEN** the user saves a provider key
- **THEN** Smith stores the secret in that service
- **AND** writes only its reference into Smith configuration

#### Scenario: Credential service is unavailable

- **GIVEN** no supported credential service is available
- **WHEN** the user explicitly chooses encrypted-file storage
- **THEN** Smith encrypts the secret at rest under user state
- **AND** requires a passphrase or external master key to decrypt it

### Requirement: Secret redaction

Smith SHALL redact known secrets from logs, events, command previews,
extension payloads, diagnostic metadata, and persisted provider error details.

#### Scenario: Provider error echoes a key

- **GIVEN** a provider error body contains a configured credential value
- **WHEN** Smith records and displays the failure
- **THEN** the stored and visible forms replace the credential with a redaction
  marker
