## ADDED Requirements

### Requirement: Bridged calls render as Smith tool calls

TUI and headless surfaces SHALL present a call the CLI made through the
bridge as a Smith tool call: one transcript row per call, the ordinary
approval prompt, and the ordinary tool events in machine output. It MUST be
distinguishable from a tool the CLI ran itself, and it MUST NOT also appear
as an external tool row. The turn keeps its harness label.

A turn that offered the bridge and ended without the CLI ever connecting to
it SHALL report that as a diagnostic naming the harness.

#### Scenario: A bridged call in the TUI

- **GIVEN** a harness turn in which the CLI calls `agent` through the bridge
- **WHEN** the transcript renders
- **THEN** the call appears once, as a Smith tool row
- **AND** a file the CLI read itself in the same turn appears as a CLI-run
  tool

#### Scenario: A bridged call in headless machine output

- **GIVEN** a headless harness run with machine output
- **WHEN** the CLI makes a bridged call
- **THEN** the call and its result appear under the ordinary tool event types
- **AND** no external tool event is emitted for the same call

#### Scenario: An approval for a bridged call

- **WHEN** a bridged call needs approval
- **THEN** the prompt is the one a native call to that tool would show

#### Scenario: The CLI never connected

- **GIVEN** a harness turn that offered the bridge
- **WHEN** the turn ends and the CLI never connected to the endpoint
- **THEN** a diagnostic says the harness did not load Smith's tools
