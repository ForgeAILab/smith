## Context

Interface 0.1 gives a sandboxed module tools, context contributors,
tool-output processors and status items, with file, HTTP, key-value and
history host functions. Provider authority is never granted. A module is
entered only from a tool call, a pipeline phase, or a status refresh.

Smith already has both mechanisms this change exposes. The advisor tool
makes a side request on a configured provider and accounts for it
separately. Monitor notifications reach a session through its inbox without
a user prompt and are handed to the model at a safe boundary.

## Goals / Non-Goals

- Goals: a plugin can call the session's model without credentials; a
  plugin can bring an event to a session that is not currently calling it;
  both are granted, bounded and visible.
- Non-Goals: compaction replacement, streaming, tool use inside a module's
  model call, selecting another provider, pre-tool-call hooks, UI.

## Decisions

- **One new package version, 0.2.0.** New imports and a new contribution
  variant require it under the existing rule. 0.1 components are loaded
  through an adapter that maps them onto the 0.2 host; they cannot reach the
  new functions.
- **`complete` is a host function, not a provider handle.** Input: an
  optional system text, up to 64 text messages, and a maximum output token
  count. Output: text, a finish reason, and usage counters. No tools, no
  streaming, no reasoning controls. The request goes to the session's
  current provider and model through the same route construction the
  advisor uses, so credentials, rotation and transport stay host-owned.
  The shared one-shot path also supports the session's installed-agent route:
  message roles are rendered into one user input, tools are withheld, and CLI
  usage is unpriced. CLI generation limits are owned by the installed agent;
  the requested output-token ceiling is enforced by provider routes. Both
  routes retain host deadlines, call counts and ABI payload bounds. Truncated
  answers return `length`; filtered, empty and unfinished answers are errors.
- **Capability `model`.** A boolean manifest key. Sending text to the
  provider the session already uses does not also require `data_egress`;
  the install review says the module can send data to the active model
  provider.
- **Spend bounds.** Defaults: 4096 output tokens per call, 4 calls per turn,
  and input bounded by the 1 MiB payload ceiling and the model's input
  limit. The user file may lower either per module; no layer may raise them
  above the defaults in this version. A refused call returns a typed host
  error and is not an invocation failure.
- **Deadlines.** Time spent inside `complete` does not count against the
  guest's 5 second tool deadline; the call has its own 120 second deadline.
  `complete` is unavailable from `describe`, `mount`, `status` and `poll`,
  which keeps those phases cheap. Context contributors and tool-output
  processors may not call it either: a model call on every request would
  multiply cost silently. Only tool calls may.
- **Usage.** Each call is one provider attempt attributed to the module id,
  reported like advisor usage and included in session totals.
- **Watcher.** A module registers a `watcher` id at mount. Smith calls
  `poll(id)` on a schedule the host owns: at most once every 5 seconds per
  module, with the 500 ms pipeline deadline, serialised with the module's
  other calls. `poll` returns up to 8 notices of at most 4096 bytes each.
  Polling rather than a guest-initiated push keeps the guest passive and
  gives the host the rate limit for free.
- **Delivery.** Notices enter the session inbox as module notifications,
  labelled with the plugin name, and follow the monitor delivery rule:
  shown to the user at once, supplied to the model at the next safe
  boundary, never interrupting a stream. Flood protection reuses the monitor
  batching limits. A notice is untrusted text and is framed as such.
- **Capability `session_notify`.** A boolean manifest key. Without it a
  watcher contribution is refused at mount with a reason, rather than
  failing at the first poll.
- **Headless runs.** A `-p` run does not poll after its turn ends; a watcher
  cannot keep a headless process alive.
- **Child sessions.** Watchers run for the root session only.

## Alternatives considered

- A guest-initiated `notify` import: simpler for the tool-call case, but it
  does nothing for a module that is not being called, which is the case
  that matters.
- Host-side file watching on plugin data: works for the relay only, and
  exposes filesystem events as an interface.
- Letting a module name a provider and model: needs credential scoping per
  module and a larger review surface; deferred.

## Risks / Trade-offs

- A module with `model` can spend the user's tokens. Mitigated by the
  install review, per-turn and per-call caps, and attributed usage.
- A module with `session_notify` can start turns. Mitigated by the poll
  interval, notice bounds, monitor flood limits, and the existing
  three-failure disable rule.
- Supporting two interface versions adds an adapter to maintain.

## Open Questions

- Should a notice be able to start a turn in a session whose approval mode
  is `ask`, given the turn may then stop at an approval prompt? Proposed:
  yes, as monitor notifications already do.
- Is 4 model calls per turn the right default?
