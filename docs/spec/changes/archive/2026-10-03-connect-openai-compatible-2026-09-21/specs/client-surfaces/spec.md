## MODIFIED Requirements

### Requirement: In-session provider connection

Smith SHALL provide `/connect [PROVIDER]` as an idle-only local command that
selects a provider and one of its supported authentication methods. The
picker SHALL include a generic OpenAI-compatible entry that runs the same
reviewed add-provider ceremony as `smith setup add-provider`, so endpoints
without a built-in flow are connectable in-session. The connection ceremony
MUST NOT send an inference request, and durable changes MUST use the reviewed
user-scope credential transaction.

#### Scenario: Connect OpenRouter from the provider picker

- **GIVEN** the user submits `/connect` while the session is idle
- **WHEN** they select OpenRouter, choose protected API-key storage, enter a
  key, and confirm the secret-free review
- **THEN** Smith stores the key through the reviewed credential transaction
- **AND** records the standard OpenRouter provider endpoint without requiring
  the user to type it
- **AND** sends no inference request during connection

#### Scenario: Connect a custom OpenAI-compatible endpoint

- **GIVEN** the user submits `/connect` while the session is idle
- **WHEN** they select the OpenAI-compatible entry and complete the add-provider
  ceremony (distinct provider name, endpoint, authentication, and first usable
  model with enforceable limits) through the secret-free review
- **THEN** Smith adds the provider through the same reviewed user-config
  transaction as `smith setup add-provider`
- **AND** sends no inference request during connection
- **AND** the new provider/model becomes selectable after the session rebuild

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
