## ADDED Requirements

### Requirement: Headless execution follows an explicitly active goal

An ordinary headless prompt SHALL retain its existing one-turn lifecycle unless
that turn explicitly creates or activates a goal. Once a goal is active, the
headless host SHALL remain subscribed across attributed conditional internal
turns until the goal reaches a stopped state or existing process/global limits
terminate execution.

#### Scenario: Ordinary prompt completes without a goal

- **GIVEN** a headless prompt neither restores nor explicitly creates a goal
- **WHEN** its explicit turn completes
- **THEN** `smith -p` exits under existing one-turn semantics
- **AND** emits no goal record or continuation turn

#### Scenario: Explicit headless goal completes

- **GIVEN** the prompt explicitly creates a persistent active goal
- **WHEN** several internal continuations eventually mark it complete
- **THEN** the headless host observes every attributed turn and exits after the
  complete state commits
- **AND** reports the final answer and final goal usage evidence

#### Scenario: Headless goal stops without completion

- **GIVEN** an active headless goal becomes paused, blocked, usage-limited, or
  budget-limited
- **WHEN** that state commits
- **THEN** automatic continuation stops and the process exits predictably
- **AND** output distinguishes the stopped reason from successful completion

#### Scenario: Headless goal needs user interaction

- **GIVEN** no bidirectional interaction broker is configured
- **WHEN** goal work reaches a material questionnaire requirement
- **THEN** the goal becomes blocked and headless execution returns the existing
  structured `interaction_required` outcome
- **AND** includes the final goal snapshot without fabricating an answer

### Requirement: Machine output projects goal lifecycle explicitly

Goal-aware text, JSON, and JSON Lines output SHALL preserve existing non-goal
field meanings while adding bounded typed goal projections. Machine output MUST
identify final goal status, stable goal identity, usage provenance, optional
budget, actual overshoot, active elapsed time, stopped reason, and number of
continuation turns without reconstructing state from prose.

#### Scenario: JSON goal result is complete

- **GIVEN** a goal-aware headless run completes successfully
- **WHEN** Smith writes its final JSON record
- **THEN** it includes one optional structured final-goal object and
  continuation count
- **AND** existing assistant text, usage, turn, and terminal fields retain their
  documented meaning

#### Scenario: JSON Lines streams goal progress

- **GIVEN** a headless goal runs across several turns
- **WHEN** JSON Lines output is selected
- **THEN** each typed goal update and attributed turn lifecycle is emitted in
  canonical order
- **AND** consumers need not parse assistant or diagnostic text to follow state

#### Scenario: Budget overshoots by one request

- **GIVEN** the provider reports usage only after a response that crosses the
  budget
- **WHEN** machine output reports the budget-limited terminal state
- **THEN** it includes actual reported usage and the requested budget
- **AND** does not claim the budget was a pre-spend hard cap

### Requirement: Interactive and headless goal semantics are equivalent

Smith SHALL commit equivalent goal transitions, usage accounting, internal-turn
identities, tool effects, and persistence in interactive and headless hosts
given identical resolved policy, persisted goal state, provider events, and
user-independent inputs. Presentation and availability of live user controls
may differ without changing canonical goal behavior.

#### Scenario: Same deterministic goal fixture runs on both surfaces

- **GIVEN** identical persistent sessions and scripted provider/tool outcomes
- **WHEN** TUI and headless hosts execute the fixture
- **THEN** their canonical goal states, usage totals, turn sequence, and tool
  results are equivalent
- **AND** only their local rendering/output projections differ

#### Scenario: Both surfaces shut down

- **GIVEN** an active goal exists when the current Smith process shuts down
- **WHEN** either surface completes bounded shutdown
- **THEN** both persist equivalent latest goal state and stop all work
- **AND** neither surface starts detached continuation after exit
