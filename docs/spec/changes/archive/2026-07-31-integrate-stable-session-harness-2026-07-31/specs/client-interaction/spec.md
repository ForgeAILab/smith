## ADDED Requirements

### Requirement: Attempt-aware transcript reduction

Smith clients SHALL buffer provider text and reasoning by request and attempt
until Agent Runtime commits or discards that attempt. Live reduction and
journal replay MUST produce an equivalent committed transcript.

#### Scenario: First attempt streams then retries
- **GIVEN** a failed attempt emitted partial visible text
- **WHEN** the runtime discards it and a later attempt commits
- **THEN** the final transcript contains only the committed attempt
- **AND** status may still report usage and a bounded retry diagnostic for the
  failed attempt

### Requirement: Interrupt affects only the active turn

The interactive interrupt action SHALL cancel the current turn and enter a
visible interrupting state without terminally cancelling the session. Session
cancellation remains reserved for confirmed shutdown or revocation.

#### Scenario: Prompt follows an interruption
- **GIVEN** the user interrupted a streaming turn
- **WHEN** its cancelled terminal event arrives and the user submits again
- **THEN** the later turn executes normally on the same session
- **AND** the composer does not inherit a cancelled root token

### Requirement: Prepared approval is exact and queued

The TUI SHALL display the immutable prepared action, exact canonical target,
material arguments, permissions, broad-authority warning, and deadline before
approval. Multiple pending actions MUST use deterministic batching or queuing
and MUST NOT silently supersede each other.

#### Scenario: Parallel calls require approval
- **GIVEN** several prepared calls await decisions
- **WHEN** Smith presents them
- **THEN** every call receives one explicit decision or terminal cancellation
- **AND** no prompt is dropped merely because another prompt arrived

### Requirement: Questionnaire has a distinct interaction surface

The interactive TUI SHALL present agent-originated questionnaires as a
temporary accessible overlay supporting bounded choices, optional free-form
answers, explicit submit/decline, cancellation, deadline, and restored pending
state. Its responder MUST be separate from security approval.

#### Scenario: User selects a design option
- **GIVEN** the active turn asks a structured design question
- **WHEN** the user selects and submits one option
- **THEN** Smith returns the typed answer to the same turn
- **AND** the answer grants no tool authority

### Requirement: Non-interactive interaction fails predictably

An ordinary headless Smith run MUST omit questionnaire readiness or return a
versioned `interaction_required` non-success outcome when no bidirectional
interaction protocol is configured. It MUST NOT wait indefinitely or treat
prompt stdin as an asynchronous answer.

#### Scenario: Headless model requests clarification
- **GIVEN** no interactive broker is configured
- **WHEN** a forced or resumed questionnaire reaches the host
- **THEN** Smith terminates with a structured interaction-required result
- **AND** no answer is fabricated

### Requirement: Child questions route through the parent by default

Smith MUST keep direct questionnaire readiness root-only unless an explicit
agent profile grants attributed child interaction. A child without readiness
SHALL return a structured needs-input result through the parent safe-boundary
path.

#### Scenario: Child encounters ambiguity
- **GIVEN** a child needs a material user choice
- **AND** its profile has no direct interaction readiness
- **WHEN** it returns needs-input
- **THEN** the parent receives the attributed request
- **AND** no competing child overlay opens automatically
