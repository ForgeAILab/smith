---
created_at: 2026-08-04T16:55:00Z
updated_at: 2026-08-04T16:55:00Z
completed_at:
---

## 1. Runtime task registry

- [ ] 1.1 Add a session-owned background-task registry to `smith-runtime`:
      task IDs, owned process-group handles, bounded per-task output spool
      files under the session directory, terminal-state tracking
- [ ] 1.2 Emit exactly one terminal notification per task through the session
      inbox (exit, stopped, deadline kill, shutdown), carrying task ID,
      terminal state, exit code, and a bounded output tail
- [ ] 1.3 Journal metadata-only `task_started` / terminal markers mirroring
      the monitor lifecycle markers; extend replay validation accordingly
- [ ] 1.4 Reconcile on resume: a start marker without a terminal marker
      becomes `interrupted_by_process_exit`, never respawned
- [ ] 1.5 Fold running tasks into active-work accounting: TUI exit
      confirmation and headless `error` / `wait` / `stop` policies; shutdown
      kills registered groups within the grace period

## 2. Shell tool

- [ ] 2.1 Add `run_in_background` to the `shell` schema; on true, spawn under
      the same validation/approval, hand the child and collector to the
      registry, and return task ID + spool reference immediately
- [ ] 2.2 Support optional `timeout_ms` on background tasks (deadline kill via
      the registry); keep the foreground default and max unchanged
- [ ] 2.3 Enrich the foreground timeout outcome text: elapsed limit, partial
      output note, and the three options (`timeout_ms`, narrower command,
      `run_in_background`)

## 3. Task tools

- [ ] 3.1 Add `task_output`: status, exit code when terminal, and an
      offset-addressed bounded slice of the spool; stable unknown-ID error
- [ ] 3.2 Add `task_stop`: group termination with grace period, idempotent on
      terminal tasks, stable unknown-ID error
- [ ] 3.3 Register both tools with redaction-safe display summaries

## 4. TUI

- [ ] 4.1 Show running background tasks in operational status with task IDs;
      drop them on terminal notification
- [ ] 4.2 Add the manual backgrounding action on a running foreground shell
      call (ctrl+b): adopt the group into the registry, resolve the pending
      call with output-so-far + task ID + explicit user-moved statement;
      keep interrupt semantics untouched; no headless affordance

## 5. Validation

- [ ] 5.1 Tests: background spawn returns promptly; process survives call
      resolution; deadline kill; stop idempotency; spool cap truncation;
      unknown-ID errors; timeout text names all three options
- [ ] 5.2 Tests: resume reconciliation, exit-policy blocking, shutdown group
      kill, inbox delivery at a safe boundary
- [ ] 5.3 TUI tests: status listing, ctrl+b adoption path, interrupt
      unchanged; update `docs/spec` validation and `DESIGN.md` if needed
