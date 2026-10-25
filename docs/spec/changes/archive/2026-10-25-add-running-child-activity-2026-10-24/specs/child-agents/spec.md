## ADDED Requirements

### Requirement: Running-child status reports bounded current-task activity

Smith SHALL report an `activity` object for each running child in `agent
wait`, `agent list`, and the `delegation.wait-timeout` continuation. It SHALL
include the elapsed time of the current task when known, the number of
committed steps since the task's user message, the last committed step, tools
called but not yet answered, and the attempt number, age, time since last
output, and character counts of a response still streaming. Committed text
SHALL be redacted with the transcript's credential policy and bounded. Content
of a response still streaming MUST NOT be included. Children that are not
running SHALL carry no `activity`.

#### Scenario: A parent checks a child that is streaming after a tool step

- **GIVEN** a running child has committed a tool call and its result
- **AND** its next response is streaming
- **WHEN** the parent calls `agent wait` with timeout 0 or `agent list`
- **THEN** the child's status reports two committed steps, the tool result as
  the last step, no pending tools, and the streaming response's size and age
- **AND** none of the streaming text appears in the status

#### Scenario: A wait deadline expires while a child is still working

- **GIVEN** a parent armed a wait timer for a running child
- **WHEN** the timer expires and the wait-timeout continuation is admitted
- **THEN** the continuation reports the child's activity beside its state
- **AND** it points to `agent transcript` for the committed steps
- **AND** it includes no streaming or final child output

#### Scenario: A finished child reports its result, not activity

- **GIVEN** a child has finished its task and is idle
- **WHEN** the parent calls `agent wait` or `agent list`
- **THEN** the status carries the result and no `activity` object
- **AND** a child idle after stopping at a limit reports that limit and no
  `activity`
