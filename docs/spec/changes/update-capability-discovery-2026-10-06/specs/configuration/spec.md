## ADDED Requirements

### Requirement: Capability limits per profile

A profile SHALL be able to declare `capabilities.allow` and
`capabilities.deny` as lists of registry id patterns (`<domain>:<name>`,
with `*` as a wildcard). A denied capability SHALL be absent from the
session: not active, not listed, not searchable, and not activatable. `deny`
SHALL take precedence over `allow`. With no `allow`, every capability not
denied is allowed. An unparseable pattern SHALL fail configuration
resolution with its source.

#### Scenario: Shell denied by profile
- **GIVEN** a profile with `capabilities.deny = ["tool:shell"]`
- **WHEN** a session starts on that profile
- **THEN** the tool list has no `shell`
- **AND** a search for "shell" returns no such entry

#### Scenario: Allow list
- **GIVEN** a profile with `capabilities.allow = ["tool:*"]`
- **WHEN** the agent lists capabilities
- **THEN** only tools are listed and no skill can be activated

#### Scenario: Deny wins
- **GIVEN** `allow = ["tool:*"]` and `deny = ["tool:edit"]`
- **WHEN** a session starts
- **THEN** `edit` is absent

#### Scenario: Invalid pattern
- **GIVEN** `capabilities.deny = ["shell"]`
- **WHEN** configuration is resolved
- **THEN** resolution fails naming the key, the value, and its file
