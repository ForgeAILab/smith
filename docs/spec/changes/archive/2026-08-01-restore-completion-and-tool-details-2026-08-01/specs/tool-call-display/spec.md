## MODIFIED Requirements

### Requirement: Redaction-safe tool invocation summaries

Smith SHALL derive bounded local display summaries for its built-in tool calls
from explicit tool-specific schemas after applying credential-key and
registered-secret redaction. The summaries MUST identify the ordinary
operation inputs and actionable local target without enabling raw tool
arguments in canonical events, journals, observability, or machine output.

#### Scenario: Read and list calls identify their operation bounds

- **GIVEN** the model requests `read` for `src/lib.rs` with offset `4` and
  limit `20`, and recursive `list` for the project root
- **WHEN** the interactive transcript renders those calls
- **THEN** it identifies `Read(src/lib.rs · offset 4 · limit 20)` and
  `List(. · recursive)`
- **AND** each row states its running or completed status in text

#### Scenario: Search and shell show ordinary operation inputs

- **GIVEN** a built-in search has a pattern, path, extension, case mode, and
  limit, or a shell call has a command, cwd, and timeout
- **WHEN** Smith derives the local interactive summary
- **THEN** those bounded operational values appear in the typed row
- **AND** line, terminal, and bidi controls are normalized out

#### Scenario: Credential material appears in displayed arguments

- **GIVEN** canonical arguments contain credential-shaped keys or exact secret
  literals registered with the session redactor
- **WHEN** Smith prepares the local display projection
- **THEN** API keys, authorization values, tokens, passwords, credentials,
  private keys, secrets, and registered literals render only as `[redacted]`
- **AND** ordinary paths, limits, flags, patterns, commands, and timeouts are
  not replaced by a blanket `values protected` label

#### Scenario: Bulk content remains outside the compact row

- **GIVEN** an edit call contains old/new bodies or a tool has result content
- **WHEN** the compact invocation row renders
- **THEN** it shows the edit target and mode but not the edit or result bodies
- **AND** the omission is a compact-presentation rule rather than a claim that
  every argument value is secret

#### Scenario: Display input attempts terminal injection

- **GIVEN** a displayed argument is oversized or contains line breaks,
  terminal controls, bidi controls, or other control characters
- **WHEN** Smith projects it for display
- **THEN** the result is bounded and normalized to one safe logical line
- **AND** the tool call remains executable according to its independent
  validation and approval policy

### Requirement: Local enrichment without event disclosure

Smith SHALL enrich interactive tool rows from the matching canonical in-process
tool call by stable call ID and SHALL retry that reviewed enrichment at tool
completion when request-time lookup did not produce a display. Display
enrichment MUST NOT change the shared runtime event schema, opt into raw event
arguments, persist the summary in the event journal, or alter tool execution.

#### Scenario: Protected live event has a matching canonical call

- **GIVEN** `ToolCallRequested` contains argument keys and a fingerprint but no
  raw arguments
- **AND** canonical session history contains the matching validated call
- **WHEN** the interactive host handles the event
- **THEN** it supplies only the credential-redacted typed projection to the TUI
- **AND** the canonical event and journal remain argument-value free

#### Scenario: Request-time canonical lookup races visibility

- **GIVEN** request-time lookup cannot yet resolve a known built-in call
- **WHEN** the matching completion event arrives after canonical history is
  visible
- **THEN** Smith retries projection by the same stable call ID
- **AND** the completed row replaces its fallback with the reviewed display

#### Scenario: Canonical call cannot be resolved

- **GIVEN** a protected event has no matching canonical call or reviewed tool
  schema
- **WHEN** Smith renders it after every valid enrichment boundary
- **THEN** the row shows the tool, argument keys, and an honest unavailable or
  unknown-schema label
- **AND** it does not guess values, claim all values are secret, delay, fail,
  or change tool execution

### Requirement: Live and resumed tool-row parity

Smith SHALL render one compact invocation row per tool call and SHALL derive
the same credential-redacted built-in summary from live or resumed canonical
state. Tool-result and bulk edit bodies MUST remain outside the normal
transcript row.

#### Scenario: Resume a tool-assisted session

- **GIVEN** a saved session contains built-in tool calls and matching results
- **WHEN** the user resumes it
- **THEN** each tool row identifies the same operation details shown during the
  completed live session
- **AND** its completed or failed status is reconstructed without revealing a
  result or bulk edit body

#### Scenario: Unknown tool has no reviewed projector

- **GIVEN** canonical history contains a third-party or unknown tool call
- **WHEN** Smith renders it live or after resume
- **THEN** the row shows its name and argument keys with an unknown-schema
  fallback
- **AND** Smith does not guess which argument values are safe
