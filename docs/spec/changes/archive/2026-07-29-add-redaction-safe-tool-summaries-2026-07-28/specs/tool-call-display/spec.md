## ADDED Requirements

### Requirement: Redaction-safe tool invocation summaries

Smith SHALL derive bounded local display summaries for its built-in tool calls
from explicit tool-specific field allowlists. The summaries MUST identify the
tool and actionable local target without enabling raw tool arguments in
canonical events, journals, observability, or machine output.

#### Scenario: Read and list calls identify their targets

- **GIVEN** the model requests `read` for `src/lib.rs` and recursive `list` for
  the project root
- **WHEN** the interactive transcript renders those calls
- **THEN** it identifies `Read(src/lib.rs)` and `List(. · recursive)`
- **AND** each row states its running or completed status in text

#### Scenario: Arbitrary argument content remains protected

- **GIVEN** built-in calls contain edit bodies, a shell command, a search
  pattern, tool-result content, or an unrelated unknown field
- **WHEN** Smith derives their display summaries
- **THEN** none of those arbitrary values appears in the summary
- **AND** only allowlisted target and numeric/boolean qualifier fields may be
  shown

#### Scenario: Display input attempts terminal injection

- **GIVEN** an allowlisted target is oversized or contains line breaks,
  terminal controls, or other control characters
- **WHEN** Smith projects it for display
- **THEN** the result is bounded and normalized to one safe logical line
- **AND** the tool call remains executable according to its independent
  validation and approval policy

### Requirement: Local enrichment without event disclosure

Smith SHALL enrich interactive tool rows from the matching canonical in-process
tool call by stable call ID. Display enrichment MUST NOT change the shared
runtime event schema, opt into raw event arguments, persist the summary in the
event journal, or alter tool execution.

#### Scenario: Protected live event has a matching canonical call

- **GIVEN** `ToolCallRequested` contains argument keys and a fingerprint but no
  raw arguments
- **AND** canonical session history contains the matching validated call
- **WHEN** the interactive host handles the event
- **THEN** it supplies only the safe typed projection to the TUI
- **AND** the canonical event and journal remain argument-value free

#### Scenario: Canonical call cannot be resolved

- **GIVEN** a protected request event has no matching call available to the
  interactive host
- **WHEN** Smith renders it
- **THEN** the row falls back to the tool name and protected argument keys
- **AND** display enrichment does not delay, fail, or change tool execution

### Requirement: Live and resumed tool-row parity

Smith SHALL render one compact invocation row per tool call and SHALL derive
the same safe built-in summary from live or resumed canonical state. Tool-result
bodies MUST remain outside the normal transcript row.

#### Scenario: Resume a tool-assisted session

- **GIVEN** a saved session contains built-in tool calls and matching results
- **WHEN** the user resumes it
- **THEN** each tool row identifies the same safe target shown during the live
  session
- **AND** its completed or failed status is reconstructed without revealing the
  result body

#### Scenario: Unknown tool has no reviewed projector

- **GIVEN** canonical history contains a third-party or unknown tool call
- **WHEN** Smith renders it live or after resume
- **THEN** the row shows the tool name with a protected-key fallback
- **AND** Smith does not guess which argument values are safe
