# advisor Specification

## Purpose
How the main model consults a stronger advisor model through a model-facing tool, what the advisor sees, and how its advice and usage are reported.
## Requirements
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

### Requirement: Session advisor override

A root session SHALL accept a session-scoped advisor override that turns the
advisor off, restores the configured advisor, or selects another profile,
`provider/model`, or installed agent `cli/<kind>/<model>` as the advisor. The
override SHALL take effect at the next safe boundary without ending the
session, SHALL NOT modify configuration files, and SHALL NOT apply to child
sessions. Tool registration and advisor guidance SHALL follow the effective
advisor after the override.

#### Scenario: Turned off

- **GIVEN** the active profile resolves advisor `sol`
- **WHEN** the session's advisor override is set to off
- **THEN** the next turn's tool list has no `advisor` tool
- **AND** the instructions contain no advisor section

#### Scenario: Switched to another target

- **GIVEN** the active profile resolves advisor `sol`
- **WHEN** the override selects `openai/gpt-5.3`
- **THEN** the next advisor call is served by `openai/gpt-5.3`
- **AND** its usage is recorded under the advisor attribution

#### Scenario: Switched to an installed agent

- **GIVEN** the active profile resolves advisor `sol`
- **WHEN** the override selects `cli/claude-code/opus`
- **THEN** the next advisor call runs on `claude` with model `opus`
- **AND** its usage is recorded under the advisor attribution

#### Scenario: Restored to configuration

- **GIVEN** an override is in effect
- **WHEN** the override is cleared
- **THEN** the session uses the advisor that configuration resolves, or none

#### Scenario: Target does not resolve

- **WHEN** the override names a profile or model that cannot be resolved
- **THEN** the change is refused with the reason
- **AND** the previously effective advisor stays in effect

#### Scenario: Configuration untouched

- **WHEN** any advisor override is applied
- **THEN** no configuration file is written

### Requirement: Advisor on a harness session

Smith SHALL offer the `advisor` tool through the bridge on a root harness
session whose profile resolves an advisor, and it SHALL behave as it does in
a native session: no arguments, no approval prompt, the answer returned as
the tool result, and usage recorded under the advisor attribution.

The transcript sent to the advisor SHALL be built from what Smith observed
of the conversation: earlier turns, and for the current turn the CLI's text,
the bridged calls and their results, and the tools the CLI ran itself with
their outcomes. A tool the CLI ran itself MUST be labelled as run by the
CLI, not presented as a Smith tool call. The existing input-budget omission
rule applies.

#### Scenario: Mid-turn call from a harness

- **GIVEN** the CLI read a file itself and spawned a child through the
  bridge earlier in the current turn
- **WHEN** it calls `advisor`
- **THEN** the advisor request contains the user's task, the CLI's read
  labelled as run by the CLI, and the bridged spawn with its result

#### Scenario: The CLI's own tools are off

- **GIVEN** a harness running without its own tools
- **WHEN** the CLI calls `advisor`
- **THEN** the transcript contains the conversation and the bridged calls

#### Scenario: No advisor configured

- **GIVEN** a harness session whose profile resolves no advisor
- **WHEN** a turn runs
- **THEN** `advisor` is not bridged and no advisor guidance is delivered

### Requirement: Installed-agent advisor

Smith SHALL run an advisor whose resolved model is an installed agent,
`cli/<kind>/<model>`, on that CLI, and SHALL NOT call the provider that
advisor resolved for identity and limits. Smith SHALL choose how an advisor call executes with the
same selection that main and child turns use. This applies whether the target
names the agent directly or names a profile whose model is one. The
`[harness.<kind>]` executable, arguments, and environment SHALL apply as
they do to a turn.

Each call SHALL be a fresh CLI turn: no resumed CLI session, no bridge, no
Smith tools offered, and the CLI's own tools off. The built-in reviewer
prompt, any profile instructions, and any selected skill instructions SHALL
be delivered as the CLI's instructions. The transcript SHALL be delivered as
the turn's input, built and bounded as for a provider advisor. The CLI's text
SHALL be the advice. Tools the CLI ran itself in its read-only mode SHALL NOT
fail the call. The result, cancellation, deadline, output-bound, and usage
rules SHALL be the same as for a provider advisor. Cost SHALL be reported as
unknown when no price reference exists.

A missing program SHALL NOT prevent the session from starting. Each advisor
call SHALL instead return a tool error naming the program that is not on
`PATH`.

#### Scenario: Advisor named by agent id

- **GIVEN** the session's advisor is `cli/claude-code/opus` and `claude` is
  on `PATH`
- **WHEN** the main model calls `advisor`
- **THEN** Smith runs one `claude` turn with model `opus` and its own tools
  off
- **AND** that turn receives the reviewer prompt as instructions and the
  transcript as input
- **AND** the tool result is the CLI's answer
- **AND** no provider request is made for the advisor

#### Scenario: Profile advisor whose model is an agent

- **GIVEN** profile `reviewer` with model `cli/codex/gpt-6-sol` and
  `advisor = "reviewer"`
- **WHEN** the main model calls `advisor`
- **THEN** Smith runs one `codex` turn in its read-only sandbox
- **AND** the turn's instructions include `reviewer`'s instructions

#### Scenario: Calls do not share a CLI session

- **GIVEN** an installed-agent advisor that answered once
- **WHEN** the main model calls `advisor` again
- **THEN** the second CLI turn is not offered the first turn's session

#### Scenario: Usage is attributed

- **GIVEN** the CLI reports token usage for the advisor turn
- **WHEN** the call finishes
- **THEN** the usage is recorded under the advisor attribution
- **AND** `/status` reports its cost as unknown

#### Scenario: CLI fails or answers nothing

- **WHEN** the advisor CLI turn fails, or completes with no text
- **THEN** the tool result is an error naming the failure
- **AND** the main model's turn continues

#### Scenario: Program missing

- **GIVEN** configuration selects advisor `cli/claude-code/opus` and `claude`
  is not on `PATH`
- **WHEN** the session starts and the main model calls `advisor`
- **THEN** the session starts
- **AND** the tool result is an error saying `claude` is not on `PATH`

#### Scenario: Interrupted

- **GIVEN** an installed-agent advisor call is running
- **WHEN** the user interrupts the turn
- **THEN** the CLI turn is cancelled and the call ends as cancelled
