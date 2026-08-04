## ADDED Requirements

### Requirement: Busy ordinary input has explicit steer and queue intent

Smith SHALL give ordinary user input explicit steer and queue intent while
eligible provider-backed work is serving and no overlay owns input. `Enter` on
a valid ordinary prompt targets the active turn, while `Tab` on a non-empty
ordinary prompt explicitly queues a future turn. Local commands, shell
shortcuts, child-agent actions, approvals, and questionnaires MUST retain their
distinct validation and input ownership.

#### Scenario: Enter steers serving work

- **GIVEN** an eligible provider-backed turn is serving
- **AND** the composer contains a valid ordinary user prompt
- **WHEN** the user presses `Enter`
- **THEN** Smith targets the serving runtime turn with that input
- **AND** does not submit a separate whole turn merely to stage it

#### Scenario: Tab queues a later turn

- **GIVEN** a turn is serving and the composer contains a valid ordinary user
  prompt
- **WHEN** the user presses `Tab` outside an overlay
- **THEN** Smith stores that prompt as a bounded process-local future turn
- **AND** does not send it to Agent Runtime until an eligible terminal boundary

#### Scenario: Existing input owner keeps precedence

- **GIVEN** a palette, picker, approval, questionnaire, or confirmation owns
  input
- **WHEN** the user presses `Enter` or `Tab`
- **THEN** that surface retains its documented behavior
- **AND** Smith neither steers nor queues the composer draft accidentally

### Requirement: Pending input remains ordered and editable where safe

Smith SHALL keep accepted-but-uncommitted steers, rejected-steer follow-ups,
and explicit future turns in separate bounded FIFO state. The user MAY restore
the newest explicit future turn for editing, but MUST NOT edit an input already
accepted by the active runtime turn.

#### Scenario: User edits the newest queued turn

- **GIVEN** two explicit future turns are queued and no modal owns input
- **WHEN** the user invokes the queued-input edit shortcut
- **THEN** Smith removes the newest queued entry and restores it exactly to the
  composer
- **AND** preserves the older entry in FIFO order

#### Scenario: Rejected steer precedes an ordinary queue

- **GIVEN** a steer is rejected because the serving work is not steerable
- **AND** an ordinary future turn is already queued
- **WHEN** the serving work completes successfully
- **THEN** Smith dispatches the rejected steer follow-up first
- **AND** retains the ordinary queued turn for a later boundary

### Requirement: Terminal handling is lossless and exactly once

Smith SHALL remove pending steer text from process-local state only after the
runtime reports its committed or discarded disposition. A successful terminal
boundary SHALL start at most one queued follow-up, while interruption and other
non-success outcomes MUST preserve uncommitted input without duplication.

#### Scenario: Steer commits within the active turn

- **GIVEN** Smith displays an accepted pending steer
- **WHEN** Agent Runtime reports that steer committed at a safe boundary
- **THEN** Smith appends its user transcript row at that boundary exactly once
- **AND** removes only the matching pending preview

#### Scenario: Interrupt sends pending steers immediately

- **GIVEN** one or more steers are accepted but uncommitted
- **WHEN** the user invokes the documented interrupt-for-steer action
- **THEN** Smith interrupts the serving turn
- **AND** after cancellation merges and submits the still-uncommitted steers as
  one ordinary turn in FIFO order
- **AND** never resubmits a steer already reported committed

#### Scenario: Turn fails with pending input

- **GIVEN** pending or queued user input exists
- **WHEN** the serving turn fails, reaches a limit, or returns needs-input
- **THEN** Smith restores the uncommitted material for explicit user review
- **AND** performs no automatic provider spend for that material
