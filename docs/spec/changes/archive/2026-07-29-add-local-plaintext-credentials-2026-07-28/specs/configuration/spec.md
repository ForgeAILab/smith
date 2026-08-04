## MODIFIED Requirements

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
