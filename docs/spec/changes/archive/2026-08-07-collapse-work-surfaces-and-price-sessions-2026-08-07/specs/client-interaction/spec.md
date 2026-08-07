## MODIFIED Requirements

### Requirement: Replay-equivalent anchored todo pane

Smith SHALL derive one replaceable todo projection from versioned runtime
events. Public items SHALL render in a bounded, non-focusable pane immediately
above the composer and MUST NOT enter transcript history. Sensitive plan item
text MUST NOT render. Completed items SHALL collapse to at most one struck
labelled row below the open items, and a plan whose items are all completed
SHALL retire from the pane once its turn is no longer running. `/details` SHALL
toggle bounded tool lifecycle detail without revealing protected arguments.
Live reduction and journal replay MUST produce the same todo projection. A
compact picker SHALL temporarily replace the todo presentation in the anchored
pane without changing that projection.

#### Scenario: Multi-step coding turn advances
- **GIVEN** a turn has a public multi-step plan
- **WHEN** plan lifecycle events arrive
- **THEN** the authored todo items update in place immediately above the
  composer
- **AND** the transcript retains only the quiet working timer and ordinary
  attributed events

#### Scenario: Completed items collapse below the open work
- **GIVEN** a public plan has three completed items and two open items
- **WHEN** Smith renders the anchored pane
- **THEN** the two open items render first in authored order
- **AND** the completed items render as one struck row naming the most recently
  completed item and reporting `(+2 done)`
- **AND** the collapsed row counts against the same visible-row budget an
  uncollapsed item would have used

#### Scenario: One completed item needs no count
- **GIVEN** a public plan has exactly one completed item and open items remain
- **WHEN** Smith renders the collapsed row
- **THEN** it names that item struck through
- **AND** it reports no `(+N done)` suffix, because no item is hidden behind it

#### Scenario: A cancelled item is not reported as done
- **GIVEN** a public plan contains a cancelled item
- **WHEN** Smith renders the anchored pane
- **THEN** the cancelled item keeps its own row among the open items
- **AND** it is excluded from the completed collapse and its count

#### Scenario: A finished plan retires when the turn stops
- **GIVEN** every item in a public plan is completed
- **WHEN** the turn is still running
- **THEN** the pane still renders the completed plan
- **AND** once the turn succeeds, fails, is interrupted, or reaches a limit,
  the pane renders nothing rather than pinning the finished list until the next
  turn

#### Scenario: Turn reaches a terminal result with work outstanding
- **GIVEN** an anchored todo pane is visible with at least one item not
  completed
- **WHEN** the turn succeeds, fails, is interrupted, or reaches a limit
- **THEN** the reconciled terminal todo remains visible until the next turn
  starts
- **AND** Smith commits no aggregate `work` row to the transcript

#### Scenario: Compact interaction replaces the todo presentation
- **GIVEN** an anchored public todo projection is visible
- **WHEN** the user opens command or resource completion
- **THEN** the compact picker replaces the todo presentation directly above
  the fixed composer
- **AND** closing the picker restores the unchanged todo projection
- **AND** the temporary picker controls disappear with it

#### Scenario: Details remain redaction-safe
- **GIVEN** a prepared tool contains a command, edit body, or sensitive answer
- **WHEN** the user invokes `/details`
- **THEN** Smith shows only the reviewed typed projection and lifecycle evidence
- **AND** never reconstructs raw values from redacted events
