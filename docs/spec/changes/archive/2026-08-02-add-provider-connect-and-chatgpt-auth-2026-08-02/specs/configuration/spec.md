## ADDED Requirements

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
