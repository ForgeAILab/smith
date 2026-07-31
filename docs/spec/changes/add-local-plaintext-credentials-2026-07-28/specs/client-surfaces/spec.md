## ADDED Requirements

### Requirement: Explicit no-prompt credential setup

Smith SHALL offer plaintext user-config storage as an explicit no-prompt
authentication choice and SHALL support changing only an existing provider's
credential storage. Every review and result surface MUST state the at-rest
risk while redacting the value.

#### Scenario: User reviews config authentication

- **GIVEN** authentication offers Keychain, environment, and local-config
  choices
- **WHEN** the user selects “Store in config (no prompts)”
- **THEN** setup explains same-user process exposure and backup risk
- **AND** the API-key field remains masked
- **AND** review shows `api_key = [redacted]`

#### Scenario: User migrates an existing provider

- **GIVEN** provider `zai` already has a valid endpoint, model, limits, and a
  `keychain:` credential reference
- **WHEN** the user runs `smith setup credential --provider zai`, selects
  config storage, enters a key, and confirms
- **THEN** setup changes only that provider's credential source
- **AND** full preflight uses the unchanged provider/model without opening the
  Keychain
- **AND** the next ordinary Smith startup opens no credential-service prompt

#### Scenario: Credential migration fails preflight

- **GIVEN** setup has atomically published a candidate config containing the
  inline key
- **WHEN** runtime preflight fails
- **THEN** setup restores the exact prior config bytes
- **AND** errors, review state, temporary files, stdout, and stderr contain no
  key value

#### Scenario: Existing non-prompting source remains selected

- **GIVEN** a provider already uses `env:` or `api_key`
- **WHEN** the user reviews or cancels credential migration
- **THEN** Smith does not consult the Keychain
- **AND** cancellation writes nothing
