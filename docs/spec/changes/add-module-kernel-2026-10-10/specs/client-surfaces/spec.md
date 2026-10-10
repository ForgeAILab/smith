## ADDED Requirements

### Requirement: Module list and switch

The TUI SHALL provide `/modules`, listing every module known to the running
binary plus every first-party module it was built without. Each row MUST
show the module id, a one-line description, its state (mounted, off, not
built, blocked, or failed), the configuration key and layer that decided it,
and whether it is first-party or third-party. `/modules <id> on|off` MUST
write only the user configuration file, through the previewed and
rollback-capable user-config edit, after confirmation, and MUST apply at the
next safe boundary. When a higher-precedence layer overrides the user value,
Smith MUST report that instead of claiming the change took effect.
`smith config modules` SHALL print the same listing without starting a
session.

#### Scenario: Switch a module off

- **GIVEN** `image-generation` is mounted from its default
- **WHEN** the user runs `/modules image-generation off` and confirms
- **THEN** `~/.smith/config.toml` gains `modules.image-generation.enabled = false`
- **AND** the change applies at the next safe boundary
- **AND** `/modules` shows it off with the user file as its source

#### Scenario: Project layer overrides the user value

- **GIVEN** a project file turns `image-generation` on
- **WHEN** the user switches it off
- **THEN** Smith reports that the project file still turns it on
- **AND** does not claim the module is off

#### Scenario: Module is not built

- **GIVEN** a binary built without `image-generation`
- **WHEN** the user runs `/modules image-generation on`
- **THEN** Smith refuses, saying the module is not in this build
- **AND** writes nothing

#### Scenario: Third-party module is listed

- **GIVEN** a binary with a compiled-in third-party module
- **WHEN** the user runs `/modules`
- **THEN** its row is marked third-party and names its crate

#### Scenario: Unknown module id

- **WHEN** the user runs `/modules nope on`
- **THEN** Smith refuses and lists the known ids

### Requirement: Module status items

Smith SHALL accept declarative status items from a mounted module, each a
bounded label with an optional severity. The TUI SHALL render them in the status
area in a deterministic order and MUST truncate rather than let an item
displace the built-in status. A module MUST NOT receive a renderer handle,
and an item MUST disappear at the safe boundary where its module unmounts.

#### Scenario: Budget notice is visible to the user

- **GIVEN** `budget-notice` is mounted
- **WHEN** context pressure crosses the notice threshold
- **THEN** the status area shows the module's item while the model-facing
  notice is active
- **AND** the item is gone once the notice no longer applies

#### Scenario: Oversized status item

- **GIVEN** a module contributes a status label longer than the bound
- **WHEN** the TUI renders the status area
- **THEN** the label is truncated
- **AND** the built-in status remains fully visible
