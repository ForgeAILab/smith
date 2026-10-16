## ADDED Requirements

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

## MODIFIED Requirements

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
