## ADDED Requirements

### Requirement: Advisor command

The TUI SHALL provide `/advisor` in command discovery and help. With no
argument it SHALL open the shared inline picker listing off, the configured
default, and each profile and model that can serve as an advisor, with the
current choice preselected. `/advisor off`, `/advisor on`, `/advisor default`,
`/advisor <profile>`, and `/advisor <provider>/<model>` SHALL select directly.
The command SHALL require an idle turn, SHALL confirm the result with a
notice, and `/status` SHALL show the effective advisor and whether it comes
from configuration or the session override.

#### Scenario: Listed in discovery

- **WHEN** the user types `/`
- **THEN** the command list includes `/advisor` with its argument hint

#### Scenario: Disable from the TUI

- **GIVEN** an idle session with an advisor
- **WHEN** the user runs `/advisor off`
- **THEN** a notice reports the advisor is off for this session
- **AND** `/status` shows no advisor, attributed to the session override

#### Scenario: Enable with nothing configured

- **GIVEN** configuration names no advisor
- **WHEN** the user runs `/advisor on`
- **THEN** the command is refused with guidance to name a profile or
  `provider/model`

#### Scenario: Picker

- **WHEN** the user runs `/advisor` with no argument
- **THEN** the picker lists off, default, and the available targets
- **AND** the current choice is preselected

#### Scenario: Busy turn

- **GIVEN** a turn is running
- **WHEN** the user runs `/advisor off`
- **THEN** the command is refused and the draft is preserved
