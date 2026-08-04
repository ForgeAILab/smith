## ADDED Requirements

### Requirement: Experimental Smith-native ChatGPT authentication

Smith SHALL offer ChatGPT subscription login as an explicitly experimental,
unsupported direct integration. Smith SHALL perform the trusted browser PKCE
or device-code ceremony itself, exchange and refresh tokens at the fixed
issuer, and persist only its own versioned bundle in the fixed owner-only
plaintext `~/.smith/auth.json` store.

#### Scenario: Connect with ChatGPT in a browser

- **GIVEN** the user selects experimental ChatGPT browser login from `/connect`
- **WHEN** Smith opens the reviewed authorization URL and receives a valid
  state-bound loopback callback
- **THEN** Smith exchanges the code directly, validates the returned account
  identity, and commits the owner-only auth-file/config transaction
- **AND** no Codex process or another client's auth cache is required
- **AND** no Keychain or Secret Service operation occurs

#### Scenario: Device-code login is disabled by policy

- **GIVEN** the account or workspace does not permit device-code login
- **WHEN** the issuer rejects that method
- **THEN** Smith reports a fixed classified policy failure
- **AND** retains browser login or API-key choices without selecting one
  automatically

#### Scenario: Callback state is forged

- **GIVEN** a browser callback has a missing or mismatched state or an
  unexpected target
- **WHEN** Smith's loopback listener receives it
- **THEN** Smith rejects the callback, writes no credential, and closes the
  bounded ceremony
- **AND** callback parameters are absent from diagnostics and render state

### Requirement: Direct ChatGPT Responses execution

Smith SHALL call the fixed experimental ChatGPT Codex Responses backend
directly through its normal Agent Runtime provider path. Status and help MUST
identify Smith as execution owner and label the public support boundary.

#### Scenario: Start ChatGPT-backed work

- **GIVEN** Smith-native login and direct-provider preflight succeed
- **WHEN** the user selects a trusted ChatGPT model
- **THEN** Smith sends canonical work through the dedicated Responses adapter
- **AND** the ordinary Smith runtime owns tools, approvals, persistence,
  cancellation, recovery, events, and usage
- **AND** no external agent loop is started

#### Scenario: Required Smith policy is unavailable

- **GIVEN** a request cannot be represented without violating a Smith tool,
  approval, checkpoint, or recovery guarantee
- **WHEN** the adapter preflights or decodes it
- **THEN** the request fails before unsafe work is accepted
- **AND** does not silently substitute Codex behavior

### Requirement: No external client dependency or credential reuse

Smith MUST NOT launch Codex for login or inference, extract Codex/OpenCode
managed tokens, or read another client's token cache. The trusted integration
MAY pin public native-client parameters and the currently observed direct
backend only behind the approved experimental disclosure.

#### Scenario: Codex is not installed

- **GIVEN** no Codex executable or auth cache is present
- **WHEN** the user connects and runs the experimental ChatGPT provider
- **THEN** login and inference remain available through Smith's own OAuth,
  auth file, refresh source, and HTTP adapter
- **AND** no behavior changes based on Codex installation state

#### Scenario: Direct contract becomes incompatible

- **GIVEN** the undocumented OAuth or backend behavior changes incompatibly
- **WHEN** Smith detects a fixed protocol, authentication, or stream failure
- **THEN** it reports the experimental integration as unavailable with a
  redaction-safe actionable message
- **AND** points to OpenAI Platform API-key access as the supported fallback
