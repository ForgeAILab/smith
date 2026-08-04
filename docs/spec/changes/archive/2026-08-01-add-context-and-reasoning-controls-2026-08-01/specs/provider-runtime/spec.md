## ADDED Requirements

### Requirement: Exact reasoning request dialects

Smith SHALL translate a typed reasoning selection only through the exact
request dialect trusted for the resolved provider/model binding. OpenAI effort,
OpenRouter reasoning-object, and Z.AI thinking-object fields MUST remain
distinct, and unknown OpenAI-compatible endpoints MUST NOT inherit a dialect
from their name alone.

#### Scenario: OpenAI effort request

- **GIVEN** the resolved binding advertises an OpenAI-style `high` effort
- **WHEN** Smith prepares the provider request
- **THEN** it emits `reasoning_effort = "high"`
- **AND** it emits no OpenRouter `reasoning` or Z.AI `thinking` object

#### Scenario: OpenRouter unified reasoning request

- **GIVEN** the resolved OpenRouter model advertises optional `low` effort
- **WHEN** Smith prepares the provider request
- **THEN** it emits the typed OpenRouter `reasoning` object
- **AND** the object cannot override model, messages, tools, or other normalized
  fields

#### Scenario: Z.AI thinking toggle request

- **GIVEN** the resolved Z.AI model supports turn-level thinking
- **WHEN** Smith prepares a turn with thinking disabled
- **THEN** it emits `thinking.type = "disabled"`
- **AND** it does not emit `reasoning_effort`

#### Scenario: Unknown endpoint exposes no inferred control

- **GIVEN** an OpenAI-compatible endpoint has no trusted control metadata
- **WHEN** configuration or a session asks to control reasoning
- **THEN** Smith refuses before provider I/O
- **AND** it sends no guessed vendor extension or downgrade

### Requirement: Immutable per-turn reasoning selection

Smith SHALL snapshot the effective reasoning selection at turn acceptance and
apply it unchanged to every attempt and tool-call continuation in that turn.
An idle-session update MAY affect a later turn but MUST NOT race the active
selection.

#### Scenario: Tool continuation retains thinking settings

- **GIVEN** a turn starts with thinking enabled at a supported effort
- **WHEN** the model calls a tool and Smith sends a continuation
- **THEN** the initial request and continuation use the same typed selection
- **AND** preserved reasoning content follows the existing continuation policy

#### Scenario: Retry retains thinking settings

- **GIVEN** a provider attempt fails retryably
- **WHEN** Smith sends another attempt for the same turn
- **THEN** the retry uses the identical reasoning selection
- **AND** capability validation is not silently downgraded

#### Scenario: New child inherits resolved selection

- **GIVEN** the parent has a valid effective reasoning selection
- **WHEN** Smith creates a new child session
- **THEN** the child inherits that resolved selection
- **AND** a later parent override does not mutate the already-running child
