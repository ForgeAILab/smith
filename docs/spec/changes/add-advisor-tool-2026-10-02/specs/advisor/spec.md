## ADDED Requirements

### Requirement: Advisor tool

Smith SHALL register a model-facing `advisor` tool on a root session whose
active profile resolves an advisor, and SHALL NOT register it on a child
session or when no advisor is configured. The tool SHALL take no arguments,
SHALL declare no workspace effects or permissions, and SHALL NOT present an
approval prompt.

#### Scenario: Advisor configured

- **GIVEN** the active main profile resolves advisor `sol`
- **WHEN** a root session starts
- **THEN** the model's tool list includes `advisor` with an empty argument
  schema

#### Scenario: No advisor

- **GIVEN** no `advisor` key applies to the active profile
- **WHEN** a session starts
- **THEN** no `advisor` tool is registered and no advisor section appears in
  the instructions

#### Scenario: Child session

- **GIVEN** the root profile resolves an advisor
- **WHEN** a child agent starts
- **THEN** the child has no `advisor` tool

### Requirement: Advisor sees the whole conversation

When the `advisor` tool is invoked, Smith SHALL send the advisor's model the
session's conversation so far, including the current turn's tool calls and
results, as a plain-text transcript framed as data, together with a built-in
reviewer prompt and, for a profile advisor, that profile's instructions. The request
SHALL declare no tools. If the transcript exceeds the advisor's input budget,
Smith SHALL omit the oldest messages after the first user message and SHALL
say how many were omitted.

#### Scenario: Mid-turn call

- **GIVEN** the main model ran `read` and `shell` earlier in the current turn
- **WHEN** it calls `advisor`
- **THEN** the advisor request contains the user's task, both tool calls, and
  both results

#### Scenario: Transcript too long

- **GIVEN** the rendered transcript exceeds the advisor's input budget
- **WHEN** the advisor is called
- **THEN** the request keeps the first user message and the most recent
  messages
- **AND** states the number of omitted messages

#### Scenario: Images

- **WHEN** the conversation contains an image
- **THEN** the transcript shows `[image omitted]` in its place

### Requirement: Advisor result

Smith SHALL return the advisor's answer as the tool result. A provider error,
timeout, cancellation, or empty answer SHALL become a tool error result with
a short reason, and the turn SHALL continue. An interrupt of the turn SHALL
cancel an advisor call in flight.

#### Scenario: Advice returned

- **WHEN** the advisor answers
- **THEN** the tool result is the answer's text, bounded by the tool output
  limit

#### Scenario: Advisor provider fails

- **GIVEN** the advisor provider returns an error
- **WHEN** the main model calls `advisor`
- **THEN** the tool result is an error naming the failure
- **AND** the main model's turn continues

### Requirement: Advisor guidance

The main agent's instructions SHALL include advisor guidance when, and only
when, the `advisor` tool is registered: consult the advisor before
substantive work, when stuck, and before declaring the task complete, and
weigh its advice seriously.

#### Scenario: Guidance follows the tool

- **WHEN** the `advisor` tool is registered
- **THEN** the instructions contain the advisor section
- **AND** a session without the tool has no such section

### Requirement: Advisor usage is accounted

Smith SHALL record the advisor's provider usage in the session's usage and
cost under an advisor attribution.

#### Scenario: Cost after an advisor call

- **GIVEN** an advisor call that reported usage
- **WHEN** the user runs `/status`
- **THEN** the session's usage and cost include the advisor call
