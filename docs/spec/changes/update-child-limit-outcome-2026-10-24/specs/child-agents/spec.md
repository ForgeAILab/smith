## ADDED Requirements

### Requirement: A child task stopped at a limit is reported as that limit

The system SHALL tell the parent and the user that a child's task stopped
at a limit when a configured limit (output, tool steps, time, or provider
attempts) ended it. The system MUST NOT report it as a
completed task or present anything the child said during that task as its
result. The child SHALL remain available for a follow-up within its limits,
and artifacts the stopped task returned SHALL still reach the parent.

#### Scenario: Output limit discards the child's final response

- **GIVEN** a child's task ends at its output limit after an earlier preamble
- **WHEN** the outcome is delivered to an idle parent
- **THEN** the delivery says the child stopped at its output limit and gave no
  final answer
- **AND** neither the preamble nor the discarded response appears as a result

#### Scenario: Parent reads the outcome with the agent tool

- **GIVEN** a child's task stopped at a limit
- **WHEN** the parent calls `agent` `wait` or `result` for it
- **THEN** the result carries `stopped_at_limit` with the limit's name and a
  null `result`
- **AND** it says the child can continue through `follow_up`
- **AND** no automatic delivery repeats the outcome

#### Scenario: TUI shows a limit-stopped child

- **GIVEN** the TUI shows a running child
- **WHEN** the child's task stops at a limit
- **THEN** the child row leaves the running state and names the limit
- **AND** the root transcript carries an attributed notice
- **AND** the row is not dismissed as a success

#### Scenario: Sessions recorded before the limit event resume

- **GIVEN** a session journal written with event schema 16
- **WHEN** the session resumes on a build that emits schema 17
- **THEN** the journal loads and resume proceeds
