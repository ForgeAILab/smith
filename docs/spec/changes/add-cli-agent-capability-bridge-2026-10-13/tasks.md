---
created_at: 2026-10-13T22:40:43Z
updated_at: 2026-10-15T04:48:58Z
completed_at:
---

Tests are written with the code they cover, in the layer `design.md` names
for them. A task is done when its tests exist and pass, not when the code
compiles.

## 1. Spike: verify what each CLI does

No production code. Record the findings in `design.md` and revise the
decisions they contradict before starting section 2.

Done 2026-10-13 against Claude Code 2.1.296 and Codex 0.162.1. Findings
are in `design.md` under Spike Findings; recordings are in
`crates/smith-runtime/tests/fixtures/cli_bridge/`.

- [x] 1.1 A throwaway stdio MCP server with one slow tool and one fast tool.
  Confirm `claude -p` loads it from inline `--mcp-config`, that
  `--allowedTools mcp__<server>` runs its tools without a prompt with and
  without `--permission-mode acceptEdits`, that the CLI's permission-gated
  built-ins are still refused with only `mcp__<server>` allowed, and that
  `--resume` reloads the server.
- [x] 1.2 The same server through `codex exec` with `-c mcp_servers.<name>`
  overrides. Confirm the exact keys, whether an MCP call runs without
  approval in `exec` mode under `read-only` and `workspace-write`, and that
  `exec resume` reloads it.
- [x] 1.3 For both: the tool-call timeout setting and its default, what the
  CLI sends when it abandons a call, and whether parallel calls arrive.
- [x] 1.4 For both: whether MCP server `instructions` reach the model, and
  how each reports an MCP call in its JSON stream (the fields the echo
  suppression will key on). For Claude Code: whether appended guidance from
  the first turn persists on `--resume` under the default
  `--system-prompt-snapshot`, and what turning it off costs.
- [x] 1.5 Tool-name constraints each CLI enforces, with a dotted name and a
  name over 64 characters.
- [x] 1.6 Launch the throwaway server through a stdio relay to a Unix
  socket from each CLI, including Codex under `read-only`. Confirm the
  relay transport, or fall back to loopback HTTP and update `design.md`.
- [x] 1.7 Save the real JSON lines each CLI printed around an MCP call, a
  CLI-run tool, an abandoned call, and a failed turn as fixtures, with the
  CLI version in the file name.

## 2. Test assets

Built before the code they test.

- [x] 2.1 The scripted CLI stand-in: records argv and environment, parses
  the server declaration in each CLI's dialect, launches the declared
  command, runs a script of MCP steps, and prints output built from the
  1.7 fixtures. Scripts: list, call, two calls at once, abandon, exit
  mid-call, never connect.
- [x] 2.2 A counting tool for tests whose invocation count and arguments are
  observable, so "nothing executed" is asserted, not inferred. (The
  stand-in's `--counting-mcp-server` mode: each call appends a line to a
  log file.)
- [x] 2.3 The traceability table: one row per scenario in the deltas, the
  layer that covers it, and the test name. Empty rows are allowed until
  section 8. (`traceability.md` in this folder, 65 scenarios.)
- [x] 2.4 A deadline wrapper for every bridge test, so a hang fails with the
  step it stopped at. (`tests/cli_contract/support.rs`. The stand-in is an
  example target of `smith-cli`: a run narrowed with `--test cli_contract`
  needs `cargo build -p smith-cli --example cli_stand_in` first.)

## 3. Agent Runtime

On a new branch from `d2752c1`. Each item lands with its layer 1 tests in
`tests/external_agent.rs`.

Landed as `db8dff2` on the runtime branch `feat/smith-external-tool-bridge`
(one commit on `d2752c1`), under the runtime's own spec change
`add-external-tool-dispatch-2026-10-13`. Its gate passes there:
`cargo fmt --check`, clippy with all features and warnings denied, and
1,304 tests across the workspace (2026-10-14). The
checkpoint schema version and transition revision are unchanged, so saved
sessions still resume.

