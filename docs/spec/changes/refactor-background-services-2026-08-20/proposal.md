---
created_at: 2026-08-20T22:13:02Z
updated_at: 2026-08-20T22:22:11Z
---

## Why

Background shell tools currently discover a process-global host through a
first-install-wins `OnceLock`, and that host reaches a second process-global
task registry. This hidden ambient state prevents isolated concurrent Smith
embeddings, while `task_stop` can return `running` immediately after accepting
a stop because the worker's terminal transition is asynchronous.

## What Changes

- Inject a runtime-owned `BackgroundTaskHost` into shell, task-output, and
  task-stop tool construction. Remove the process-global installed-host seam.
- Make each standard Smith host own an `Arc<BackgroundTaskRegistry>` and pass it
  explicitly through factory services. Session IDs still isolate tasks inside
  one registry, while separate hosts have separate registries.
- Change the stop protocol to carry a completion acknowledgement. `task_stop`
  waits within the cleanup bound for the worker to terminate its process group,
  commit one terminal transition, publish its terminal notification, and
  acknowledge the resulting state.
- Preserve idempotence: an already-terminal task returns that state immediately;
  an unknown ID remains a stable error.

## Impact

- Affected specs: `tool-execution`, `runtime-integration`
- Affected code: `smith-tools` background/shell/task tools,
  `smith-runtime` background registry/factory/host lifecycle, CLI shutdown
  policy, and background-task tests
- Compatibility: internal constructor and host-trait changes are source
  breaking for direct embedders. User-visible task IDs, output polling, terminal
  states, and journal formats remain compatible.
- Dependency: this change may land before `add-modular-harness-boundaries`; its
  explicit service field is later incorporated into `ResolvedHarness` without
  restoring global state.
