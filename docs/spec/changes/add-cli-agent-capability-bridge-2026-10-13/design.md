## Context

A harness turn is executed by `ExternalTurnMachine`
(`agent-runtime/src/agent/driver/external_turn.rs`). It hands the whole turn
to an `ExternalAgentBackend` and projects the backend's events onto the
canonical stream. Its module header states what it deliberately does not do:
plan context, call a provider, dispatch tools. Smith's backend
(`smith-runtime/src/cli_agent.rs`) builds one `claude -p` or `codex exec`
invocation per turn and normalizes the output.

Three facts from the pinned runtime (`d2752c1`) shape this design:

- `Session::run_local_tool` already dispatches a host-requested call through
  the full tool path without a provider, but refuses unless the session is
  idle ("a local tool action requires an idle session"). It cannot be called
  while an external turn is in flight.
- `Driver::run_internal_turn` always constructs the native `TurnMachine`.
  A child-completion turn on a harness session would therefore call the
  stand-in provider with a `cli/...` model id. This is unreachable today only
  because the CLI cannot call `agent`.
- `AdvisorTool` renders `session.history()`. For an external turn, history
  holds the user input and, after completion, one assistant message. The
  turn's in-flight work is not there.

## Goals / Non-Goals

- Goals:
  - The CLI's model can use `agent`, `advisor`, Smith-declared MCP servers,
    plugin and module tools, and skills, with the semantics they have in a
    native session.
  - One owner per tool call. A bridged call is dispatched, approved, and
    recorded by Smith; a CLI-run tool stays an observation.
  - The CLI's session continuation is untouched: Smith supplies the bridge on
    every invocation, so `--resume` and `exec resume` keep working.
- Non-Goals:
  - Replacing the CLI's file and shell tools, or approving them.
  - Making the CLI's context Smith's: no context planning, compaction, or
    system-prompt replacement.
  - A general "Smith as an MCP server" product surface. The endpoint exists
    for one turn of one CLI process.

## Decisions

### The CLI calls Smith over MCP

Both CLIs accept MCP servers per invocation: `claude --mcp-config` and
`codex -c mcp_servers.<name>...`. Nothing else is common to both, and MCP is
the only mechanism by which either CLI's model can call something outside
the CLI. The server is named `smith`.

Alternative considered: make the CLI a model again by promoting the
`command-jsonl` bridge. Rejected for the reason the harness change exists:
it replays the whole history every turn and reads tool intent out of prose.

### The bridged set is a rule, not a list

Bridged = the session's authorized tool view, minus:

- the core coding tools (`capability_limits::CORE_TOOL_NAMES`),
- the todo-planning tool (both CLIs plan for themselves),
- the goal tools (goal continuation on a harness is out of scope; see
  Resolved Questions).

Everything else is included by construction, so a tool added later (the
project-memory tool, a new module) is bridged without touching this code,
and every existing gate keeps working because it already shaped the view:
`delegation_eligible`, `advisor_eligible`, `questionnaire_eligible`,
read-only posture, child surface, child tool scope, `capabilities.allow`
and `capabilities.deny`.

Three tools exist only to support other bridged capabilities and must not
keep the set non-empty alone, or every harness turn would start an endpoint:

- `registry.search` and `registry.activate` are the runtime's discovery
  bootstraps; the `/capabilities` catalog hides them, and this design does
  not rely on a deny pattern removing them. Because every bridged tool is
  listed outright (see below), they serve only skills here, so they are
  bridged only when at least one skill is authorized.
- `artifact.read` is bridged only beside another tool: a bounded bridged
  result points at it.

### Agent Runtime owns dispatch

`ExternalTurnRequest` gains a dispatch handle:

```rust
pub struct ExternalToolBridge { /* channel into the turn machine */ }

impl ExternalToolBridge {
    /// The specs the host may offer for this turn.
    pub fn specs(&self) -> Vec<ToolSpec>;
    /// Runs one call through the ordinary tool path.
    pub async fn call(&self, name: String, arguments: Value)
        -> Result<ToolResultBlock, RuntimeError>;
}
```

