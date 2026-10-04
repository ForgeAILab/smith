## ADDED Requirements

### Requirement: Every runtime event projects to a known client event

Smith SHALL mirror every event variant of the pinned Agent Runtime in its
client event projection, and MUST fail to build, not degrade to an unknown
event at runtime, when the pinned runtime adds a variant Smith does not
mirror. A context or output budget failure SHALL reach the user.

#### Scenario: A budget cannot be satisfied

- **GIVEN** a turn whose context or output budget cannot be met
- **WHEN** the runtime reports the budget failure
- **THEN** the TUI shows which budget failed with the requested and allowed
  token counts
- **AND** headless stream output carries it as a typed event

#### Scenario: The runtime adds an event

- **GIVEN** a runtime revision with an event variant Smith does not mirror
- **WHEN** Smith is built against it
- **THEN** the build fails in the projection test that names the variant
