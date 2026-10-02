# fix-interaction-defects — verification, 2026-10-02

Implementation was written by Codex (five dispatches: four parallel chunks
and one test-fix pass). Builds, tests, and the live checks below were run by
the orchestrator. Nothing is committed.

## Gates

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `cargo test --workspace --locked --no-fail-fast` | 1,719 passed, 7 failed, 6 ignored in the last full run; every failure is a timeout — see below |
| Spec validation, strict | pass |

No single full run was green. The machine was running several unrelated
`open-forge` builds throughout (load average 4 to 6), and the failures moved
between runs:

- `smith-cli` unit tests: failed 5 of 148 in the full run, then passed 148
  of 148 in three of four isolated runs. The failures were three existing
  headless flow tests and two of the new shell-shortcut tests, all `Elapsed`.
- `smith-runtime` `host_session`: 2 to 5 of 41 failed, all `Elapsed` or a
  missed notification. **Control:** the same target on a clean worktree at
  `58bc098`, with none of these changes, failed 2 of 41 under the same load
  (`a_background_task_terminal_notification…`,
  `interrupting_one_turn_does_not_cancel…`). This change does not touch
  `smith-runtime`.
- `model_catalog::tests::stale_snapshot_schedules_one_non_blocking_refresh`
  failed once in the first run and passes alone.

The suite needs one clean run on a quiet machine before this change is
called complete.

### Clean run, later the same day

The temp directory is a cause of the timeouts. The shell exports
`TMPDIR=/Volumes/Data/tmp/`, a directory of about 5,400 entries on a volume
that is 96% full; every test that starts a host session creates its home and
project there. Why that location is slow was not established. The
shell-shortcut rows below compare the two locations at the same load
(about 3.2). The full-suite row does not isolate the cause: load had also
fallen from about 10 to about 3.5 since the failing runs. With `TMPDIR` on
the internal disk, on the same uncommitted tree:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | pass |
| `TMPDIR=/private/tmp/smith-fixture-tmp cargo test --workspace --locked --no-fail-fast` | 1,726 passed, 0 failed, 6 ignored; slowest suite 10 s |
| The eight `local_shell_tests`, internal `TMPDIR`, five runs | 8 of 8 each time, about 1.2 s |
| The same eight, default `TMPDIR`, three runs | 2 of 8, then 8 of 8 twice (7 to 18 s) |

One full run is green. The shell-shortcut authorization tests no longer
depend on a quiet machine.

## Live checks (rebuilt `target/debug/smith`, isolated `HOME`, fake provider)

| Scenario | Result |
| --- | --- |
| First-run setup, "Anthropic Messages API", Enter | Advances to Authentication ([capture](captures/after-01-setup-anthropic.txt)) |
| Anthropic flow to the end: environment-variable credential, `claude-opus-5-5`, default, review, confirm | Limits resolved from the trusted catalog; config published; the real local preflight passed (no provider request); `smith config explain model` resolves |
| `/model`, Esc, `/status` | Composer empty after cancel; `/status` runs |
| `!ls -la` under `ask` policy | Runs with no approval prompt ([capture](captures/after-02-bang.txt)) |
| `smith setup --help`, `smith help`, `smith sessions --help`, `smith config explain --help` | Usage on stdout, exit 0, includes a SETUP OPTIONS section |
| `smith --bogus`, `smith sesions list` | One hint, exit 2 |
| `echo hi \| smith` | "the interactive surface needs a terminal…; use `smith -p -`…", exit 1 |
| `smith sessions list` on a terminal | Header, aligned columns, local date and time |
| `smith sessions list` piped | Unchanged tab-separated rows |

## Covered by tests only

- Approval keystroke guard (typing through an arriving approval, the 500 ms
  boundary, rotation). The fake provider requests no tools, so this was not
  exercised live. Later the same day the typing-through case was walked
  live against `zai/glm-5.3`; see
  [the live walkthrough](live-walkthrough-real-provider.md). The exact 500 ms
  boundary and rotation remain test-only.
- Redo, MCP-trust, and skill-trust confirmation copy.
- "Connect ChatGPT" hand-off from setup: the PTY test confirms it advances
  and cancels without writes; a real OAuth sign-in was not attempted.
- Configured `background.exit_policy` (`wait`, and the flag overriding it).
- A model-requested shell call still asks after a user shortcut, including
  an identical command queued during the shortcut.

## Sweeps re-run

The 09-21 drivers were never checked in, so
[`sweeps/run_sweeps.py`](sweeps/run_sweeps.py) rebuilds both sweeps from that
audit's result lists and captures. It drives `target/debug/smith` in a private
tmux server with an isolated home, fake providers, and a loopback models
endpoint for setup. Run on 2026-10-02 against main at `0f5fc48` plus this
change and `show-provider-retry-progress`:

- Command sweep: **26/26 passed** ([results](sweeps/results/command-sweep.json)).
- Startup sweep: **40/40 passed** at 100×32, 80×24, 44×16, no-color, and
  setup Back/cancel ([results](sweeps/results/startup-checks.json)).

Two differences from 09-21. A command check now ignores the echoed command
and anything printed before it, because words like `goal` and `redo` match
the echo alone. `/timeline` expects `No turns`: its empty state no longer
contains the word "timeline". Frames are in
[sweeps/results/captures](sweeps/results/captures/).

## Preconditions

Precondition 1.2 is resolved by committing `show-provider-retry-progress`
together with this change; `show-idle-compaction-summary` has no code.

## Follow-ups

- The shell shortcut learns its turn identity from a steering rejection
  because the pinned runtime exposes no handle for a local call. It fails
  closed (the prompt appears) if that ever changes. A first-class API in
  `agent-runtime` would replace it.
- `docs/security.md` still describes approval for the shortcut.
- `https://api.anthropic.com/v1` is now a literal in `smith-tui/src/setup.rs`
  beside `smith_config::model::ANTHROPIC_DEFAULT_ENDPOINT`;
  `refactor-client-structure` removes it.
- Sessions with no model turn list as `?/?` and `no user preview`.
- The eight new shell-shortcut tests are load-sensitive. They passed in three
  full `smith-cli` unit runs (148 of 148) and then failed in five of five
  serial re-runs once the machine load rose, each time on a 10-second
  timeout (`local action completed`, `approval arrived`, `model turn
  completed`), never on an authority assertion. These tests are the only
  automated evidence that a model call cannot use the shortcut's
  authorization, so they need a clean run on a quiet machine.
- The shortcut's authority binding assumes the pinned runtime rejects, and
  does not queue, a local tool call when the session is busy, so a pending
  first poll proves the local action owns the turn. Codex read this in the
  pinned source; it is worth confirming with the runtime.
