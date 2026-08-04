## ADDED Requirements

### Requirement: Local session and child timeline

Smith SHALL provide `/timeline` as a bounded local transcript result containing
ordered root turns, child runs, terminal plan/gate outcomes, and undo/revert/
redo transactions. Timeline inspection MUST spend no provider tokens, expose
no protected arguments, and preserve the root composer draft.

#### Scenario: Inspect an agent-heavy session
- **GIVEN** a session contains two turns, one child review, and one undo
- **WHEN** the user invokes `/timeline`
- **THEN** Smith renders those entries in canonical sequence with stable IDs
- **AND** offers bounded child/result and recovery navigation locally

#### Scenario: Timeline is replayed
- **GIVEN** the session is resumed from journal and checkpoint state
- **WHEN** `/timeline` is rendered
- **THEN** its committed entries match the live session ordering and outcomes
- **AND** interrupted ephemeral children are labelled rather than restarted

### Requirement: Safe exact redo

`/redo` SHALL target only the newest successful undo or selective revert whose
recorded forward patch is exact and whose current paths match the expected
pre-image. Smith MUST preview the complete patch, require explicit non-default
confirmation, apply atomically, and journal the result.

#### Scenario: Redo an exact undo
- **GIVEN** Smith successfully undid one fully attributable edit turn and no
  affected path changed afterward
- **WHEN** the user invokes `/redo`, reviews, and confirms
- **THEN** Smith reapplies the exact forward patch atomically
- **AND** records a redo transaction linked to the original turn and undo

#### Scenario: Concurrent edit blocks redo
- **GIVEN** an affected path changed after the undo
- **WHEN** the user attempts `/redo`
- **THEN** Smith refuses without modifying any path
- **AND** points to `/diff` and `/timeline` with a structured conflict

#### Scenario: Ambiguous shell delta is not redoable
- **GIVEN** a prior recovery record depends on an unattributable shell delta
- **WHEN** the user invokes `/redo`
- **THEN** Smith reports that no exact redo candidate exists
- **AND** does not synthesize or apply a patch from observed Git state
