## Context

`smith-tools::background` stores its `BackgroundTaskHost` in a static
`OnceLock`, and `smith-runtime::BackgroundTaskRegistry` has its own static
`OnceLock`. Every adapter call reaches `BackgroundTaskRegistry::global()`.
`stop_task` takes and sends the stop channel, then immediately rereads status;
the worker updates that status only after asynchronous process cleanup, so the
accepted stop may be rendered as `running`.

## Goals / Non-Goals

- Goals:
  - Make background-task authority and lifecycle explicit per Smith host.
  - Support multiple isolated runtimes and deterministic tests in one process.
  - Make a successful `task_stop` result report a terminal state.
  - Preserve exactly-once terminal journal and notification behavior.
- Non-Goals:
  - No durable process resurrection across Smith process exit.
  - No change to task IDs or bounded spool format.
  - No automatic conversion of foreground work to background work.

## Decisions

### Decision: inject tools with one host-owned service

The standard host creates one `Arc<BackgroundTaskRegistry>` and wraps it as the
`BackgroundTaskHost` supplied to built-in tool constructors. Host exit policy,
session registration, and shutdown retain the concrete registry arc. Direct
embedders must supply a service or explicitly omit background-capable tools.

A task-local or replaceable global was considered. It still introduces ambient
lookup, nesting ambiguity, and cross-runtime interference. Constructor
injection makes ownership inspectable and testable.

### Decision: acknowledge completed stop

The current tool contract says `task_stop` terminates a task, not merely queues
a request, so the stop call waits for terminal acknowledgement within the
existing cleanup bound. The request carries the terminal reason and a oneshot
acknowledgement. The worker acknowledges only after process-group cleanup,
terminal state publication, journal append, and notification enqueue.

Returning a new `stop_requested` status was considered. It would be truthful,
but changes every client and requires the model to poll for a command that is
already specified as synchronous. A bounded acknowledgement better matches the
current contract.

### Decision: terminal transition has one owner

Natural exit, stop, deadline, and shutdown race through one transition function.
Only the winning transition records and notifies; losing paths observe the
winning terminal status and complete any waiter with it.

## Risks / Trade-offs

- Awaiting acknowledgement makes `task_stop` latency include the process-group
  grace period. The call is bounded and reports failure honestly if the bound is
  exceeded.
- Tool constructors become less convenient in unit tests. A small explicit fake
  is preferable to tests sharing mutable process state.
- Host shutdown must retain the service bundle until tools and workers finish;
  premature drop is covered by lifecycle tests.

## Migration Plan

1. Add instance registry and service-bearing tool constructors alongside the
   globals for one internal transition.
2. Move standard hosts, tests, exit policy, and shutdown to the injected
   instance.
3. Introduce the acknowledged stop protocol and strengthen status assertions.
4. Remove both globals and compatibility functions in the same release.

## Open Questions

None. The existing bounded process cleanup period is the stop acknowledgement
deadline unless implementation evidence shows it needs a separately named
configuration value.
