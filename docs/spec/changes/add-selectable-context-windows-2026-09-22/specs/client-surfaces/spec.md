## ADDED Requirements

### Requirement: Context window command

The TUI SHALL accept `/context <NAME|default>` to set or clear the session's
context window override. It SHALL apply the change at the next idle boundary
and persist it on resume. `/context` with no argument SHALL keep its current
report and add the available windows, marking the active one.

#### Scenario: Switch window while idle

- **GIVEN** the active model declares windows `272k` and `872k`
- **WHEN** the user runs `/context 872k` while the session is idle
- **THEN** the runtime is rebuilt with the `872k` limits
- **AND** the status line shows `872k`

#### Scenario: Model has one window

- **GIVEN** the active model declares no windows
- **WHEN** the user runs `/context 872k`
- **THEN** the TUI reports that the model has no selectable windows
- **AND** the session is unchanged

#### Scenario: Resume keeps the selection

- **GIVEN** a session ran with the `/context 872k` override
- **WHEN** that session is resumed
- **THEN** the `872k` window is still active
