## MODIFIED Requirements

### Requirement: Safe parent reporting

Child progress and results SHALL appear immediately in the active host and
enter the parent session inbox. They MUST be introduced to the parent model only
at a safe provider/tool boundary, and a final child result MUST not be silently
dropped during progress coalescing.

In the interactive TUI, "appear immediately" is satisfied by the delegated-work
panel and the child's retained log; the root transcript SHALL carry only
delegation's boundaries — spawn, exact-checkpoint resume start, needs-input,
interruption, completion, stop, and failure. Mid-flight progress MUST NOT be
discarded: every lifecycle event, printed or not, MUST append to that child's
bounded inspectable log.

#### Scenario: Child finishes during parent streaming

- **GIVEN** the parent model is streaming when a child completes
- **WHEN** the final child result arrives
- **THEN** the TUI marks the child complete immediately
- **AND** the result is queued for the parent's next safe continuation
- **AND** the active parent stream is not interrupted

#### Scenario: Child runs a tool while the root conversation is open

- **GIVEN** a running child calls a tool
- **WHEN** the TUI folds the progress event
- **THEN** the child's panel row shows the new activity
- **AND** the event appends to that child's inspectable log
- **AND** no notice is written to the root transcript

#### Scenario: Child reaches a terminal state

- **GIVEN** a child completes, stops, is interrupted, or fails
- **WHEN** the TUI folds the terminal event
- **THEN** an attributed notice enters the root transcript
- **AND** the same event appends to that child's inspectable log

### Requirement: Child timeline navigation

Smith SHALL expose current and completed direct children through `/agent`,
`/timeline`, and keyboard selection of the delegated-work panel, with stable
previous, next, and parent navigation. Child inspection MUST be a temporary
read-only view and MUST NOT move persistent input focus away from the root
composer. Turn, token, session, and workspace figures shown for a child MUST
come from the delegation coordinator, never from client-side estimation.

#### Scenario: Inspect consecutive child results
- **GIVEN** two attributed children completed
- **WHEN** the user opens one and requests next
- **THEN** Smith displays the next child's bounded lifecycle and result
- **AND** returning to parent restores the same composer draft and scroll state

#### Scenario: Walk the delegated-work panel from the keyboard
- **GIVEN** at least one direct child exists and composer history has nowhere
  left to go
- **WHEN** the user presses the panel's next-agent key
- **THEN** Smith selects the next child in the panel's own order
- **AND** replaces the transcript region with that child's read-only view
- **AND** leaves the composer focused and its draft unchanged

#### Scenario: Continue the inspected child from the composer
- **GIVEN** a child that accepts a follow-up is being inspected
- **WHEN** the user submits an ordinary prompt
- **THEN** Smith confirms a follow-up turn addressed to that child
- **AND** a local command submitted in the same view still addresses the root

#### Scenario: No child is selected
- **GIVEN** no direct child exists
- **WHEN** the user opens child navigation
- **THEN** Smith reports the empty state locally with available preset hints
- **AND** does not fabricate a session or provider request
