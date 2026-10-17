## MODIFIED Requirements

### Requirement: WASM module calls are bounded and isolated

Every call into a WASM module MUST be bounded by a deadline, a memory cap,
and payload size limits. A trap, timeout, memory exhaustion, oversized
payload, or malformed result MUST fail only that call, MUST NOT end the
session or corrupt canonical session state, and MUST be recorded as a
diagnostic. After repeated consecutive failures Smith SHALL disable the
module for the rest of the session and report it as failed. A failing
pipeline contribution MUST contribute nothing for that phase. A context
contributor SHALL run without the session-history capability, and it then
receives an empty history. A view that Smith itself cannot fit within the
payload limits MUST skip that contribution and MUST NOT count as a failure
of the module.

#### Scenario: Tool call times out

- **GIVEN** a WASM tool that never returns
- **WHEN** the model calls it
- **THEN** the call ends at its deadline with a tool error
- **AND** the turn continues

#### Scenario: Module traps repeatedly

- **GIVEN** a WASM module whose calls trap three times in a row
- **WHEN** the third failure is recorded
- **THEN** the module is disabled for the session and listed as failed
- **AND** its tools are no longer offered from the next safe boundary

#### Scenario: Context contributor fails

- **GIVEN** a WASM context contributor that returns a malformed patch
- **WHEN** the provider request is assembled
- **THEN** the request is assembled without that contribution

#### Scenario: Contributor without history access

- **GIVEN** a WASM context contributor whose module was not granted session
  history
- **WHEN** the provider request is assembled
- **THEN** the contributor runs with an empty history
- **AND** its tools stay available

#### Scenario: History too large to pass

- **GIVEN** a granted WASM context contributor and a session history beyond
  the payload limits
- **WHEN** the provider request is assembled
- **THEN** the request is assembled without that contribution
- **AND** the module is not disabled

## ADDED Requirements

### Requirement: Readable WASM module ids

A WASM module's id SHALL be its plugin name, `/`, and its component path
without the `.wasm` suffix, when that value fits the module id alphabet and
length. Otherwise it SHALL be `wasm.` followed by hashes of the plugin name
and component path. Both forms SHALL be stable across installs. The hashed
form SHALL remain an alias for a module that has a readable id, so install
records, capability grants, and configuration written before this change
keep applying without reinstalling. Listings and diagnostics SHALL show the
readable id.

#### Scenario: Readable id

- **GIVEN** plugin `relay` with component `modules/mailbox.wasm`
- **WHEN** the plugin is installed
- **THEN** the module id is `relay/modules/mailbox`

#### Scenario: Unrepresentable name

- **GIVEN** a plugin or component path with characters outside the module
  id alphabet
- **WHEN** the plugin is installed
- **THEN** the module id is the hashed form

#### Scenario: Install from before the change

- **GIVEN** a module installed and granted capabilities under its hashed id
- **WHEN** Smith starts after upgrading
- **THEN** the module loads with the same grants under its readable id
- **AND** a configuration entry naming the hashed id still applies to it
