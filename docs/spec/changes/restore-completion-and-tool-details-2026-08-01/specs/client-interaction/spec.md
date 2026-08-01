## MODIFIED Requirements

### Requirement: Quiet successful turn terminal

The interactive TUI SHALL treat a successful `TurnCompleted` event as both a
state transition and one concise attributed transcript notice. It MUST close
active streaming/work state, return to idle, and append `turn · completed`
with canonical elapsed duration when valid start/completion timestamps are
available, while retaining canonical lifecycle, usage, journal, and timeline
evidence. Non-success terminals requiring explanation or action MUST remain
visible.

#### Scenario: A visible answer completes successfully

- **GIVEN** a turn has committed visible assistant text
- **AND** its canonical start and completion timestamps form a valid interval
- **WHEN** its successful terminal event arrives
- **THEN** the TUI closes the turn and returns to idle
- **AND** it appends one attributed completion notice with that elapsed time

#### Scenario: Tool or reasoning work completes without answer text

- **GIVEN** a successful turn reports no visible assistant output
- **WHEN** its terminal event arrives
- **THEN** the TUI appends the same successful completion notice
- **AND** it does not label the turn `reasoning only` or infer why text was
  absent

#### Scenario: Successful duration is below one second

- **GIVEN** canonical timestamps show a successful turn lasted less than one
  second
- **WHEN** the completion notice renders
- **THEN** it uses bounded millisecond precision or `<1ms`
- **AND** it does not round the duration to `0s`

#### Scenario: Successful duration is unavailable

- **GIVEN** the start timestamp is absent or later than the completion
  timestamp
- **WHEN** the successful terminal is reduced
- **THEN** the notice says `turn · completed` without a duration
- **AND** it does not substitute local reducer or replay processing time

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
- **AND** it derives duration only from the canonical timestamp interval
