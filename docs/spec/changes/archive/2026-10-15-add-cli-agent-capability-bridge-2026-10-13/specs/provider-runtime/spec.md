## ADDED Requirements

### Requirement: Harness turns are offered Smith capabilities through a turn-scoped bridge

For each harness turn whose bridged set is not empty, Smith SHALL serve an
MCP endpoint for the duration of that turn and SHALL name it in the CLI
invocation as one server, `smith`, so the CLI's model can call Smith
capabilities as MCP tools. Smith MUST supply the endpoint again on every
invocation, including one that resumes the CLI's session.

When the bridged set is empty, Smith MUST NOT start an endpoint and MUST
invoke the CLI exactly as it would without this capability. Smith MUST NOT
change the CLI's own MCP servers, settings, or tool permissions beyond
pre-allowing the `smith` server.

#### Scenario: A harness session with delegation enabled

- **GIVEN** a root session on `cli/claude-code/sonnet` whose profile enables
  delegation
- **WHEN** a turn runs
- **THEN** the CLI is invoked with the `smith` server declared
- **AND** the CLI's tool list includes Smith's `agent` tool

#### Scenario: Nothing to bridge

- **GIVEN** a harness session whose authorized tools are all excluded or
  denied
- **WHEN** a turn runs
- **THEN** no endpoint is started
- **AND** the CLI's argument list is the one built without the bridge

#### Scenario: A resumed CLI session

- **GIVEN** a completed harness turn that reported a session identity
- **WHEN** the next turn resumes that session
- **THEN** the invocation declares the endpoint serving the new turn

#### Scenario: The endpoint cannot start

- **GIVEN** a non-empty bridged set
- **WHEN** Smith cannot bind the endpoint
- **THEN** the turn fails with the reason before the CLI is started

### Requirement: The bridged set excludes what the CLI replaces

The bridged set SHALL be the tools the session is authorized to use, less
the core coding tools (`read`, `list`, `search`, `edit`, `shell`,
`task_output`, `task_stop`), the todo-planning tool, and the goal tools.
Posture, the child-surface rules, a child's tool scope, the profile's
delegation and advisor settings, and capability limits SHALL narrow the
bridged set exactly as they narrow a native session's tools. A capability
that is denied or not registered MUST NOT be reachable through the bridge.

Tools that only support other bridged capabilities MUST NOT keep the set
non-empty by themselves: `registry.search` and `registry.activate` SHALL be
bridged only when the session is authorized for at least one skill, and
`artifact.read` only when at least one other tool is bridged.

#### Scenario: Only support tools remain

- **GIVEN** a harness session whose every tool and skill is denied
- **WHEN** the bridged set is built
- **THEN** it is empty
- **AND** `registry.search` is not offered by itself

#### Scenario: Core tools stay with the CLI

- **GIVEN** a harness session authorized for `shell` and `agent`
- **WHEN** the CLI lists the `smith` server's tools
- **THEN** `agent` is listed
- **AND** `shell` is not

#### Scenario: A denied capability

- **GIVEN** a profile whose `capabilities.deny` matches `agent`
- **WHEN** the CLI lists the `smith` server's tools
- **THEN** `agent` is not listed
- **AND** a call naming it is refused without being dispatched

#### Scenario: A child on a harness

- **GIVEN** a delegated child running on `cli/codex/gpt-6-sol`
- **WHEN** the CLI lists the `smith` server's tools
- **THEN** neither `agent` nor `advisor` is listed

### Requirement: Bridged calls are dispatched as Smith tool calls

A call the CLI makes through the bridge SHALL be executed by Smith through
the same schema validation, authorization, approval, workspace enforcement,
cancellation, and output bound as a tool call requested by a model in a
native turn, and SHALL be recorded in the session's events and history as a
Smith tool call. Its result SHALL be returned to the CLI as the MCP tool
result; a refusal, denial, or failure SHALL be returned as a tool error and
MUST NOT fail the turn.

A call the CLI abandons, by cancelling the request or closing the
connection, MUST cancel any pending approval and MUST NOT execute
afterwards. Smith MUST configure the CLI so that the CLI's own tool-call
timeout cannot expire while Smith is still deciding or running a bridged
call, because a CLI may give up on a call without telling the server.

#### Scenario: An approval outlasts the CLI's default timeout

- **GIVEN** a bridged call waiting for approval for longer than the CLI's
  default tool-call timeout
- **WHEN** the user approves it
- **THEN** the CLI is still waiting and receives the result

#### Scenario: A bridged call needs approval

- **GIVEN** an approval policy that asks before a remote tool runs
- **WHEN** the CLI calls that tool through the bridge
- **THEN** Smith requests approval before invoking it
- **AND** the CLI receives the result only after the decision

