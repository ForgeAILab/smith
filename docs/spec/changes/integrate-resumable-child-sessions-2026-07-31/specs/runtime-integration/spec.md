## ADDED Requirements

### Requirement: One Smith factory composes durable child recovery

Smith SHALL compose Agent Runtime's child catalog, child session/checkpoint
stores, lifecycle leases, policy revisions, and recovery operation through the
same `smith-runtime::factory` path used by root, TUI, headless, test, and
embedded sessions. It MUST NOT add a Smith-local child loop, reconstruct exact
state from journals, or bypass runtime authorization/checkpoint semantics.

#### Scenario: Rebuild a recovered read-only child

- **GIVEN** a durable read-only child is idle after parent restart
- **WHEN** a follow-up requires its runtime to be lazily reconstructed
- **THEN** the one factory composes the original compatible provider/model,
  protected stores, workspace, and narrowed read-only ability view
- **AND** the child still receives no delegation-management ability

#### Scenario: Current policy would widen authority

- **GIVEN** a recovered child record declares a narrower tool/workspace policy
  than current defaults
- **WHEN** Smith rebuilds it
- **THEN** the factory retains the recorded upper bound or fails closed
- **AND** defaults do not silently grant edit, shell, network, interaction, or
  child-management authority
