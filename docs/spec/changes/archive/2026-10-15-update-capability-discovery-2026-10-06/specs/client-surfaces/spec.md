## ADDED Requirements

### Requirement: Capabilities command

The TUI SHALL provide `/capabilities`, which lists the capabilities active in
the session, those available to activate, and those denied, with whether the
profile or the session denied each. `/capabilities deny <id>` SHALL add a
session denial and `/capabilities allow <id>` SHALL remove one the session
added. Both SHALL require an idle turn and apply at the next safe boundary. A
session SHALL NOT be able to allow a capability its profile denies.

#### Scenario: Listing
- **WHEN** the user runs `/capabilities`
- **THEN** the result shows active, available, and denied capabilities
- **AND** each denial names the profile or the session as its source

#### Scenario: Deny for this session
- **GIVEN** an idle session with `shell` active
- **WHEN** the user runs `/capabilities deny tool:shell`
- **THEN** the next provider request has no `shell` tool
- **AND** `/capabilities` shows it as denied by the session

#### Scenario: Cannot widen the profile
- **GIVEN** a profile that denies `tool:shell`
- **WHEN** the user runs `/capabilities allow tool:shell`
- **THEN** the command is refused and names the profile setting