#### Scenario: Approval is denied

- **WHEN** the user denies a bridged call
- **THEN** the CLI receives a tool error naming the denial
- **AND** the turn continues

#### Scenario: Headless run without an approver

- **GIVEN** a headless harness run whose policy cannot approve a bridged call
- **WHEN** the CLI makes that call
- **THEN** the call fails closed under the existing headless approval rule

#### Scenario: A question with nobody to answer it

- **GIVEN** a headless harness run with no interactive user
- **THEN** the tool that asks the user a question is not offered to the CLI,
  as it is not offered to a native session in the same run
- **AND** a call to it anyway is refused as an unknown tool, no answer is
  invented, and the turn continues
- **AND** no pending question is left on the session to block a later turn

#### Scenario: The CLI gives up on a call

- **GIVEN** a bridged call waiting for approval
- **WHEN** the CLI cancels the request
- **THEN** the approval prompt is withdrawn
- **AND** approving it afterwards executes nothing

#### Scenario: The turn is interrupted

- **GIVEN** a bridged call in flight
- **WHEN** the user interrupts the turn
- **THEN** the call is cancelled with the turn

### Requirement: The bridge endpoint is private to one turn

The endpoint SHALL accept connections only from processes of the user
running Smith, SHALL NOT listen on a network interface, and SHALL stop
accepting calls when its turn reaches a terminal event. Smith MUST NOT place
a credential in the CLI's argument list to reach it. A call arriving for a
turn that has ended MUST be refused.

#### Scenario: Another local user

- **GIVEN** a harness turn with a running endpoint
- **WHEN** a process owned by a different user tries to connect
- **THEN** the connection is refused

#### Scenario: The turn ends

- **GIVEN** a harness turn that has completed
- **WHEN** a process connects to the endpoint that served it
- **THEN** no call is dispatched

#### Scenario: Argument list is inspected

- **WHEN** another process reads the CLI's argument list
- **THEN** it contains nothing that authorizes a call to the endpoint

### Requirement: Bridged tool names are stable and unambiguous

Smith SHALL present each bridged tool under a name that is valid for the
CLI, derived deterministically from the Smith tool name, and unique within
the bridge. A tool whose name cannot be presented within those constraints
MUST be omitted and reported as a diagnostic; it MUST NOT be truncated or
merged with another tool.

#### Scenario: A dotted name

- **WHEN** `registry.search` is bridged
- **THEN** it is presented under the same valid name in every turn

#### Scenario: Two names collide for the CLI

- **GIVEN** two authorized tools whose names are equal once the CLI
  replaces characters it does not allow
- **WHEN** the bridged set is built
- **THEN** both are omitted from the bridge
- **AND** the diagnostic names both tools

### Requirement: Bridged capability guidance reaches the CLI

Smith SHALL deliver the instruction fragment of each bridged capability to
the CLI's model as the instructions of the `smith` server, and MUST NOT
include a fragment for a capability that is not bridged in that invocation.
The assembled guidance MUST be at most 4,000 characters. Smith MUST NOT send
its system prompt, project instructions, or memory index.

A CLI that records its instructions when a conversation starts keeps that
conversation's guidance as first delivered. Smith MUST NOT disable that
record to refresh guidance, because doing so re-sends the CLI's whole prompt
on every turn. The tools the CLI lists remain exact on every turn.

#### Scenario: Delegation guidance follows the tool

- **GIVEN** a harness session with `agent` bridged
- **WHEN** the CLI connects to the `smith` server
- **THEN** the server's instructions contain the delegation guidance
- **AND** a session without `agent` bridged receives none

#### Scenario: The largest bridged set

- **GIVEN** every capability that has an instruction fragment is bridged
- **WHEN** the guidance is assembled
- **THEN** it is at most 4,000 characters

#### Scenario: A capability is removed from a running conversation

- **GIVEN** a Claude Code conversation started with `advisor` bridged
- **WHEN** a later turn runs after the advisor was turned off
- **THEN** `advisor` is absent from the tools the CLI lists
- **AND** Smith does not disable the CLI's recorded instructions

### Requirement: Skills reach a harness as activation results

A harness session SHALL be able to find the skills it is authorized to use
through `registry.search` and SHALL receive a skill's instructions as the
tool result of activating it. Activation MUST apply the same trust and
content-hash checks as in a native session.

#### Scenario: A skill is activated from a harness

- **GIVEN** a trusted skill the session is authorized to use
- **WHEN** the CLI activates it through the bridge
- **THEN** the tool result contains the skill's instructions

#### Scenario: A skill's content changed

- **GIVEN** a skill whose bytes no longer match its pinned hash
- **WHEN** the CLI activates it through the bridge
- **THEN** activation fails with the existing trust error
- **AND** no instructions are returned

