## ADDED Requirements

### Requirement: Quiet successful turn terminal

The interactive TUI SHALL treat a successful `TurnCompleted` event as a state
transition rather than a transcript message. It MUST close active streaming and
work state and return to idle without appending a `turn · completed ...`
notice, while retaining canonical lifecycle, usage, journal, and timeline
evidence. Non-success terminals requiring explanation or action MUST remain
visible.

#### Scenario: A visible answer completes successfully

- **GIVEN** a turn has committed visible assistant text
- **WHEN** its successful terminal event arrives
- **THEN** the TUI closes the turn and returns to idle
- **AND** it appends no successful completion or elapsed-time notice

#### Scenario: Tool or reasoning work completes without answer text

- **GIVEN** a successful turn reports no visible assistant output
- **WHEN** its terminal event arrives
- **THEN** the TUI closes the turn without appending a reasoning-only success
  notice
- **AND** committed tool/result projections and canonical terminal evidence
  remain available

#### Scenario: A turn does not complete successfully

- **GIVEN** a turn is cancelled, reaches a limit, needs input, or fails
- **WHEN** its terminal event arrives
- **THEN** the TUI renders a concise attributed non-success notice
- **AND** includes locally measured elapsed time when available

#### Scenario: Journaled success is replayed

- **GIVEN** a successful turn's canonical start and completion envelopes were
  journaled
- **WHEN** the transcript is reconstructed by replay
- **THEN** replay reaches the same idle and terminal work state as live
  reduction
- **AND** neither path fabricates a successful completion transcript row