`ExternalTurnMachine::run` selects over backend events and bridged calls. A
call goes through the same prepare, authorize, approve, execute, process,
and commit steps as `prepare_and_run_local`, attributed to the external
turn, and emits the ordinary tool events. Smith never invokes a `Tool`
object itself: that would skip approval, the journal, and the checkpoint.

The host passes the excluded names when it builds the session; the runtime
does not know which tools a CLI replaces.

### Dispatch resolves against the authorized view

A native call must be in the frozen activation epoch. Activation exists to
budget Smith's prompt, and an external turn has no Smith prompt. Bridged
calls therefore resolve against the authorized view, and `specs()` returns
all of it. `ActivationBudget` does not apply.

Alternative considered: list only active tools and rely on
`notifications/tools/list_changed` after `registry.activate`. Rejected: it
depends on each CLI honouring the notification mid-turn, and it buys nothing
when the context is not Smith's.

### Bridged calls enter canonical history

When a bridged call arrives, the text observed so far is flushed as an
assistant message carrying the call, and the result commits as a tool
message, exactly the shape a native round leaves. Reasons:

- the child-outcome acknowledgement keys on a committed tool result
  (`child-agents`: "A child result the model already read is not delivered
  again");
- a later switch to a native model keeps the child and advisor results;
- recovery sees a checkpointed call, not an invisible one.

CLI-run tools stay out of history, as today. A failed turn keeps the rounds
that committed before it failed, as a native turn does.

### Internal turns run on the harness

On a session with an external backend, `run_internal_turn` builds an
`ExternalTurnMachine`. `InternalTurnInput.content` is already bounded text;
it is sent as the prompt of a resumed CLI session, prefixed as a Smith
delivery.
Admission, attribution (`delegation.child-completion`), and outcome
consumption are unchanged. No user message is added to Smith's canonical
history; that the CLI's own conversation records the delivery as a prompt is
unavoidable and is the CLI's history, not Smith's.

### Transport: a Unix socket and a stdio relay

Smith binds a Unix socket in a fresh mode-`0700` directory, verifies the
peer's uid on accept, and serves MCP over it. The CLI is told to launch
`smith mcp-relay <socket>`, a hidden subcommand that copies bytes between
its stdio and the socket and does nothing else.

| | Claude Code 2.1.296 | Codex 0.162.1 |
| --- | --- | --- |
| declare | `--mcp-config '{"mcpServers":{"smith":{"command":…,"args":["mcp-relay",…]}}}'` | `-c mcp_servers.smith.command="…"`, `-c mcp_servers.smith.args=[…]` |
| run without the CLI's own prompt | `--allowedTools mcp__smith` | `-c mcp_servers.smith.default_tools_approval_mode="approve"` |
| own tools off | the allow rule above in place of today's empty list | `--sandbox read-only`, unchanged |
| own tools on | `--permission-mode acceptEdits` plus the allow rule | `--sandbox workspace-write`, unchanged |
| tool-call timeout | `MCP_TOOL_TIMEOUT` (milliseconds) in the child environment | `-c mcp_servers.smith.tool_timeout_sec=…` |
| wait for the server to start | `MCP_TIMEOUT` (milliseconds); 30 s by default | `-c mcp_servers.smith.required=true` and `.startup_timeout_sec=…` |
| guidance | MCP server `instructions` | MCP server `instructions` |

Every cell was run against the installed CLI in the spike; see Spike
Findings. Without its row two, Claude Code refuses the call with a
`permission_denied` event and Codex with "MCP tool call requires approval,
but approval policy is never", in both sandbox modes.

Codex does not hold a turn for an MCP server that is still starting: a
server that took 15 seconds was simply absent from that turn, whatever its
`startup_timeout_sec`. Marking the server `required` makes Codex wait, and
fail the turn (exit status 1, "required MCP servers failed to
initialize") if it never starts. Smith marks the bridge required, so a
Codex turn either has Smith's tools or fails with a reason. Claude Code
waits up to its connect timeout and otherwise runs the turn without the
server; the not-connected diagnostic covers that case.

With the CLI's own tools off, Claude Code's built-ins behave with
`--allowedTools mcp__smith` exactly as they do with today's empty list:
read-only commands and `Read` run, `Write` is refused. The allow rule widens
nothing but the `smith` server.

Alternative considered: streamable HTTP on `127.0.0.1` with a bearer token.
It works with both CLIs, but any local user can reach the port, the token
must be kept out of argv, and it adds an HTTP server stack to a workspace
that has only an HTTP client. The spike ran the relay from both CLIs,
including Codex under `read-only`, so the fallback is not needed.

Nothing secret appears in argv: the socket path is not a credential, because
the directory mode and the peer check are the access control.

The socket directory is created with `mkdtemp` under the shortest of
`$XDG_RUNTIME_DIR`, `$TMPDIR`, and `/tmp`, never under Smith's state
directory: macOS bounds a socket path at 104 bytes, and the spike hit that
bound with a 106-byte path.

### Names pass through unchanged

Both CLIs normalize a tool name for their model themselves (`registry.search`
is shown as `mcp__smith__registry_search`) and send the original name back
in `tools/call`. Neither enforced a 64-character limit: a 75-character name
was listed and called on both. The bridge therefore lists Smith's names as
they are. It only checks that no two names are equal after the CLIs'
normalization (anything outside `[A-Za-z0-9_-]` becomes `_`) and that each
is at most 128 bytes. A tool that fails either check is omitted and
reported; it is never renamed or truncated.

### The CLI's echo is suppressed

Both CLIs report their MCP calls in their own stream, and the normalizers
drop the ones addressed to `smith`, because Smith already emitted the
dispatched call:

- Claude Code prints an assistant `tool_use` part named `mcp__smith__<tool>`
  and later a `tool_result` part that carries only `tool_use_id`. The
  normalizer remembers the ids it suppressed so the result is dropped too.
- Codex prints `item.started` and `item.completed` with
  `"type":"mcp_tool_call"` and `"server":"smith"`.

Claude Code defers MCP tools and calls its own `ToolSearch` before first
using one. That call stays a CLI-run observation.

### Skills use the registry tools

`registry.search` is bridged as is. `registry.activate` on a skill id runs
the existing trust and hash check and returns the instruction body in the
tool result when the turn is external. A separate Smith-only `skill` tool
was rejected: it would fork the trust check.

### The advisor reads an observation log

History already carries flushed text and bridged calls. What is missing is
CLI-run tool activity. The runtime keeps a bounded log of the in-flight
external turn's tool observations and exposes it to tools through the
session view; `AdvisorTool` merges it into the transcript in order and
labels each entry as run by the CLI.

### Calls are serialized in the first version

Bridged calls run one at a time in arrival order. A serial order satisfies
side-effect-aware scheduling trivially. The cost is that two parallel `wait`
calls take the sum of their waits; `spawn` returns immediately, so fan-out
is unaffected.

Both CLIs already send calls to one server one after another, even when the
model asked for three in one step. Claude Code overlaps them only for tools
that carry `readOnlyHint`, which the bridge does not set. The endpoint still
queues concurrent requests safely, since that is the CLI's choice, not
Smith's.

### Guidance travels in the MCP server's instructions

The fragments for bridged capabilities are joined into the `instructions`
field of the MCP `initialize` result. Both CLIs show that text to their
model, it needs no prompt text in argv, and it disappears with the server.
Claude Code cuts instructions off a little past 4,000 characters, so the
assembled guidance is capped at 4,000 and a test holds the largest bridged
set under the cap. Smith's fragments are a few lines each.

Codex reads the instructions again on every invocation. Claude Code does
not: it records its system prompt, MCP instructions included, on a
conversation's first request and reuses the record on every resume. Turning
that off (`--system-prompt-snapshot off`) makes the guidance track the
bridged set exactly, but in the spike it cost about 17,000 cache-write
tokens on every turn against 67 with the record on, because the rest of
Claude Code's prompt is re-rendered too. The record stays on.

The consequence is accepted: on Claude Code, guidance is fixed when a
conversation starts. If the bridged set changes later, the tool list is
still exact, because Claude Code tells its model which tools appeared and
disappeared, but the guidance text stays as first delivered until the CLI
compacts or a new conversation starts.

### An abandoned call never executes

A bridged call can block on an approval or a child wait for a long time.
The two CLIs abandon a call differently when their own timeout fires:
Claude Code sends `notifications/cancelled` with the request id; Codex
sends nothing and keeps the connection open, so Smith could not tell.

Smith therefore never lets the CLI time out first. It sets both timeouts to
one week (`MCP_TOOL_TIMEOUT=604800000`, `tool_timeout_sec=604800`; a
75-second call survived on both). What remains is unambiguous: a
`notifications/cancelled` for the request, or the connection closing. On
either, Smith cancels the pending interaction and the call, and an approval
answered afterwards executes nothing.

### Interrupting a turn kills the CLI's process group

Smith kills the CLI with `SIGKILL` when a turn is cancelled. For Claude
Code, a single native process, the bridge connection closed in the same
millisecond. For Codex, whose `codex` is a Node launcher around a native
binary, the launcher died and the native process carried on: it finished
its 45-second call and its turn 46 seconds after the "kill". Today that
means an interrupted Codex turn keeps running commands. With the bridge it
would also keep calling Smith.

The adapter starts the CLI in its own process group and kills the group.
The relay is in that group, so the connection closes with it. The endpoint
refusing calls once the turn has ended is the second line, not the first.

The CLI's separate process group no longer receives the terminal's signals,
and Smith can die without running destructors. A hidden `smith
harness-supervise -- <program> [args...]` therefore leads that group and starts
the CLI inside it, inheriting stdout and stderr directly. Smith holds the
write end of the supervisor's stdin pipe for the turn; EOF, including after
Smith crashes or receives SIGKILL, makes the supervisor kill its entire group.
SIGINT, SIGTERM, and SIGHUP sent to the supervisor also kill the group.
If the CLI exits first, the supervisor propagates its exit status.

Headless SIGINT, SIGTERM, and SIGHUP cancel through the user-interrupt path,
save and shut down normally, and report the existing cancelled result with
exit status 1; a second signal exits immediately with status 1.

The relay and the supervisor are both subcommands of the `smith` binary, so
the host supplies that binary's path (`RuntimeRequest::smith_executable`);
smith-runtime does not guess it from `current_exe()`, which in any other
process embedding the runtime (a test binary, a stand-in) names a program
without those subcommands. Without a path, the turn runs unbridged and the
CLI is started directly, with a one-time warning that it may outlive Smith.

Found while testing: Agent Runtime dropped the result of a tool cancelled
with its turn whenever an output processor was installed, as Smith's
artifact offloader is, so neither history nor the event stream recorded it.
The runtime change now commits the cancellation result, bypassing the
remaining processors, before the turn's terminal event (runtime task 1.8).

Not addressed here: a headless shutdown does not close Smith-declared stdio
MCP servers, so one busy in a call can outlive Smith briefly. That predates
this change and applies to native sessions too.

### Codex resume takes its flags before the subcommand

`codex exec resume` in 0.162.1 accepts neither `-C` nor `--sandbox`, so the
arguments Smith builds today (`exec resume <id> --json … -C <dir> --sandbox
<mode>`) exit with status 2 and every second turn of a Codex session fails.
`codex exec <flags> resume <id> <prompt>` works, and the server declared
with `-c` is loaded again on the resumed turn. The adapter builds that
order.

## Risks / Trade-offs

- **CLI releases can change what the spike recorded.** Every flag, key, and
  output shape this design uses was observed on one version of each CLI.
  The drift check in the test plan exists to catch a release that changes
  them.
- **The handshake Claude Code starts with.** Before `initialize` it sends a
  `server/discover` probe for protocol `2026-07-28`. The spike's server
  answered "method not found" and Claude Code fell back to `initialize`
  (`2025-11-25`). `rmcp` 3.1.2 instead answers the probe and continues
  without `initialize`. That path is untested here, so the endpoint is run
  against both real CLIs as soon as it exists (task 4.3), before anything
  is built on it. The fallback is to refuse the probe as the spike did.
- **Guidance is fixed per Claude Code conversation.** See "Guidance travels
  in the MCP server's instructions".
- **Tool-list size.** Listing every authorized tool can put many schemas in
  the CLI's context. This is the same cost as declaring those servers in the
  CLI directly, and `capabilities.deny` narrows it.
- **Socket path length.** Unix socket paths are bounded at 104 bytes on
  macOS. The directory is created under a short base, and a path that still
  does not fit fails the turn with the reason.
- **Two conversations.** The CLI's own history contains bridged results
  Smith also holds. They can diverge only if the CLI compacts; Smith's copy
  is canonical for Smith.

## Spike Findings

Run on 2026-10-13 against Claude Code 2.1.296 and Codex 0.162.1 with a
throwaway MCP server. The recorded output is in
`crates/smith-runtime/tests/fixtures/cli_bridge/`, with a README naming the
invocation behind each file.

| Question | Claude Code | Codex |
| --- | --- | --- |
| Loads a server declared per invocation | yes, inline `--mcp-config` | yes, `-c mcp_servers.<name>.command` and `.args` |
| Runs its tools without asking | only with `--allowedTools mcp__<server>` | only with `default_tools_approval_mode="approve"` |
| Server instructions reach the model | yes, cut off past about 4,000 characters | yes, 8,000 characters arrived whole |
| Instructions re-read on resume | no, recorded at the first request | yes |
| Dotted and 75-character tool names | listed and called | listed and called |
| Calls to one server | one at a time unless `readOnlyHint` | one at a time |
| When its tool timeout fires | `notifications/cancelled` | nothing sent, connection stays open |
| A 75 s call with a one-week timeout | returned | returned |
| Stdio relay to a Unix socket | works | works under `read-only` |
| Resume reloads the server | yes | yes, with flags before `resume` |
| Resume without the server | tools are gone, and the model is told | tools are gone |
| `SIGKILL` during a call | connection closes at once | native process survives and finishes the turn |
| Coexists with the user's own servers | yes | yes |
| Waits for a server that starts slowly | yes, up to `MCP_TIMEOUT` (30 s default) | only when the server is `required` |
| Smith's own endpoint and `smith mcp-relay` | tools listed, call returned, guidance visible | the same |
| Keeps running after its terminal event | about 0.7 s | about 1.5 s |

What a CLI does after its terminal event (hooks, telemetry) is cut short,
because Smith kills the group as soon as the turn has its result. That was
already true of Claude Code and of Codex's launcher; it is now true of the
Codex agent process too. Codex still resumed its thread on the next turn in
a live check through the `smith` binary. Waiting for the CLI to exit by
itself would add that time to every turn and is left as a follow-up.

Two defects in the existing harness adapter surfaced and are fixed under
this change because its scenarios cannot pass without them: the Codex
resume arguments, and the Codex process surviving an interrupt. Both are
described in Decisions.

Also noted: inside a cmux terminal, `claude` and `codex` on `PATH` are
launcher scripts, another reason to kill the process group and not one pid.

## Test Plan

Most of this change is behaviour at a process boundary Smith does not
control, so the tests are layered to keep the real CLIs out of the gate
while still exercising the real wire path.

| Layer | Where | What it proves | Needs a real CLI |
| --- | --- | --- | --- |
| 1. Runtime conformance | Agent Runtime `tests/external_agent.rs`, with its scripted backend calling the handle | dispatch path, history order, cancellation, late-call refusal, internal turns on the backend, replay | no |
| 2. Bridge units | `smith-runtime` unit tests | bridged-set rule, support-tool rule, name mapping, argv per arm, guidance assembly, echo suppression | no |
| 3. Endpoint | `smith-runtime` tests driving the socket with the workspace's existing MCP client | MCP conformance of the server, peer check, permissions, lifetime, malformed frames, abandoned calls | no |
| 4. Stand-in end to end | `smith-cli` tests with the built `smith` binary | argv, relay, socket, dispatch, approval, events, history, and persistence together | no |
| 5. Surfaces | `smith-tui` render tests, headless golden output | one row per call, approvals, diagnostics, TUI and headless equivalence | no |
| 6. Live | opt-in, ignored by default, like `mcp_live.rs` | the installed `claude` and `codex` still behave as the spike recorded | yes |

### The scripted CLI stand-in

Layer 4 rests on one new test asset, extending the existing
`fake_claude_cli` helper. It is an executable that plays either CLI:

- records the argv and environment Smith gave it;
- reads the server declaration from that argv in the CLI's own dialect and
  launches the declared command exactly as declared, so the test goes
  through the real `smith mcp-relay` and the real socket;
- runs a script: list tools, call a tool, call two at once, abandon a call,
  exit mid-call, never connect;
- prints the CLI's JSON stream, including its echo of each MCP call.

Because the stand-in parses the declaration instead of being told the socket
path, a wrong flag or a malformed inline config fails layer 4, not only the
live suite.

Headless cannot hold an approval or a question open: its brokers answer at
once. The layer 4 tests for a delayed approval, an abandoned approval, and a
questionnaire round trip therefore run an in-process terminal host inside
the stand-in example (`examples/cli_stand_in/approvals.rs`), built through
`smith_runtime`'s public host API with brokers the test controls, and given
the real `smith` binary for the relay and supervisor. The endpoint, relay,
and stand-in CLI are the real ones. The example observes the session only
through the Smith client event protocol, as the architecture test requires
of `smith-cli` code.

### Recorded output, not invented output

The spike saves the real JSON lines each CLI prints around an MCP call, a
CLI-run tool, an abandoned call, and a failed turn as fixtures. The
normalizer tests and the stand-in's output are built from those files. A
shape Smith guessed is not a test.

### Effects, not presentation

Following the delegation parking tests, a bridge test asserts what happened,
not what was drawn: which tool ran and how many times, the approval that was
requested and its answer, canonical history, journal events, usage
attribution, provider request counts, and what is on disk after the turn.
Each safety property has a negative test that observes the side effect:

- a call abandoned before approval leaves the tool's invocation count at
  zero even when approval arrives later;
- a late call to a finished turn's endpoint dispatches nothing;
- suppression drops the echo of a bridged call and keeps a CLI-run tool
  reported in the same turn;
- a denied capability is absent from the list and refused when called by
  name anyway;
- an interrupted turn is not replayed and its bridged call does not run
  again.

### Failure injection

Layers 3 and 4 cover the ways the boundary breaks: the relay exits mid-call,
the CLI is killed mid-call, the socket directory is removed under a running
turn, a frame is malformed or oversized, a call names an unknown tool, the
arguments fail the schema, two calls arrive together, and the CLI never
connects. Each ends in a tool error or a failed turn with a reason, never a
hang: every such test runs under a deadline.

### Live suite and drift

The live tests run one flow per CLI: spawn a provider child and read its
result, call a remote MCP tool behind an approval, activate a skill, call
the advisor, resume the CLI session and confirm the bridge is still there.
A separate canary only checks that each installed CLI still accepts the
flags and keys Smith passes and still prints the recorded shapes, so a CLI
release that changes them is caught without running a model.

### Traceability

Every scenario in the deltas maps to at least one named test in layers 1 to
5. `tasks.md` section 8 closes only when that table, `traceability.md` in
this change folder, has no empty row.

## Migration Plan

Additive. No configuration changes meaning. Ship behind the runtime pin
bump; a Smith built against the old pin has no bridge.

## Resolved Questions

Decided by the owner on 2026-10-13:

- **Transport.** The Unix-socket relay. Loopback HTTP stays the fallback
  only if the spike finds a CLI that cannot launch the relay.
- **No off switch.** A root harness session always has something to bridge
  (the questionnaire tool at least in an interactive session; a headless
  run offers no questionnaire, natively or bridged, because nobody can
  answer), so the bridge is on by default for
  every root `cli/*` session. It is turned off by denying every tool and
  skill id on that profile (`tool:*` and `skill:*`; task 4.6 confirms the
  exact patterns), which costs nothing there because the CLI never uses
  Smith's core tools.
- **Goal tools stay out.** Goal continuation on a harness session is a
  separate change.

## Open Questions

- **Parallel calls.** Whether to schedule bridged calls through the batch
  executor instead of serializing them. Not needed for the first version.
