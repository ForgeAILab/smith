## Context

`shell` currently guarantees that nothing outlives the invocation: the command
runs in its own process group, and timeout, cancellation, or drop kills the
whole group (`crates/smith-tools/src/shell.rs`). Background tasks deliberately
relax that invariant from *invocation*-scoped to *session*-scoped, which is
the same lifetime the monitor capability already owns: work IDs, bounded
spool files, session-inbox terminal notifications, active-work exit policy,
and `interrupted_by_process_exit` reconciliation on resume. This design reuses
that machinery rather than inventing a parallel lifecycle.

Monitors and background tasks stay distinct on purpose: a monitor turns output
lines into chat events as they arrive; a background task runs to completion
silently, spools everything, and notifies exactly once at exit. A model that
wants progress streaming should use `monitor`; a model that wants a result
should use `run_in_background`.

## Goals / Non-Goals

- Goals:
  - Let the model opt a shell command into background execution at call time.
  - Let the user rescue an already-running foreground command without killing
    it.
  - Make the foreground timeout outcome teach the model its options.
  - Keep every background process session-owned: no orphan survives shutdown,
    crash-resume never fabricates a running task.
- Non-Goals:
  - No automatic timeout-to-background conversion. A timed-out foreground
    command is killed, exactly as today.
  - No change to the foreground defaults (120 s default, 600 s max).
  - No cross-session or daemonized tasks; a task never outlives its Smith
    process.
  - No streaming of background output into chat (that is `monitor`).

## Decisions

- Decision: `run_in_background: true` returns immediately after spawn with a
  task ID and spool reference; approval and validation run exactly as for a
  foreground call, before spawn.
  - Alternatives considered: returning after first output (rejected —
    ambiguous for silent commands); a separate `shell_background` tool
    (rejected — same schema, same approval surface, one more tool to teach).
- Decision: background tasks default to no deadline and run until exit,
  `task_stop`, or session shutdown; `timeout_ms` MAY still be supplied to
  bound one. Mirrors the persistent-monitor precedent; the active-work exit
  policy is the orphan guard.
  - Alternatives considered: inheriting the 120 s default (defeats the
    purpose); a mandatory cap (arbitrary; monitors already trust the exit
    policy).
- Decision: output spools to a bounded per-task file under the session
  directory (monitor's output-file pattern), with the byte cap enforced at the
  spool. `task_output` reads incrementally by offset and reports status and
  exit code; the exit notification itself carries only a bounded tail.
- Decision: journal records metadata-only lifecycle markers
  (`task_started` / `task_exited` mirroring
  `record_monitor_started` / `record_monitor_stopped`), never output bodies.
  Resume reconciles a started-without-terminal task as
  `interrupted_by_process_exit` and never restarts it.
- Decision: manual backgrounding resolves the pending foreground tool call as
  a successful outcome containing output-so-far, the task ID, and an explicit
  statement that the user moved it to background — the model must not read it
  as completion. Implementation seam: the foreground `select!` gains one more
  arm (a background-request signal) that hands the child and its collector to
  the session task registry instead of `stop_group`.
- Decision: `task_stop` addresses session work IDs. The monitor spec already
  names `TaskStop` for stopping monitors; this change introduces the concrete
  snake_case tools (`task_stop`, `task_output`) and treats the monitor spec's
  `TaskStop` as the same host action. Naming alignment in the monitor truth
  spec happens at its next touch, not in this change.

## Risks / Trade-offs

- Relaxing the shell crate's "nothing outlives the invocation" invariant risks
  orphaned groups if the registry misses a handoff path. Mitigation: adoption
  is a single explicit seam out of the `select!`; everything else keeps
  `kill_on_drop`, and shutdown kills all registered groups within the existing
  grace period.
- A silent long-running task can be forgotten. Mitigation: operational status
  lists running tasks; exit policy blocks silent orphaning; the exit
  notification always arrives at a safe boundary.
- Spool growth on chatty commands. Mitigation: hard byte cap at the spool with
  a truncation marker, same posture as `MAX_CAPTURE_BYTES`.
- Manual backgrounding surprises a model mid-turn. Mitigation: the resolved
  outcome text names the user action explicitly and points at `task_output`.

## Migration Plan

Purely additive: no schema change to existing journal records (new marker
kinds only), no change to foreground shell semantics or defaults. Old
transcripts replay unchanged. New journal markers require a schema-version
bump only if the journal's validation is closed-world; follow the pattern the
monitor markers used.

## Open Questions

- Whether the exit notification should wake an idle session into a new turn or
  wait for the next user submission — follow whatever the monitor terminal
  notification does today; the spec deltas only require safe-boundary inbox
  delivery.
- Exact spool byte cap default (proposal: 8 MiB to match `MAX_CAPTURE_BYTES`).
