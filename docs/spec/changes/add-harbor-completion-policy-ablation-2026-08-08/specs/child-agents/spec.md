## ADDED Requirements

### Requirement: Profile-controlled root delegation availability

Smith SHALL allow an effective main-agent profile to narrow root delegation
availability. When delegation is disabled, the runtime MUST omit both the
delegation instruction fragment and the `agent` tool from the sealed
capability registry; child surfaces MUST remain non-delegating regardless of
profile configuration.

#### Scenario: Disable delegation for a main profile

- **GIVEN** the effective main profile has delegation disabled
- **WHEN** Smith composes the runtime and prompt
- **THEN** no delegation prompt fragment is contributed
- **AND** no `agent` tool schema or callable delegation route is registered
- **AND** every unrelated eligible tool remains unchanged

#### Scenario: Preserve existing main-profile behavior

- **GIVEN** an existing main profile omits delegation availability
- **WHEN** Smith composes the runtime and prompt
- **THEN** root delegation remains enabled under the existing host policy
- **AND** the prompt and tool registry continue to agree

#### Scenario: Child profile attempts to enable delegation

- **GIVEN** a direct child resolves a profile whose delegation field is true or
  omitted
- **WHEN** Smith composes the child runtime
- **THEN** the child receives no delegation prompt or tool
- **AND** the one-level hierarchy remains enforced

#### Scenario: Waiting for a running child is bounded

- **GIVEN** a root invokes `agent.wait` for a child that remains running
- **WHEN** the explicit bounded wait or the default wait interval elapses
- **THEN** the tool returns a structured still-running outcome to the root
- **AND** the child remains running and unchanged
- **AND** the enclosing invocation cancellation and deadline remain
  authoritative
