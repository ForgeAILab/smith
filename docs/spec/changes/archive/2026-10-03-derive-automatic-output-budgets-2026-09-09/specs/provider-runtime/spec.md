## ADDED Requirements

### Requirement: Automatic request output budget

Smith SHALL derive one deterministic per-request output budget when no explicit
`max_output_tokens` value is selected. The budget MUST be bounded by the frozen
model output ceiling, 32,768 tokens, one quarter of the context window, and the
space remaining after the reasoning reserve while preserving input space. The
same value MUST be sent as the ordinary provider request maximum and used as
the default context output reserve.

#### Scenario: Large output ceiling receives a bounded default

- **GIVEN** a model profile has a 500,000-token context and output ceiling
- **AND** the reasoning reserve is zero
- **AND** no explicit request output limit or context output reserve exists
- **WHEN** Smith constructs the runtime
- **THEN** the provider loop requests at most 32,768 output tokens
- **AND** the context planner reserves exactly 32,768 output tokens
- **AND** the immutable model ceiling remains 500,000 tokens

#### Scenario: Small model remains input-usable

- **GIVEN** a model's output ceiling equals its context window
- **AND** one quarter of that context is less than 32,768 tokens
- **WHEN** Smith derives the automatic request budget
- **THEN** the request and default reserve use at most one quarter of context
- **AND** the model retains input space without a user-authored limit

#### Scenario: Explicit request limit retains precedence

- **GIVEN** configuration selects a valid `max_output_tokens` below the model
  ceiling
- **WHEN** Smith constructs the runtime
- **THEN** that explicit value is the provider request maximum
- **AND** it is the default output reserve unless
  `context.output_reserve` is also explicit
- **AND** the automatic budget does not override either explicit value

#### Scenario: Reasoning reserve leaves no derivable budget

- **GIVEN** the selected reasoning reserve leaves no space for both output and
  input inside the model context window
- **WHEN** no explicit request output limit exists
- **THEN** runtime preflight fails before provider or credential I/O
- **AND** the model inventory reports the same bounded reason

#### Scenario: Frozen policy is coherent across a turn

- **GIVEN** a catalog-backed runtime derives an automatic request output budget
- **WHEN** the turn retries, continues after a tool result, or constructs a
  child from the same frozen model profile
- **THEN** every request uses the same effective budget
- **AND** the budget participates in the runtime identity rather than being
  recomputed from a later catalog refresh
