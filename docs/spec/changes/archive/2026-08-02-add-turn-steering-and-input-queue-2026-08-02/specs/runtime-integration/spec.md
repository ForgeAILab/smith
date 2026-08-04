## ADDED Requirements

### Requirement: Smith delegates steering semantics to Agent Runtime

Smith SHALL use Agent Runtime's typed active-turn steering admission and
disposition contracts. It MUST NOT emulate steering by submitting another
whole turn, mutating an in-flight provider request, parsing provider output, or
treating the generic monitor/child injection inbox as an indistinguishable user
steer.

#### Scenario: Runtime accepts a steer

- **GIVEN** Smith tracks the eligible serving turn identity
- **WHEN** the runtime accepts a matching ordinary user input as a steer
- **THEN** Smith retains the returned stable steer identity until disposition
- **AND** the runtime owns safe-boundary delivery and same-turn continuation

#### Scenario: Serving turn changes during submission

- **GIVEN** Smith's tracked turn identity becomes stale before steering
- **WHEN** Agent Runtime returns a typed mismatch or no-active-turn result
- **THEN** Smith retries at most once against a reported eligible active turn or
  falls back to ordinary idle submission
- **AND** the input is neither lost nor submitted twice

### Requirement: User input wins automatic continuation admission

Smith SHALL dispatch a pending real-user follow-up before allowing an idle-only
goal continuation attempt at the same terminal boundary. An accepted steer to
a goal-owned serving turn MUST remain real user input and MUST NOT change goal
identity, authority, or accounting policy implicitly.

#### Scenario: Goal and queued user input reach an idle boundary

- **GIVEN** an active goal is eligible for automatic continuation
- **AND** Smith has one queued real-user turn
- **WHEN** the serving turn reaches its terminal boundary
- **THEN** Smith submits the real-user turn first
- **AND** the goal controller observes busy or waits for the later boundary

### Requirement: Pending input is process-local until runtime commitment

Smith SHALL label steering and future-turn queue state as process-local and
MUST include it in live-work exit policy. Session journals and replay MUST
represent only runtime-committed input; they MUST NOT fabricate commitment for
pending state lost to an unclean process exit.

#### Scenario: Process exits before steer commitment

- **GIVEN** a steer was accepted in process but no committed disposition was
  recorded
- **WHEN** a later process resumes the canonical session
- **THEN** replay does not claim that the steer entered model history
- **AND** Smith does not invent or automatically resend unavailable text
