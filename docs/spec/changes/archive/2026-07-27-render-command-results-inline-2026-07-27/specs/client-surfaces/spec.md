## ADDED Requirements

### Requirement: Inline informational command results

Smith SHALL render read-only local command results as attributed blocks in the
normal transcript instead of opening a blocking viewer. These blocks MUST keep
composer input available, participate in normal transcript scrolling and
follow behavior, remain bounded, and MUST NOT be sent to the provider or added
to canonical model conversation history.

#### Scenario: Status appears in the conversation

- **GIVEN** the interactive composer is available
- **WHEN** the user invokes `/status`
- **THEN** a titled status block is appended to the transcript
- **AND** the composer remains immediately available without a close step
- **AND** no provider request is issued

#### Scenario: Consecutive local results remain visible

- **GIVEN** one informational command result is already in the transcript
- **WHEN** the user invokes another informational command
- **THEN** Smith appends the new titled result after the earlier result
- **AND** does not replace, cover, or dismiss the earlier result

#### Scenario: Diff states render inline

- **WHEN** `/diff` produces a patch, empty result, non-Git outcome, binary
  notice, oversized notice, or conflict
- **THEN** Smith renders the bounded result inline with its state stated in
  text
- **AND** normal transcript scrolling remains available

#### Scenario: Interactive safety surfaces remain modal

- **WHEN** Smith needs command selection, tool approval, provider-spend
  confirmation, undo confirmation, or revert confirmation
- **THEN** Smith may open the corresponding modal with explicit controls
- **AND** informational command results themselves never require dismissal

#### Scenario: Local results do not become model context

- **GIVEN** an informational result is visible in the transcript
- **WHEN** the user sends the next provider prompt or resumes the session
- **THEN** Smith does not represent that local result as a user or assistant
  conversation message
- **AND** protected local status or patch detail is not exposed to the
  provider merely because it was displayed
