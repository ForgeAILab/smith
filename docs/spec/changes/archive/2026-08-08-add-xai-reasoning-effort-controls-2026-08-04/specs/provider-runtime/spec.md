## MODIFIED Requirements

### Requirement: Exact reasoning request dialects

Smith SHALL translate a typed reasoning selection only through the exact
request dialect trusted for the resolved provider/model binding. OpenAI effort,
OpenRouter reasoning-object, and Z.AI thinking-object fields MUST remain
distinct, and unknown OpenAI-compatible endpoints MUST NOT inherit a dialect
from their name alone. The exact normalized xAI Responses endpoint SHALL grant
the OpenAI-effort dialect to catalog-advertised reasoning models the same way
the OpenAI endpoint does, using Models.dev effort ladders when present and the
universal `low`/`medium`/`high` fallback otherwise.

#### Scenario: OpenAI effort request

- **GIVEN** the resolved binding advertises an OpenAI-style `high` effort
- **WHEN** Smith prepares the provider request
- **THEN** it emits `reasoning_effort = "high"`
- **AND** it emits no OpenRouter `reasoning` or Z.AI `thinking` object

#### Scenario: xAI catalog endpoint grants OpenAI-effort controls

- **GIVEN** the resolved provider uses the exact xAI catalog endpoint
- **AND** the selected model is a catalog-advertised reasoning model
- **WHEN** configuration or a session selects an advertised effort such as
  `high`
- **THEN** Smith resolves the OpenAI-effort dialect before provider I/O
- **AND** prefers the Models.dev effort ladder when the snapshot advertises one
- **AND** refuses `off` unless that ladder includes `none`

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
