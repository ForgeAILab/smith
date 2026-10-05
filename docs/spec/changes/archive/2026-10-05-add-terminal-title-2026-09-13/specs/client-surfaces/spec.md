## ADDED Requirements

### Requirement: Dynamic terminal window title

The interactive TUI SHALL set the terminal window/tab title from current
session state (product name, project, model, and an in-flight activity label)
so concurrent sessions are distinguishable at a glance. Title text assembled
from untrusted-derived inputs (project paths, model ids) MUST be sanitized
before emission: no control characters, no bidi/invisible formatting
codepoints, collapsed whitespace, and a bounded length. The TUI MUST NOT emit
a title sequence when stdout is not a terminal, and it SHALL clear the title
it last wrote when the session exits, without attempting to restore any
pre-session title.

#### Scenario: Concurrent sessions are distinguishable

- **GIVEN** two terminals running Smith TUI sessions in different projects
- **WHEN** both TUIs are running
- **THEN** each window/tab title contains its own project and model
- **AND** the two titles differ without user configuration

#### Scenario: Title tracks turn activity

- **GIVEN** a running TUI session
- **WHEN** a turn starts and later completes
- **THEN** the title gains an activity label while the turn is in flight
- **AND** returns to the idle form when the turn ends

#### Scenario: Untrusted text cannot shape the escape sequence

- **GIVEN** a project directory whose name contains control characters or
  bidi override codepoints
- **WHEN** the title is rendered
- **THEN** those codepoints are stripped from the emitted payload
- **AND** the only escape bytes written are the single OSC introducer and
  terminator the TUI itself emitted

#### Scenario: Headless output stays clean

- **GIVEN** `smith -p` with stdout piped or redirected
- **WHEN** the run completes
- **THEN** stdout contains no OSC title sequence

#### Scenario: Exit clears only the managed title

- **WHEN** the TUI exits normally
- **THEN** the title the TUI wrote is cleared with an empty title payload
- **AND** no attempt is made to read or restore the terminal's previous title
