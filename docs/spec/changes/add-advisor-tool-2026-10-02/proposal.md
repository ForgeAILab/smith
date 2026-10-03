---
created_at: 2026-10-03T03:19:53Z
updated_at: 2026-10-03T05:23:35Z
---

# Proposal: Add an advisor tool backed by a stronger model

## Why

A working model can consult a stronger reviewer at the moments that decide
a task's outcome: before committing to an approach, when stuck, and before
declaring the work done. Today a Smith session has no way to do that short of
spawning a child agent, which starts cold, re-derives context, and costs a
whole agent run. The owner uses this pattern in Claude Code and asked for it
in Smith (2026-10-02: "add advisor to smith as well … i really like the
advisor feature").

## What Changes

- Profiles gain an `advisor` placement in `use`, and a top-level or
  per-profile `advisor = "<profile>"` key selects which profile advises the
  main agent. `advisor = false` on a profile turns it off. The feature is off
  unless configured.
- Root sessions with an advisor register a model-facing `advisor` tool that
  takes no arguments. Calling it sends the session's conversation so far,
  including this turn's tool calls and results, to the advisor profile's
  provider and model as one request with no tools, and returns the advice as
  the tool result.
- The main agent's instructions gain a short section on when to consult the
  advisor and how to weigh its advice, contributed only when the tool is
  registered.
- Advisor provider usage counts toward the session's usage and cost.
- The TUI and headless output show the call as an ordinary tool row whose
  result is the advice.

## Impact

- Affected specs: new `advisor`; `configuration`.
- Affected code: `smith-config` (profile placement, advisor key, validation,
  `config explain`), `smith-runtime` (advisor route built like a child route,
  the `advisor` tool, transcript rendering, prompt section, usage
  attribution), `smith-tools` (tool-call display label), docs.
- No change for configurations that do not set `advisor`.
- The advisor sends the conversation, including tool output, to the advisor
  profile's provider. That provider may differ from the main one; the
  configuration documentation says so.

## Authorization

The owner approved this proposal before review ("i approve the spec proposal
as i trust you", 2026-10-02) and delegated the open design choices; they are
recorded in design.md. Implementation runs on `feat/advisor-tool`, based on
`refactor/client-structure`, and ships with that release.
