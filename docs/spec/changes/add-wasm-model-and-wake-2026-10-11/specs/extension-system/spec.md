## ADDED Requirements

### Requirement: Sandboxed modules can call the session's model

Smith SHALL offer a sandboxed module a host function that performs one
bounded, non-streaming, tool-free request on the session's current model
route, including an installed agent when selected, only when the module's manifest requested the model capability
and the user granted it. The module MUST NOT receive a credential, choose a
provider, or exceed the per-call output limit and the per-turn call limit.
The function MUST be available only while the module is handling a tool
call. A refused request MUST return a typed error to the module and MUST NOT
count towards disabling it.

Smith SHALL use the shared one-shot executor. Provider routes SHALL receive
the requested output-token ceiling; installed-agent routes use the CLI's own
generation limits and SHALL receive the role-labelled conversation rendered
as one user input. Both routes remain subject to the host deadline, call-count
and ABI payload limits. A truncated successful answer SHALL return finish
reason `length`; filtered, empty, tool-calling or unfinished answers SHALL
return a recoverable error. Installed-agent usage SHALL remain unpriced.

#### Scenario: Granted module asks the model

- **GIVEN** a WASM module granted the model capability
- **WHEN** its tool calls the model function with a prompt
- **THEN** the request is sent on the session's current provider and model
- **AND** the module receives the response text and usage
- **AND** no credential is visible to the module

#### Scenario: Ungranted module asks the model

- **GIVEN** a WASM module without the model capability
- **WHEN** its tool calls the model function
- **THEN** the function returns an error naming the missing capability
- **AND** no provider request is made

#### Scenario: Per-turn limit reached

- **GIVEN** a WASM module that has made the permitted number of model calls
  in the current turn
- **WHEN** it calls the model function again
- **THEN** the call is refused with a limit error
- **AND** the module is not disabled

#### Scenario: Model call outside a tool call

- **GIVEN** a WASM module granted the model capability
- **WHEN** its context contributor calls the model function
- **THEN** the call is refused

### Requirement: Sandboxed modules can notify a session

Smith SHALL let a sandboxed module contribute a watcher that Smith polls at
a host-owned bounded interval while a root session is live, only when the
module was granted the session-notify capability. Notices returned by a
watcher SHALL be delivered through the session inbox under the same rules as
monitor notifications: shown to the user without a prompt, supplied to the
model at the next safe boundary, and never interrupting a stream or tool
result. Notices MUST be bounded in count and size, MUST be labelled with the
plugin that produced them, and MUST be presented to the model as untrusted
text.

#### Scenario: Notice reaches an idle session

- **GIVEN** an idle interactive session with a granted watcher
- **WHEN** a poll returns one notice
- **THEN** the notice is shown with its plugin name
- **AND** the session takes a turn with the notice supplied to the model

#### Scenario: Notice arrives during a stream

- **GIVEN** the agent is streaming a response
- **WHEN** a poll returns a notice
- **THEN** the stream is not interrupted
- **AND** the notice is supplied at the next safe boundary

#### Scenario: Watcher without the capability

- **GIVEN** a WASM module that contributes a watcher without the
  session-notify capability
- **WHEN** the module is mounted
- **THEN** the watcher is refused with a reason
- **AND** the module's other contributions remain available

#### Scenario: Headless run ends

- **GIVEN** a headless run with a granted watcher
- **WHEN** its turn completes
- **THEN** the process exits without waiting for further polls

### Requirement: Interface versions coexist

Smith SHALL load modules built against interface 0.1 and interface 0.2. A
0.1 module MUST behave as it did before and MUST NOT be able to reach
functions introduced in 0.2.

#### Scenario: Older module still loads

- **GIVEN** an installed module built against interface 0.1
- **WHEN** a Smith that supports 0.2 starts a session
- **THEN** the module mounts and its tools work as before
