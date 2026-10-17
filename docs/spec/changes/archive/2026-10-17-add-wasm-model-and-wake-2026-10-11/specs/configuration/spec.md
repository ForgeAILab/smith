## ADDED Requirements

### Requirement: Model and notify capabilities are reviewed and narrowable

The plugin manifest SHALL accept `model` and `session_notify` as module
capability keys. Each MUST be shown in the install and update review, MUST
be granted only by the confirmed install record, and MUST be withholdable
per module from the user file. The user file SHALL be able to lower a
module's per-call output limit and per-turn call limit, and no
configuration layer may raise them above the built-in defaults.

#### Scenario: Install shows the model capability

- **GIVEN** a plugin whose module requests `model`
- **WHEN** the user installs it
- **THEN** the review states the module can send data to the active model
  provider
- **AND** nothing is installed until the user confirms

#### Scenario: User withholds notify

- **GIVEN** an installed module granted `session_notify`
- **WHEN** the user file sets that capability to false for the module
- **THEN** its watcher is not polled
- **AND** the capability is listed as withheld by the user

#### Scenario: Project file tries to raise a limit

- **GIVEN** a project file that sets a module's per-turn call limit above
  the default
- **WHEN** configuration is resolved
- **THEN** the setting is ignored and reported