- [x] 3.1 `ExternalToolBridge` on `ExternalTurnRequest`: `specs()` over the
  authorized view less the host's excluded names; `call()` into the turn.
  Tests: the listing honours deny patterns and exclusions; a call to an
  unlisted or denied name is refused without dispatch.
- [x] 3.2 `ExternalTurnMachine` selects over backend events and bridged
  calls; a call runs the prepare, authorize, approve, execute, process, and
  commit path and emits the ordinary tool events. Tests: event order and
  attribution; an approval round trip; a denied approval returns a tool
  error and the turn continues; no provider request is made.
- [x] 3.3 Flush observed text into an assistant message carrying the call
  before the result commits. Tests: history order for text, call, text; a
  failed turn keeps committed rounds; two calls in one turn.
- [x] 3.4 Cancelling a call from the backend side withdraws its pending
  interaction; the handle refuses calls after the terminal event. Tests:
  abandon before approval, then approve, and the counting tool stays at
  zero; turn interruption cancels an in-flight call; a late call is refused.
- [x] 3.5 Internal turns on an external session run on the backend, with
  the internal input as delivery text and no user message appended. Tests:
  child completion on an external session calls the backend and not the
  provider; history has no fabricated user message; replay does not invoke
  the backend again.
- [x] 3.6 A bounded observation log of the in-flight external turn's
  CLI-run tools, readable by tools through the session view. Tests: order
  relative to bridged calls; the bound; cleared at the terminal event.
- [x] 3.7 `registry.activate` returns a skill's instructions in the result
  when the turn is external. Tests: instructions returned; a changed skill
  is refused with the existing trust error.
- [x] 3.8 Recovery: a process exit during a bridged call finalizes the turn
  as interrupted and does not run the call again. Test at the checkpoint
  level.
- [x] 3.10 The CLI's continuation identity is stored redaction-safe. It was
  stored as sensitive state, which Smith's plain session file drops, so
  without a checkpoint key every `--resume` started a new CLI
  conversation. Found by the layer 4 resume tests; the same id is already
  published in the event journal.
