## ADDED Requirements

### Requirement: In-session provider connection

Smith SHALL provide `/connect [PROVIDER]` as an idle-only local command that
selects a provider and one of its supported authentication methods. The
connection ceremony MUST NOT send an inference request, and durable changes
MUST use the reviewed user-scope credential transaction.

#### Scenario: Connect OpenRouter from the provider picker

- **GIVEN** the user submits `/connect` while the session is idle
- **WHEN** they select OpenRouter, choose protected API-key storage, enter a
  key, and confirm the secret-free review
- **THEN** Smith stores the key through the reviewed credential transaction
- **AND** records the standard OpenRouter provider endpoint without requiring
  the user to type it
- **AND** sends no inference request during connection

#### Scenario: Reconnect an existing provider

- **GIVEN** a configured provider already has endpoint, models, limits,
  profiles, and a selected default
- **WHEN** the user connects that provider with a replacement credential
- **THEN** Smith changes only its authentication source
- **AND** preserves all unrelated provider and selection fields

#### Scenario: Connect while work is active

- **GIVEN** a turn, approval, child, or runtime replacement is active
- **WHEN** the user invokes `/connect`
- **THEN** Smith refuses or defers the action through the ordinary idle-boundary
  policy
- **AND** does not start login or credential persistence

### Requirement: Interactive OAuth ceremony

The connection surface SHALL support browser-URL and device-code login states
with explicit progress, cancellation, timeout, retry, and completion. It MUST
display only public authorization instructions and MUST NOT retain or render
authorization codes, access tokens, refresh tokens, PKCE verifiers, or callback
payloads.

#### Scenario: Complete browser login

- **GIVEN** the selected trusted auth method returns a public authorization URL
- **WHEN** the user completes Smith's loopback PKCE flow and token exchange
- **THEN** Smith marks the connection ready with its non-secret method/backend
  identity
- **AND** no token value enters the transcript, render state, or diagnostic

#### Scenario: Complete device-code login

- **GIVEN** browser callback login is unsuitable and device login is available
- **WHEN** Smith displays the verification URL and one-time user code
- **THEN** the user can complete login in another browser
- **AND** Smith stops polling at success, cancellation, expiry, or its bounded
  deadline

#### Scenario: Cancel OAuth login

- **GIVEN** an OAuth ceremony is waiting for completion
- **WHEN** the user presses Escape or Ctrl-C
- **THEN** Smith cancels the trusted login backend and closes temporary local
  listeners or tasks
- **AND** restores the prior connection and terminal state without writing a
  credential

### Requirement: Connection removal and visibility

Smith SHALL provide `/disconnect [PROVIDER]` and local connection status.
Disconnecting MUST clear Smith-owned credential material while preserving
unrelated provider/model setup.

#### Scenario: Disconnect ChatGPT

- **GIVEN** Smith owns a ChatGPT token bundle in its owner-only auth file
- **WHEN** the user confirms `/disconnect chatgpt`
- **THEN** Smith atomically removes the auth-file entry and provider credential
  source
- **AND** does not read, mutate, or depend on a Codex or OpenCode auth cache
- **AND** does not query or remove a legacy Smith Keychain entry

#### Scenario: Disconnect an inactive API-key provider

- **GIVEN** a configured inactive provider uses Smith-owned protected storage
- **WHEN** the user confirms `/disconnect` for that provider
- **THEN** Smith removes the reviewed credential entry and provider credential
  source
- **AND** preserves its endpoint, models, limits, and profiles

#### Scenario: Disconnect the only active provider

- **GIVEN** the current session has no other usable provider
- **WHEN** the user requests disconnection
- **THEN** Smith requires a replacement connection or session exit before
  committing
- **AND** never leaves the session presented as runnable without authentication