### Requirement: Interrupting a harness turn stops the CLI

Smith SHALL stop the CLI and every process the CLI started when a harness
turn is cancelled, when its session shuts down, and when Smith's own process
ends for any reason, including a signal Smith cannot catch. Stopping only
the process Smith launched is not sufficient where that process is a
launcher around the agent. After the turn's terminal event, nothing the CLI
started may still be running the turn.

A headless run that receives an interrupt, termination, or hangup signal
SHALL cancel its turn, clean up as a cancelled turn does, and report the
existing cancelled outcome.

#### Scenario: The CLI is a launcher around the agent

- **GIVEN** a harness whose executable starts the agent as a child process
- **WHEN** the user interrupts a turn in flight
- **THEN** the launcher and the agent both exit
- **AND** the bridge connection closes

#### Scenario: Smith's process is killed

- **GIVEN** a harness turn in flight whose CLI has started a child process
- **WHEN** Smith's process is killed without a chance to clean up
- **THEN** the CLI and its child stop

#### Scenario: A headless run is interrupted from the terminal

- **GIVEN** a headless harness run with a bridged call in flight
- **WHEN** the run receives an interrupt signal
- **THEN** the turn is cancelled and saved as cancelled
- **AND** the CLI and everything it started stop
- **AND** the run reports the cancelled outcome and leaves no bridge
  directory behind

#### Scenario: An interrupted turn stops calling Smith

- **GIVEN** a harness turn interrupted during a bridged call
- **WHEN** the turn has reached its terminal event
- **THEN** no further bridged call arrives from that CLI

### Requirement: Capability bridge tests

Smith SHALL provide deterministic capability-bridge tests that run without
an installed CLI or a network, through a scripted CLI stand-in that reads
the server declaration from the arguments Smith built, launches the declared
command, and reaches the endpoint over the real relay. They SHALL cover at
least:

1. the bridged set for a root session, a child, a read-only posture,
   delegation off, no advisor, no skills, and everything denied;
2. the exact CLI arguments for each harness with its own tools on and off,
   bridged and not, on a first turn and on a resume;
3. a bridged call that is approved, one that is denied, and one refused
   under headless approval;
4. a call abandoned before its approval is answered;
5. a turn interrupted during a bridged call, and a process exit during one;
6. a call arriving after its turn ended;
7. `agent` spawn, wait, acknowledged result, and unread result delivered by
   a child-completion turn on the harness;
8. a remote MCP tool, a skill activation, and an advisor call;
9. echo suppression beside a tool the CLI ran itself;
10. the relay exiting, the CLI exiting, and the CLI never connecting;
11. an interrupt reaching a process the stand-in started as its own child.

The fixtures MUST assert tool invocation counts, approval requests,
canonical history, journal events, usage attribution, provider request
counts, and what remains on disk, rather than only presentation. CLI output
used by the normalizer tests and the stand-in MUST be recorded from the real
CLIs, not written by hand. Every such test MUST run under a deadline.

Tests that need an installed CLI SHALL be opt-in and excluded from the
default gate, and SHALL include a check, needing no model call, that each
installed CLI still accepts the arguments Smith passes and still prints the
recorded output shapes.

#### Scenario: Bridge matrix runs without a CLI

- **GIVEN** a machine with neither `claude` nor `codex` installed
- **WHEN** the default test gate runs
- **THEN** every listed case runs against the stand-in and passes

#### Scenario: A wrong server declaration

- **GIVEN** a change that makes Smith declare the server with a malformed
  argument
- **WHEN** the stand-in tests run
- **THEN** they fail, because the stand-in could not launch the relay from
  what Smith passed

#### Scenario: An abandoned call is observed, not assumed

- **GIVEN** the stand-in abandons a call that is waiting for approval
- **WHEN** the approval is then granted
- **THEN** the test observes that the tool's invocation count is zero

#### Scenario: A CLI release changes its output

- **GIVEN** an installed CLI whose MCP call output no longer matches the
  recorded shape
- **WHEN** the opt-in drift check runs
- **THEN** it fails naming the CLI, its version, and the changed field

### Requirement: Bridged calls are not reported twice

A harness adapter SHALL NOT emit external tool observation events for a call
the CLI made to the `smith` server, because Smith already records that call
as dispatched. Observation events for tools the CLI ran itself are
unchanged.

#### Scenario: The CLI echoes a bridged call

- **GIVEN** the CLI calls `agent` through the bridge
- **WHEN** the CLI's own output reports that tool use and its result
- **THEN** the adapter emits no external tool events for it

#### Scenario: The CLI runs its own tool in the same turn

- **GIVEN** a harness permitted to run its own tools
- **WHEN** the CLI runs one of them beside a bridged call
- **THEN** that tool is still reported as an external observation
