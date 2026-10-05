## ADDED Requirements

### Requirement: A child result the model already read is not delivered again

When the parent model receives a child's terminal outcome through the agent
tool's `wait` or `result` action, Smith SHALL acknowledge that outcome to
Agent Runtime against the tool call, so that no `delegation.child-completion`
continuation delivers it again once the tool result commits. A child outcome
the model has not received SHALL still be delivered automatically. A headless
run MUST end after its root turn when no child runs, no outcome is ready, and
no admitted delivery turn is still to start.

#### Scenario: The model waits for its child

- **GIVEN** the parent model spawns a child and reads its result with `wait`
- **WHEN** the parent turn ends
- **THEN** no child-completion continuation starts for that result
- **AND** the result stays readable with `result`

#### Scenario: The model does not wait

- **GIVEN** the parent model spawns a child and ends its turn
- **WHEN** the child completes
- **THEN** one child-completion continuation delivers the result

#### Scenario: Headless run that read the result

- **GIVEN** a headless run whose model read its child's result in the root turn
- **WHEN** the root turn completes
- **THEN** the run ends without waiting for a delivery turn
