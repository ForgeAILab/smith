---
created_at: 2026-08-20T22:13:02Z
updated_at: 2026-08-21T01:33:37Z
completed_at:
---

## 1. Explicit Background Services

- [x] 1.1 Add a factory-owned background service bundle containing the task
  host/registry and require standard hosts to resolve it before tool assembly.
- [x] 1.2 Construct `ShellTool`, `TaskOutputTool`, and `TaskStopTool` with an
  explicit `Arc<dyn BackgroundTaskHost>`; provide a deliberate unavailable
  test adapter rather than consulting global state.
- [x] 1.3 Replace `smith_tools::background::HOST`, `install`, and `installed`
  with constructor injection and remove first-install-wins behavior.
- [x] 1.4 Replace `BackgroundTaskRegistry::global()` with registry instances
  owned by the standard runtime host and pass the same instance to session
  registration, tools, exit policy, and shutdown.
- [x] 1.5 Add two-host tests proving task IDs, spools, signals, notifications,
  shutdown, and injected fakes remain isolated in one process.

## 2. Acknowledged Stop Protocol

- [x] 2.1 Replace the stop oneshot payload with `StopRequest { reason,
  completed }`, where `completed` acknowledges a terminal status after process
  cleanup and the single terminal transition.
- [x] 2.2 Make the worker stop its process group, update shared status, append
  the metadata lifecycle marker, enqueue exactly one terminal notification, and
  then acknowledge completion.
- [x] 2.3 Make `BackgroundTaskHost::stop` return only an already-terminal or
  acknowledged-terminal state; return a bounded timeout/error if cleanup cannot
  be confirmed rather than reporting `running` as success.
- [x] 2.4 Preserve natural-exit races and idempotence without double terminal
  notification, double journal record, or a lost acknowledgement.
- [x] 2.5 Strengthen the existing stop test to assert the first returned status
  and add races for natural exit, repeated stop, shutdown, deadline kill, and
  cleanup timeout.

## 3. Host Lifecycle and Verification

- [x] 3.1 Route headless `error`, `wait`, and `stop` exit policies and TUI
  shutdown through the injected registry.
- [x] 3.2 Prove dropping one embedded host cannot stop, inspect, or reconfigure
  another host's tasks.
- [ ] 3.3 Run formatting, Clippy, workspace/all-feature tests, background-task
  integration tests, persistence/recovery tests, and macOS/Linux process-group
  CI.

Local macOS formatting, Clippy, workspace/all-feature, background-task, and
persistence/recovery gates are green as of 2026-08-20. The checkbox remains
open for the Linux process-group CI half.
