## ADDED Requirements

### Requirement: Stable baseline categories in context visualization

`/context` SHALL always name system instructions and tool schemas in a stable
order without revealing their content. Before the first enforced plan their
counts MUST be unknown rather than zero; after a plan their display totals MUST
be derived from canonical segment totals and MUST remain visible when zero.

#### Scenario: Context before the first plan names stable request classes

- **GIVEN** no turn has emitted a `ContextPlanned` event
- **WHEN** the user submits `/context`
- **THEN** Smith lists system instructions and tool schemas as not counted yet
- **AND** their counts render as unknown rather than zero
- **AND** no provider request is issued

#### Scenario: Planned context aggregates instruction segments for display

- **GIVEN** a context plan contains system, developer, and ability instruction
  segment totals
- **WHEN** the user submits `/context`
- **THEN** the focused view shows their sum as `system instructions`
- **AND** canonical status and telemetry retain each original segment kind

#### Scenario: Baseline category has an honest zero

- **GIVEN** a context plan has no tool-schema segment
- **WHEN** the user submits `/context`
- **THEN** the tool-schema legend row remains visible with a zero count
- **AND** the usage grid allocates no nonzero cells to that category

### Requirement: Local reasoning controls

Smith SHALL expose idle-only `/think` and `/effort` controls using the shared
command and picker grammar. Choices MUST be limited to the resolved
provider/model capability snapshot, MUST apply to the next whole turn, and
MUST NOT issue a provider request merely to inspect or change a setting.

#### Scenario: Toggleable model changes thinking for the next turn

- **GIVEN** the idle provider/model supports optional thinking
- **WHEN** the user selects `/think off`
- **THEN** Smith records a session override and confirms it locally
- **AND** the next complete turn uses the disabled setting
- **AND** no request is issued by the command itself

#### Scenario: Effort selector contains only supported levels

- **GIVEN** the resolved provider/model advertises `low`, `medium`, and `high`
- **WHEN** the user opens `/effort`
- **THEN** the picker contains only those efforts plus the provider default
- **AND** selecting one uses the same validation as a direct command argument

#### Scenario: Fixed reasoning exposes no false control

- **GIVEN** the model reasons but its controls are fixed or unknown
- **WHEN** the user opens `/think` or `/effort`
- **THEN** Smith explains locally which control is unavailable and why
- **AND** it does not infer support, send a probe, or mutate the session

#### Scenario: Mandatory reasoning cannot be disabled

- **GIVEN** the capability snapshot marks reasoning mandatory
- **WHEN** the user opens `/think` or submits `/think off`
- **THEN** the UI omits or disables the off choice with a written reason
- **AND** the direct command fails locally before provider I/O

### Requirement: Reasoning status and lifecycle visibility

Smith SHALL show the effective thinking state, effort when applicable, and
configuration/provider/session provenance in local status and context output.
Session overrides MUST survive compatible resume, MUST be revalidated on a
provider/model change, and MUST never alter an already-running turn.

#### Scenario: Status distinguishes default from override

- **GIVEN** a session effort overrides the provider/model default
- **WHEN** the user submits `/status` or `/context`
- **THEN** Smith shows the effective effort and labels it a session override
- **AND** raw reasoning content is not shown

#### Scenario: Model switch invalidates an override

- **GIVEN** the session has an effort unsupported by a newly selected model
- **WHEN** Smith switches and rebuilds the provider/model runtime
- **THEN** it clears the incompatible override with an explicit local notice
- **AND** it does not map the value to a guessed nearest effort

#### Scenario: Busy turn cannot change reasoning mid-loop

- **GIVEN** a turn is running or waiting on a tool continuation
- **WHEN** the user attempts to change thinking or effort
- **THEN** Smith refuses the command locally as busy
- **AND** every request in the active turn retains its original setting
