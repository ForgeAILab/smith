## ADDED Requirements

### Requirement: Smith command JSONL model provider

Smith SHALL expose `command-jsonl` through Agent Runtime's command-provider
mechanism as a direct model provider implementing `smith-command-provider`
protocol revision 1. The adapter MUST run one directly invoked process per
visible provider attempt, send one versioned request on stdin, decode only
versioned machine JSONL on stdout, and preserve Agent Runtime as the owner of
canonical history, context, tools, retries, usage, cancellation, and events.

#### Scenario: Complete a streamed text turn

- **GIVEN** a compatible command bridge and selected model pass preflight
- **WHEN** Smith starts a provider attempt and the bridge emits text, usage,
  and finish frames
- **THEN** the ordinary shared provider stream produces the corresponding
  canonical events in order
- **AND** TUI and headless surfaces consume them through the existing runtime
  path
- **AND** the process exits without retaining a hidden session

#### Scenario: Runtime retries an attempt

- **GIVEN** the bridge emits a retryable error for one visible attempt
- **WHEN** Agent Runtime's configured retry policy admits another attempt
- **THEN** the first attempt and error remain recorded
- **AND** the retry starts a new process with a new attempt ID and complete
  canonical request
- **AND** the bridge performs no hidden retry of its own

### Requirement: Fixed revision-1 command capabilities

The `command-jsonl` adapter SHALL contribute an authoritative provider-local
capability record for text-only streaming with Smith/MCP tool calling and
usage reporting. Revision 1 MUST declare reasoning, structured output, cache
control/evidence, server-side continuation, and non-text modalities
unsupported, regardless of broader remote catalog claims.

#### Scenario: Tool-capable command model is resolved

- **GIVEN** a configured command model has complete trusted limits
- **WHEN** Smith freezes its resolved model profile
- **THEN** the profile advertises streaming, tools, and usage for the selected
  model
- **AND** cache and continuation remain unsupported
- **AND** the capability provenance identifies the command adapter rather than
  a provider-wide guess

#### Scenario: Reasoning is requested

- **GIVEN** a profile or turn requests reasoning, structured output, image
  input, or cache behavior from a revision-1 command provider
- **WHEN** Agent Runtime validates the request
- **THEN** it rejects the unsupported capability before process I/O
- **AND** Smith does not drop, stringify, or silently downgrade the request

### Requirement: Command-provider compatibility preflight

Smith MUST run the adapter's bounded explicit probe before terminal entry or
runtime construction. The probe SHALL require the exact protocol name, schema
revision, and selected model, and MUST treat malformed, excessive, timed-out,
unsuccessful, or incompatible output as a local startup failure without
starting inference.

#### Scenario: Compatible bridge is selected

- **GIVEN** the configured executable answers the fixed probe with protocol
  `smith-command-provider`, schema revision `1`, and the selected model
- **WHEN** Smith preflights the provider
- **THEN** it records bounded implementation name/version metadata as
  diagnostic evidence
- **AND** constructs the runtime without making an inference request

#### Scenario: Autonomous Codex CLI is selected directly

- **GIVEN** configuration points `command-jsonl` at a Codex app-server or
  another executable that does not implement the Smith model-provider probe
- **WHEN** Smith preflights the provider
- **THEN** startup fails as an incompatible command protocol
- **AND** Smith does not treat its threads, agent events, tools, MCP, or
  approvals as canonical provider output

### Requirement: Command provider preserves Smith tool ownership

Smith SHALL serialize the frozen canonical tool schemas, including connected
MCP tools, into each command-provider request. Tool-call frames from the bridge
MUST re-enter the ordinary Agent Runtime assembly, validation, authority,
approval, and execution path; neither configuration nor the bridge may execute
the tool or inject a result directly.

#### Scenario: Command model calls an MCP tool

- **GIVEN** a trusted connected MCP server contributed a tool to the frozen
  ability epoch
- **WHEN** the command request advertises that schema and the bridge emits a
  correlated tool call
- **THEN** Agent Runtime validates the call and Smith applies the existing MCP
  authority and approval policy
- **AND** a later fresh command process receives only the canonical tool result
- **AND** MCP process, credential, and side-effect ownership never move into
  the bridge

#### Scenario: Bridge invents an unavailable tool

- **GIVEN** the bridge emits a call for a tool absent from the frozen request
- **WHEN** Agent Runtime assembles the call
- **THEN** the ordinary unknown-tool policy rejects it
- **AND** no executable or MCP server is invoked on the bridge's authority

### Requirement: Bounded command-provider lifecycle and output

Smith SHALL use Agent Runtime's direct-argv command supervision, cleared
environment, bounded stdin/frame/stdout/stderr/probe limits, attempt deadline,
cancellation, early-drop cleanup, and process-tree termination without a
Smith-local process loop. Raw stderr and malformed stdout MUST NOT reach
transcripts, journals, or diagnostics.

#### Scenario: Command attempt is cancelled

- **GIVEN** a command process and descendant remain active during an attempt
- **WHEN** the attempt is cancelled, reaches its deadline, or its stream is
  dropped
- **THEN** the framework terminates the process tree within its cleanup bound
- **AND** Smith records one classified attempt outcome with no detached work

#### Scenario: Bridge violates the output contract

- **GIVEN** a bridge emits malformed JSON, an unknown frame, overlapping
  usage, no required usage, output after a terminal, a duplicate terminal, an
  oversized frame, or a successful finish followed by unsuccessful exit
- **WHEN** the framework and decoder complete the attempt
- **THEN** the attempt fails without committing a successful terminal
- **AND** raw stdout and stderr remain absent from user-visible and persisted
  error detail

### Requirement: Autonomous CLI backends remain distinct

Smith MUST NOT expose a CLI as `command-jsonl` when that integration owns an
agent thread, hidden context, tool/MCP execution, approvals, retries, or
persistence. Such a CLI requires a separately specified external-agent backend
whose events and authority are not presented as a native model-provider turn.

#### Scenario: Evaluate an agent-oriented CLI adapter

- **GIVEN** a CLI's supported protocol starts or resumes its own thread and
  completes tools before returning an agent message
- **WHEN** Smith classifies the integration
- **THEN** it is excluded from `command-jsonl`
- **AND** native and command model providers keep the one shared Smith runtime
  loop
