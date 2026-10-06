## MODIFIED Requirements

### Requirement: Reviewed redundant-row suppression

Smith SHALL suppress a transcript tool row only from an explicit reviewed set
of tool calls whose effect a named non-transcript surface already reports, and
only when the call succeeded. A `write_todos` or `registry.search` call SHALL
also draw no row while it is running, so a row is never shown only to be
removed on success. The set MUST be enumerated in code rather than
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

#### Scenario: A capability search does not flash

- **GIVEN** the model calls `registry.search` and the call is still running
- **WHEN** Smith renders the transcript
- **THEN** no row is drawn for that call
- **AND** the row appears with its status if the call then fails, is denied,
  or ends unreported
