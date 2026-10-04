## ADDED Requirements

### Requirement: Standalone screens share one terminal loop

Smith SHALL run every screen shown outside a session (setup, `smith
--resume`, ChatGPT login method, account choice, login progress) through one
runner that owns entering and restoring the terminal, input events, ticks, and
the theme built from `--no-color` and `--no-motion`. Each screen MUST be a
value that draws itself and turns one input event into an outcome or an
effect, so it can be tested without a terminal.

#### Scenario: Adding a standalone screen

- **WHEN** a developer adds a new screen shown before a session
- **THEN** they write its state, drawing, and event handling
- **AND** they do not write a terminal loop, enter or restore the terminal,
  or build a theme from flags

#### Scenario: One loop in the CLI

- **WHEN** the structure tests inspect `smith-cli`
- **THEN** only the standalone runner and the session loop create an input
  event stream

#### Scenario: The merge changes nothing visible

- **GIVEN** terminal fixtures recorded for the five standalone screens before
  the runner existed
- **WHEN** the screens move onto the runner
- **THEN** every fixture is byte-identical

### Requirement: One chooser component

Smith SHALL draw every chooser, inside and outside a session, through one
list component in `smith-tui` that owns row layout, numbering, filtering,
scrolling, position count, and footer. A connection flow MUST be the same
screen value whether it runs standalone or inside a session.

#### Scenario: Changing the footer

- **WHEN** a developer changes the chooser footer's wording
- **THEN** the change is in one place
- **AND** setup, `smith --resume`, login, and every in-session picker show
  the new wording

#### Scenario: One connection flow in two places

- **GIVEN** the OpenRouter connection steps
- **WHEN** they run from `smith setup` and from `/connect`
- **THEN** both use the same screen value and effects
- **AND** only the area they are drawn in differs
