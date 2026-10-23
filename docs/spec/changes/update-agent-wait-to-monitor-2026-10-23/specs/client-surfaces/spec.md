## ADDED Requirements

### Requirement: Wait-timeout continuations are visible

Interactive and headless lifecycle output SHALL attribute a turn admitted by
an expired wait timer to `delegation.wait-timeout`, distinct from
`delegation.child-completion` and from user turns. The projection MUST NOT
imply that the child stopped or finished when the timer fired.
The delegated-work panel SHALL show a `monitor` tag and wake deadline for each
armed child. The tag SHALL clear when the wake is admitted or cancelled and
SHALL NOT be restored from historical tool results after restart.

#### Scenario: Timer wakes the parent

- **GIVEN** a `delegation.wait-timeout` turn is admitted
- **WHEN** lifecycle and usage output are inspected
- **THEN** the turn is attributed to `delegation.wait-timeout`
- **AND** the child is still shown as running

#### Scenario: Wait call no longer looks stalled

- **GIVEN** the parent calls `agent.wait` on a running child
- **WHEN** clients render the tool call
- **THEN** the call completes immediately with the armed wake time
- **AND** the session shows the parked state rather than an active tool call

#### Scenario: Armed child is visible in the panel

- **GIVEN** a running child has an armed wait timer
- **WHEN** the delegated-work panel refreshes
- **THEN** its row shows a `monitor` tag and the wake deadline
- **AND** the tag clears after timer admission or cancellation