- [x] 3.11 A tool call whose request was emitted always commits a result
  and a completion event before the turn ends. With an output processor
  installed (Smith's artifact offloader is one), cancelling a turn while a
  tool runs dropped the cancellation result before commit, so neither the
  history nor the event stream had it. Found by the layer 4 interrupt test;
  native provider turns share the path. Regression tests with a no-op and
  a blocking processor, for a provider turn and a bridged call. (Runtime
  task 1.8; `external_agent::execution::output_processing`, 11 tests.)
- [x] 3.9 Update the runtime's own spec and the `external.rs` and
  `external_turn.rs` module docs, which currently say an external turn
  dispatches no tools. Run the runtime gate, push the branch.
  (`feat/smith-external-tool-bridge` at `db8dff2`.)

## 4. Smith: bridge endpoint

Layer 2 and 3 tests land with each item.

- [x] 4.9 The host supplies the Smith executable used for the relay and the
  supervisor; smith-runtime no longer takes it from `current_exe()`, which
  names the wrong program in any process that embeds the runtime. Without
  it the turn runs unbridged and the CLI starts directly.
- [x] 4.1 Bump the six Agent Runtime pins and the lockfile. (`db8dff2`.)
- [x] 4.2 Enable `rmcp`'s server feature for `smith-runtime`; confirm
  `cargo deny` and the dependency policy still pass.
- [x] 4.3 A crate-private bridge module: bind a Unix socket in a fresh
  owner-only directory under the shortest of `$XDG_RUNTIME_DIR`,
  `$TMPDIR`, and `/tmp`, check the peer uid, serve `initialize` (with
  the guidance as `instructions`), `tools/list`, and `tools/call` from an
  `ExternalToolBridge`, honour `notifications/cancelled`, and remove the
  directory when the turn ends. Before building on it, the orchestrator
  runs the endpoint against the installed `claude` and `codex` once:
  Claude Code opens with a `server/discover` probe that `rmcp` answers
  differently from the spike's server. Tests, driving the socket with the workspace's MCP client:
  list and call round trip; directory and socket modes; directory gone
  after the turn; a second connection after the terminal event dispatches
  nothing; a bind failure fails the turn before any spawn; an over-long
  socket path fails with the reason. (Done: the endpoint refuses the
  probe itself, as the spike's server did. Both real CLIs listed the
  tools, called one, and saw a marker placed in the instructions, alone
  and at the same time against one endpoint.)
- [x] 4.4 Peer check. Test the refusal path through an injected credential
  source, and against a real second uid where the platform and CI allow it.
  (Injected source only; no second uid is available here.)
- [x] 4.5 Names pass through unchanged; a tool whose name collides with
  another after the CLIs' normalization, or exceeds 128 bytes, is omitted
  and reported once per session. Table tests: dotted names, `mcp__`
  names, a name at and over the limit, two names that collide, stability
  across runs. (Checks and tests done in `cli_bridge.rs`; reporting an
  omission once per session is wired with the endpoint.)
- [x] 4.6 Pass the excluded names (core, todo planning, goal) when the
  session is built, and apply the support-tool rule for `registry.*` and
  `artifact.read`. Tests: the bridged set for root, child, read-only
  posture, delegation off, no advisor, no skills, and everything denied;
  confirm and document the deny patterns that empty the set. (Selection
  The runtime's handle lists only authorized tools, so the set is built
  from it directly. `capabilities.deny = ["tool:*", "skill:*"]` on the
  profile empties the set and leaves the invocation unchanged.)
- [x] 4.7 `smith mcp-relay <socket>`: a hidden subcommand that copies stdio
  to and from the socket. Tests: byte-exact relay in both directions, exit
  when either side closes, absent from help and command discovery.
- [x] 4.8 Failure injection at the endpoint: malformed frame, oversized
  frame, unknown tool, schema-invalid arguments, peer closing mid-call,
  socket directory removed under a running turn. Each yields a tool error
  or a failed turn with a reason, under a deadline.

## 5. Smith: harness adapters

- [x] 5.1 Split `cli_agent.rs` by responsibility (invocation, Claude
  normalizer, Codex normalizer, selection) before adding to it; keep module
  path and visibility. Existing tests move unchanged.
- [x] 5.2 Claude Code invocation: inline `--mcp-config` declaring the
  relay, `--allowedTools mcp__smith`, `MCP_TOOL_TIMEOUT` of one week in
  the child environment, and no `--system-prompt-snapshot` flag. Argv
  tests for every arm: own tools on and off, bridged and not, first turn
  and resume; the not-bridged arms equal today's argv exactly.
- [x] 5.3 Codex invocation: `-c mcp_servers.smith.command`, `.args`,
  `.default_tools_approval_mode="approve"`, and `.tool_timeout_sec` of
  one week, with the same argv test matrix.
- [x] 5.3e The CLI cannot outlive Smith. Starting the CLI in its own
  process group (5.3b) means a terminal's Ctrl-C or hangup reaches Smith
  and not the CLI, and Smith has no signal handling, so the CLI was left
  running. Start the CLI under a hidden `smith harness-supervise` that
  leads the group and kills it when its pipe from Smith closes, which
  covers Smith ending for any reason. Black-box tests of the supervisor,
  including the holder of the pipe being killed. (`harness_supervise::`
  contract tests; `smith-cli` gains a Unix-only `nix` dependency already
  in the lockfile.)
- [x] 5.3f A headless run handles interrupt, termination, and hangup
  signals: it cancels the turn, cleans up, and reports the existing
  cancelled outcome with exit status 1. (`harness_supervise::` and
  `bridge_failures::` contract tests; `docs/headless-protocol.md`.)
- [x] 5.3d When a CLI exits without a terminal event, the failure names the
  CLI's own reason. Keep a bounded tail of the CLI's stderr and use it in
  the `Failed` message; a turn that did report a terminal event is
  unaffected. (A required bridge that cannot start is otherwise an
  unexplained failure: Codex prints its reason only on stderr.)
- [x] 5.3c Make each CLI wait for the bridge. Codex starts a turn without
  waiting for a slow MCP server unless the server is marked required, so
  pass `-c mcp_servers.smith.required=true` and
  `-c mcp_servers.smith.startup_timeout_sec=60`; a bridge that never
  starts then fails the turn. Claude Code waits 30 s by default; pass
  `MCP_TIMEOUT=60000`. The stand-in accepts and records both keys and
  plays the required-server failure. Argv and environment tests updated.
- [x] 5.3a Codex resume builds `exec <flags> resume <id> <prompt>`. The
  current order is rejected by Codex 0.162.1 (recorded in
  `codex-0.162.1/stderr/`), so every second Codex turn fails today. The
  existing resume test is corrected with it. Owner-configured `args` now
  precede the prompt on fresh Codex turns too, so they stay `exec` flags
  in both arms. Checked live: two headless turns on `cli/codex/gpt-6.1-sol`
  resumed the same thread.
- [x] 5.3b Start the CLI in its own process group and kill the group on
  cancellation and drop. Test with a launcher that starts a child which
  outlives it: after an interrupt neither is running, under a deadline.
  The group is also killed at the terminal event, as the launcher alone
  was before; Codex resumed its thread afterwards in the live check.
- [x] 5.4 Suppress each CLI's echo of calls to the bridge server: Claude
  Code by `tool_use` name prefix, remembering the suppressed ids so the
  matching `tool_result` is dropped too; Codex by `mcp_tool_call` server.
  The server name is a parameter so the 1.7 fixtures, recorded against a
  server named `spike`, drive the tests unchanged: the echo is dropped; a
  CLI-run tool in the same stream is kept; `ToolSearch` is kept; another
  server's MCP call is kept.
- [x] 5.5 Assemble guidance from the same eligibility predicates that
  register each tool and serve it as the MCP server's `instructions`.
  Tests: each fragment is present exactly when its tool is bridged; the
  largest bridged set stays within 4,000 characters; nothing of Smith's
  system prompt, project instructions, or memory index is included.
  (Assembly and tests done in `cli_bridge.rs`; serving it is part of 4.3.)
- [x] 5.6 Record whether the CLI connected; emit the diagnostic when a turn
  that offered the bridge ends without a connection.
- [x] 5.7 Layer 4, first pass, with the stand-in and the built `smith`
  binary: one bridged call end to end for each dialect, asserting argv,
  the tool's invocation, events, history, and that nothing remains on disk.

## 6. Smith: capabilities over the bridge

Each item is a layer 4 test file driven by the stand-in.

- [x] 6.1 `agent`: spawn, list, wait, result, follow-up, resume, and stop
  through the bridge against a fake provider child; the spawn approval;
  acknowledgement of a read outcome, so no completion turn follows; an
  unread outcome delivered by a completion turn on the stand-in's resumed
  session; a harness child that cannot delegate.
- [x] 6.2 Remote MCP tools and plugin and module tools: approval,
  attribution, and credentials unchanged; an untrusted server contributes
  nothing; the CLI invocation names only the `smith` server and holds no
  server address or secret.
- [x] 6.3 Skills: search and activation, the instructions in the result,
  the changed-content refusal.
- [x] 6.4 `advisor`: the request to a fake advisor provider contains the
  task, a CLI-run tool labelled as such, and a bridged call with its
  result; usage is recorded under the advisor attribution.
- [x] 6.5 Questionnaire and `artifact.read`: an answer round trip; a
  bounded result read back through `artifact.read`. (The read-back passes.
  A headless run offers no questionnaire, natively or bridged, and a call
  to it is refused as an unknown tool; the answer round trip needs an
  interactive host and is still open.)
- [x] 6.6 The boundary breaking mid-flow: stand-in abandons a call during
  approval (by `notifications/cancelled`, and by closing the connection),
  exits mid-call, sends two calls at once, never connects, and is
  interrupted while a child process of its own is still running. Assert
  effects with the counting tool. (Abandonment during approval runs through
  an in-process terminal host with a test-controlled approver,
  `bridge_approvals::`, because headless cannot hold an approval open.)
- [x] 6.7 Persistence: a session with bridged calls is saved, resumed, and
  rendered with its tool rows and usage intact; the endpoint is not
  restored.

## 7. Surfaces

- [x] 7.1 TUI render tests: bridged calls as Smith tool rows inside a
  harness-labelled turn, the approval prompt, no duplicate external row, a
  CLI-run tool beside them.
- [x] 7.2 Headless `json` and `stream-json` golden output: ordinary tool
  events for bridged calls, no external event for the same call,
  fail-closed approval. (A completed turn's diagnostics now appear in an
  optional `diagnostics` array of the result and on stderr in text mode,
  an additive field under schema 3.)
- [x] 7.3 The not-connected diagnostic in both surfaces.
- [x] 7.4 TUI and headless produce the same canonical events for one
  scripted stand-in run.

## 8. Verification and documentation

- [x] 8.1 Opt-in live tests against installed `claude` and `codex`: spawn a
  provider child and read its result; call a remote MCP tool behind an
  approval; activate a skill; call the advisor; resume and confirm the
  bridge is still offered. (`live_cli_bridge` `*_live_flow`. Passed
  2026-10-14 on Claude Code 2.1.299 and Codex 0.162.1: a bridged `agent`
  call, skill search and activation, resume with the bridge offered again.
  Child step passed 2026-10-14 on main `1106510` with
  `LIVE_CHILD_MODEL=zai/glm-5.3`, Claude Code 2.1.300 and Codex 0.162.1,
  after three fixes to the never-run step: a profile name over the
  32-character limit, `--approval deny` refusing the spawn (turn two now
  uses `allow-all` only when the child step is on), and a provider-qualified
  id written as the profile's bare `model`. `LIVE_ADVISOR=1` needs an
  advisor in the owner's config, which has none, so the advisor and the
  remote tool behind an approval were checked in tmux instead (8.5): both
  CLIs, a scratch project with `advisor = "zai/glm-5.3"` and a stdio MCP
  server; the tool ran once, only after approval; resume re-offered the
  bridge.)
- [x] 8.2 The drift canary: each installed CLI still accepts the flags and
  keys Smith passes and still prints the 1.7 shapes. No model call.
  (`live_cli_bridge` `*_drift_canary`, opt-in. Passed 2026-10-14 against
  Claude Code 2.1.299 and Codex 0.162.1.)
- [x] 8.3 The traceability table has no empty row. (68 scenarios; every
  listed test passed on 2026-10-14.)
- [x] 8.4 `docs/configuration.md` (installed coding agents, turning the
  bridge off), `docs/security.md` (the endpoint and its authority),
  `docs/skills.md`. (Also `docs/headless-protocol.md`: the `diagnostics`
  result field and signal handling.)
- [ ] 8.5 The Smith gate with `--no-fail-fast` and an internal `TMPDIR`, a
  patch release, and the installed binary checked in a tmux session with a
  real configuration. (Gate done 2026-10-14 on main at v0.4.0 plus this
  change, no local patch: `--locked` build, clippy, `cargo deny --locked
  check all`, and 2,680 tests pass. Tmux check done 2026-10-14 with a
  debug build of main `1106510` and the owner's real config, on
  `cli/claude-code/haiku` and `cli/codex/gpt-6-astra`: `/mcp trust`, an
  approved remote MCP call, a `zai/glm-5.3` child, the advisor, `/quit`
  and `--resume`; findings in `docs/qa/live-2026-10-14/findings.md`. The
  installed binary is still v0.4.0. Open: the release.)

## 9. Follow-ups from the 2026-10-14 live checks

Requested by the owner before any release ("we need that before we do
anything else"). Findings are in `docs/qa/live-2026-10-14/findings.md`.

- [x] 9.1 The exit summary and usage provenance name a harness model
  without the profile's provider: `cli/claude-code/haiku`, not
  `zai/cli/claude-code/haiku`, matching the status bar. (Usage buckets,
  the usage log, and `/status` drop the provider and price for a `cli/`
  model; legacy logs restore. Checked in tmux 2026-10-15: `price unknown for
  cli/claude-code/haiku · advisor zai/glm-5.3`.)
- [x] 9.2 Interactive sessions name untrusted MCP servers once at start
  with `/mcp trust <name>` (extension-system delta, "Interactive sessions
  name untrusted servers"). Confirm headless runs already report them, as
  the truth requirement "Untrusted servers are inert in non-interactive
  runs" demands. (Headless did not report them; it now does, in
  `diagnostics` or on stderr for text output. Checked in tmux 2026-10-15:
  one notice on start and one on `--resume`.)
- [x] 9.3 The name-validation message reads correctly for every kind
  ("a agent profile name" today). (Now "the {kind} name must contain ...";
  the kinds are `agent profile`, `agent mode`, and `child agent`.)
- [x] 9.4 `mcp_relay::stdin_eof_half_closes_the_socket_and_drains_the_response`
  no longer flakes on a loaded runner: its deadline covers starting the
  `smith` binary, and a deadline failure reads as a deadline, not EAGAIN.
  20 consecutive passes under load. (30 s fixture deadline; socket timeouts
  map to "deadline exceeded during <step>". 20/20 with all 14 cores
  saturated.)
- [x] 9.5 `stand_in::abandonment_and_exit_can_wait_for_execution_evidence`
  no longer depends on runner timing. 20 consecutive passes under load.
  (The stand-in was released as soon as the start file existed, which the
  counting server creates before writing its record, so teardown could
  kill the writer mid-record. The test now releases the stand-in only
  after reading a complete record. 20/20 with all 14 cores saturated.)
- [x] 9.6 A headless shutdown, including an interrupt during an MCP call,
  leaves no Smith-declared stdio MCP server running. Tested with a
  sleeping MCP tool; `design.md` updated. (Smith owns the stdio child and
  terminates it at process exit, headless or TUI: SIGTERM, a short wait,
  then SIGKILL; a second signal kills first. Servers stay in Smith's
  process group. Tests: `bridge_failures::headless_*_stdio_mcp_server` for
  completion, SIGINT, SIGTERM/SIGHUP, and a second signal.)
- [x] 9.7 `setup_pty::command_jsonl_provider_streams_a_turn_through_the_normal_tui`
  no longer flakes. It failed once on macOS CI for this branch and 6 of
  ~200 local runs, against 0 of ~190 on main. Cause: the test sent `/quit`
  as soon as the reply text was drawn, which can still be inside the turn;
  `/quit` during a turn opens the quit confirmation (DESIGN.md), and turn
  completion cancels pending prompts, so Smith stayed open. The test now
  waits for the turn to finish. Two regression tests, using a fixture hold
  that keeps the turn open, prove that text typed mid-turn survives with
  later input still working, and that `/quit` mid-turn goes through the
  confirmation. 100/100 and 30/30 + 30/30 local runs. No product change.

Not in this section: the runtime conformance flake
`session::one_shutdown_deadline_bounds_all_active_turns` (ForgeAILab/smith#4)
and stopping a busy stdio MCP server after Smith is SIGKILLed are in
`update-exit-bounds-2026-10-15`. The 5 s command-provider probe timeout
was never observed to fail; the runtime already exposes
`CommandLimits::with_probe_timeout` if it does.
