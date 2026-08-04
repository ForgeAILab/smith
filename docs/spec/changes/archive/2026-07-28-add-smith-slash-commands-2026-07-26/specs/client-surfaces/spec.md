## ADDED Requirements

### Requirement: Slash-command interception

Composer input whose first non-whitespace character is `/` SHALL be
intercepted and dispatched as a local command. Intercepted input MUST NOT be
sent to the provider, and an unknown command MUST produce a local error that
points at command discovery, with no provider request or spend.

#### Scenario: Known command dispatches locally

- **GIVEN** the composer contains a registered command such as `/model`
- **WHEN** the user submits it
- **THEN** Smith runs the mapped host action
- **AND** no provider request is issued

#### Scenario: Unknown command fails locally

- **GIVEN** the composer contains an unregistered command
- **WHEN** the user submits it
- **THEN** Smith renders a local error referencing `/help`
- **AND** no provider request is issued

### Requirement: Command discovery and host-action mapping

Smith SHALL provide `/help` listing every registered command with a one-line
description. Built-in commands MUST map to existing host actions (for
example, the model picker and session controls) rather than duplicating their
logic, so a command and its keybinding behave identically.

#### Scenario: Help lists registered commands

- **GIVEN** the user submits `/help`
- **WHEN** Smith renders the response locally
- **THEN** every registered command appears with a one-line description

#### Scenario: Command matches its keybinding

- **GIVEN** a host action is reachable by both a keybinding and a command
- **WHEN** the user invokes the command
- **THEN** the same host action runs with the same behavior as the keybinding

### Requirement: Literal slash passthrough

Smith SHALL provide a documented escape that sends a message beginning with a
slash to the model as an ordinary prompt.

#### Scenario: Escaped slash message reaches the model

- **GIVEN** the user applies the documented escape to input starting with `/`
- **WHEN** they submit it
- **THEN** the message is sent to the provider verbatim as a prompt
- **AND** no local command is dispatched

### Requirement: Context visibility in local status

Smith SHALL render the latest enforced context plan inside `/status`, including
percent left, counted input tokens, input budget, model window, reserved
tokens, count provenance, and bounded totals by segment kind. The display MUST
distinguish the latest request plan from cumulative provider-reported session
input and MUST name the absence of a plan before the first turn. Context
inspection MUST remain local and MUST NOT issue a provider request.

#### Scenario: Status shows the latest enforced plan

- **GIVEN** the runtime emitted a `ContextPlanned` event
- **WHEN** the user submits `/status`
- **THEN** Smith shows the latest plan's used tokens, budget, percent left,
  model window, reserves, confidence, and segment totals
- **AND** cumulative provider input is labelled as session usage rather than
  active context
- **AND** no provider request is issued

#### Scenario: Status before the first context plan

- **GIVEN** no turn has produced a `ContextPlanned` event
- **WHEN** the user submits `/status`
- **THEN** Smith states that context has not been planned yet
- **AND** it shows declared capacity and reserves without inventing usage
- **AND** no provider request is issued

### Requirement: Focused context visualization

Smith SHALL provide `/context` as a local inline visualization of the latest
enforced context plan. It SHALL show model and input-budget capacity, percent
left, bounded totals by segment category, free input space, reserved
output/reasoning capacity, count provenance, and compaction state. The
visualization MUST remain legible without color, MUST NOT retain or reveal raw
context content, and MUST NOT issue a provider request.

#### Scenario: Context command visualizes the latest enforced plan

- **GIVEN** the runtime emitted a `ContextPlanned` event
- **WHEN** the user submits `/context`
- **THEN** Smith appends an inline usage map and category legend for that plan
- **AND** the legend distinguishes used segments, free input space, and reserve
- **AND** exact or estimated provenance and compaction state are stated in words
- **AND** no provider request is issued

#### Scenario: Context command before the first plan

- **GIVEN** no turn has produced a `ContextPlanned` event
- **WHEN** the user submits `/context`
- **THEN** Smith states that usage is unavailable until the first turn
- **AND** it visualizes declared input capacity and reserves without inventing
  segment usage
- **AND** no provider request is issued
