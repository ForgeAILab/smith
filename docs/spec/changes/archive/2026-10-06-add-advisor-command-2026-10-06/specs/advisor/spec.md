## ADDED Requirements

### Requirement: Session advisor override

A root session SHALL accept a session-scoped advisor override that turns the
advisor off, restores the configured advisor, or selects another profile or
`provider/model` as the advisor. The override SHALL take effect at the next
safe boundary without ending the session, SHALL NOT modify configuration
files, and SHALL NOT apply to child sessions. Tool registration and advisor
guidance SHALL follow the effective advisor after the override.

#### Scenario: Turned off

- **GIVEN** the active profile resolves advisor `sol`
- **WHEN** the session's advisor override is set to off
- **THEN** the next turn's tool list has no `advisor` tool
- **AND** the instructions contain no advisor section

#### Scenario: Switched to another target

- **GIVEN** the active profile resolves advisor `sol`
- **WHEN** the override selects `openai/gpt-5.3`
- **THEN** the next advisor call is served by `openai/gpt-5.3`
- **AND** its usage is recorded under the advisor attribution

#### Scenario: Restored to configuration

- **GIVEN** an override is in effect
- **WHEN** the override is cleared
- **THEN** the session uses the advisor that configuration resolves, or none

#### Scenario: Target does not resolve

- **WHEN** the override names a profile or model that cannot be resolved
- **THEN** the change is refused with the reason
- **AND** the previously effective advisor stays in effect

#### Scenario: Configuration untouched

- **WHEN** any advisor override is applied
- **THEN** no configuration file is written
