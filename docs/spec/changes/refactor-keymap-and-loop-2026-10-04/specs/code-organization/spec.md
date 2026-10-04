## ADDED Requirements

### Requirement: One definition per listed key binding

Smith SHALL define each key binding that `/help` or the shortcuts panel
lists once, in a typed table that supplies its chord, the context it applies
in, its effect, and its help text. `/help` and the shortcuts panel MUST
render from that table, and a test MUST check every binding against the
TUI's key handling.

#### Scenario: A listed key does nothing

- **GIVEN** a binding in the table whose chord, in its context, does not
  produce its effect in the TUI's key handling
- **WHEN** the workspace tests run
- **THEN** the binding check fails and names the binding

#### Scenario: Help text is unchanged

- **WHEN** `/help` or the shortcuts panel renders from the table
- **THEN** its rows match the previous hand-written list byte for byte

### Requirement: The interactive loop is split by event source

The interactive TUI loop SHALL keep its state in one value and handle each
event source and each user action in its own function, so that no single
function holds the whole loop.

#### Scenario: Changing one action

- **WHEN** a developer changes how one user action is handled
- **THEN** the change is inside that action's function
- **AND** the function that runs the loop only selects the next event and
  hands it on
