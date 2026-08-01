## ADDED Requirements

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
