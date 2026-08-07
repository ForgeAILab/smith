# tool-call-display Specification

## Purpose
TBD - created by archiving change add-redaction-safe-tool-summaries. Update Purpose after archive.
## Requirements
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

Smith SHALL render at most one compact invocation row per tool call and SHALL
derive the same credential-redacted built-in summary from live or resumed
canonical state. Tool-result and bulk edit bodies MUST remain outside the
normal transcript row. Smith MAY suppress the row for a successful call from a
reviewed suppression set whose effect another surface already reports, and MUST
apply that set identically live and after resume. A call that failed, was
denied, or ended unreported MUST always render its row.

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

#### Scenario: A suppressed row is suppressed on resume too

- **GIVEN** a session contains a successful call from the suppression set
- **WHEN** the user resumes that session
- **THEN** the rebuilt transcript omits the same row the live session omitted
- **AND** no other row moves, changes text, or changes status as a result

### Requirement: Reviewed redundant-row suppression

Smith SHALL suppress a transcript tool row only from an explicit reviewed set
of tool calls whose effect a named non-transcript surface already reports, and
only when the call succeeded. The set MUST be enumerated in code rather than
inferred from a call's name, arguments, or result size. Suppression MUST NOT
change tool execution, approval, canonical history, the journal, or machine
output.

#### Scenario: A todo write is reported by the pane instead

- **GIVEN** the model successfully calls `write_todos`
- **WHEN** Smith renders the transcript
- **THEN** the transcript shows no row and no result preview for that call
- **AND** the anchored todo pane reflects the new plan

#### Scenario: A failed suppressed call still reports itself

- **GIVEN** a `write_todos`, `registry.search`, or `agent` call fails, is
  denied, or ends unreported
- **WHEN** Smith renders the transcript
- **THEN** the row renders normally with its failure status
- **AND** suppression is not applied, because a failure is redundant with
  nothing

#### Scenario: Delegation actions the lifecycle already reports

- **GIVEN** the model calls `agent` with `spawn`, `wait`, `result`, `resume`,
  or `stop`, and each call succeeds
- **WHEN** Smith renders the transcript
- **THEN** only the spawn row renders, carrying the reviewed spawn projection
- **AND** the `wait`, `result`, `resume`, and `stop` rows are suppressed in
  favour of the matching child lifecycle line
- **AND** an `agent follow_up` or `agent list` call still renders its row,
  because no lifecycle line reports it

#### Scenario: Suppression does not reach the model or the record

- **GIVEN** any suppressed row
- **WHEN** the canonical history, event journal, and machine output are
  inspected
- **THEN** the call, its arguments, and its result are present unchanged
- **AND** only the local transcript presentation omitted the row

### Requirement: Reviewed delegation invocation summaries

Smith SHALL project `agent` tool calls through a reviewed display schema that
names the delegation action, the addressed child where the action addresses
one, the child's tool scope and workspace posture where the action declares
them, and a bounded excerpt of the task text. It MUST NOT display a profile the
call did not select, and MUST label an inherited profile as inherited.

#### Scenario: A spawn names its task, scope, and workspace

- **GIVEN** the model spawns a child with a task, `tools` of `read_only`, and a
  shared workspace
- **WHEN** the interactive transcript renders the call
- **THEN** the row identifies the spawn action, a bounded one-line excerpt of
  the task text, the `read only` tool scope, and the `shared` workspace
- **AND** line, terminal, and bidi controls are normalized out of the excerpt

#### Scenario: A spawn row adopts its child's identity

- **GIVEN** a spawn row has rendered and the runtime then reports the child
  spawned
- **WHEN** Smith correlates the lifecycle event to the originating call id
- **THEN** the same row also names the child id, its workspace posture, and its
  turn ceiling where one is declared
- **AND** Smith adds no second row for the same spawn

#### Scenario: An addressed action names its child

- **GIVEN** the model calls `agent` with `follow_up` for a known child
- **WHEN** Smith renders the row
- **THEN** it identifies the action and that child id
- **AND** a bounded excerpt of the follow-up task text

#### Scenario: An unbounded child claims no turn ceiling

- **GIVEN** a spawned child has no declared maximum turns
- **WHEN** the row reports its terms
- **THEN** it omits the turn ceiling
- **AND** does not render the unlimited sentinel as a number

#### Scenario: The tool selects no profile

- **GIVEN** the `agent` tool has no profile argument and the child inherits the
  parent's agent profile
- **WHEN** Smith renders the spawn row
- **THEN** it names that profile as inherited
- **AND** does not imply the call chose it
