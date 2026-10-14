## ADDED Requirements

### Requirement: Delegation from a harness session

A root session running on a harness SHALL be able to use the `agent` tool
through the bridge with the actions, profile selection, tool scope,
workspace rules, wait policy, and one-level hierarchy it has in a native
session. A child MAY run on any child-enabled profile, whether that profile
uses a provider or a harness.

A terminal child outcome the CLI's model read through `wait` or `result`
SHALL be acknowledged against that bridged tool call, so it is not delivered
again. An outcome it did not read SHALL be delivered by a child-completion
turn that runs on the harness.

#### Scenario: A harness parent spawns a provider child

- **GIVEN** a root session on `cli/claude-code/opus` with delegation enabled
- **WHEN** the CLI calls `agent` with action `spawn` and a provider profile
- **THEN** the child starts on that provider under the existing child rules
- **AND** the spawn is approved or refused as a native spawn would be

#### Scenario: The CLI waits for its child

- **GIVEN** the CLI spawned a child and read its result with `wait`
- **WHEN** the parent turn ends
- **THEN** no child-completion turn starts for that result

#### Scenario: The CLI does not wait

- **GIVEN** the CLI spawned a child and ended its turn
- **WHEN** the child completes
- **THEN** one child-completion turn delivers the result to the CLI's
  resumed session

#### Scenario: Delegation disabled on the profile

- **GIVEN** a harness session whose main profile disables delegation
- **WHEN** a turn runs
- **THEN** `agent` is not bridged and no delegation guidance is delivered

#### Scenario: A harness child cannot delegate

- **GIVEN** a child running on a harness
- **WHEN** the CLI lists the bridged tools
- **THEN** `agent` is absent
