## MODIFIED Requirements

### Requirement: Advisor command

The TUI SHALL provide `/advisor` in command discovery and help. With no
argument it SHALL open the shared inline picker. The picker SHALL list off,
the configured default, each profile that can serve as an advisor, and the
same model rows `/model` lists, including installed agents, with the same
labels and availability checks. It SHALL leave out the session's own model
binding, and the current choice SHALL be preselected. A model row the advisor
cannot use SHALL be shown disabled with the reason, not hidden.
`/advisor off`, `/advisor on`, `/advisor default`, `/advisor <profile>`,
`/advisor <provider>/<model>`, and `/advisor cli/<kind>/<model>` SHALL select
directly. The command SHALL require an idle turn and SHALL confirm the result
with a notice. `/status` SHALL show the effective advisor and whether it
comes from configuration or the session override.

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

#### Scenario: Picker offers the same models as /model

- **GIVEN** `claude` is on `PATH` and `codex` is not
- **WHEN** the user runs `/advisor` with no argument
- **THEN** every `cli/claude-code/<model>` row `/model` offers is selectable
- **AND** every `cli/codex/<model>` row is shown disabled because `codex` is
  not on `PATH`
- **AND** the session's own model is not listed

#### Scenario: Select an installed agent directly

- **GIVEN** an idle session and `claude` on `PATH`
- **WHEN** the user runs `/advisor cli/claude-code/opus`
- **THEN** a notice confirms the advisor for this session
- **AND** `/status` shows advisor `cli/claude-code/opus`, attributed to the
  session override

#### Scenario: Busy turn

- **GIVEN** a turn is running
- **WHEN** the user runs `/advisor off`
- **THEN** the command is refused and the draft is preserved
