## Context

Main and child turns already pick their execution from the resolved config:
`cli_agent::turn_execution(config, smith_executable)` returns `Provider`,
`InstalledAgent(CliAgentPlan)`, or `MissingProgram`. An installed agent then
runs through `agent_runtime::agent::external::ExternalAgentBackend`, built by
`CliAgentPlan::backend(cwd, allow_own_tools)`. The advisor is the only route
that skips this. `prepare_advisor_route` always builds a provider, and
`AdvisorRoute::consult` always calls `Provider::stream`.

## Goals / Non-Goals

- Goals:
  - one execution choice, `turn_execution`, for main, child, and advisor
  - `cli/<kind>/<model>` accepted wherever an advisor target is accepted
  - `/advisor` model rows come from the same builder as `/model` rows
- Non-Goals:
  - no agent-runtime change: `ExternalAgentBackend::run_turn` is already
    public at the pinned rev `1d0a821`
  - no persistent CLI conversation for the advisor: every call is fresh
  - no installed-agent vision target
  - no Smith-side reasoning or effort control for a CLI advisor; the CLI
    owns that

## Decisions

- Decision: add `AdvisorTarget::InstalledAgent { kind, model }`, displayed
  as `cli/<kind>/<model>`. `parse` checks
  `cli_agents::parse_cli_model_id` before the provider split.
  - Alternatives considered: keep `Model { provider: "cli", .. }` and
    special-case it later. Rejected, because every match on `Model` would
    have to remember the special case, and validation already gets it wrong.
- Decision: route selection contributes only `model`. The host sets the
  advisor request's provider to the main session's resolved provider name,
  because route resolution clears the session's provider and profile.
  - Alternatives considered: require a top-level `provider`. Rejected:
    profile-only configs would fail for a provider that is never called.
- Decision: `AdvisorRoute` gets an `execution` field:
  `AdvisorExecution::{Provider(Arc<dyn Provider>), InstalledAgent(CliAgentPlan), MissingProgram { model, program }}`.
  Its contents come from `turn_execution`, so the harness settings and
  PATH discovery are the same code main turns use. `consult` dispatches on
  it. Transcript rendering, budget, cancellation, the deadline, the output
  bound, and `AdvisorUsage` are shared by both branches.
- Decision: the installed-agent call is
  `plan.backend_with_instructions(workspace_root, false, system_prompt)`.
  It passes `ExternalTurnRequest { input: transcript, resume: None, cancel:
  child cancellation, capabilities: default, bridge: None, tools:
  ExternalToolDispatch::disabled() }`.
  - `Text` is accumulated up to the output bound.
  - `Usage` is merged into the advisor usage.
  - `ToolInvoked`/`ToolCompleted`, `Reasoning`, `Diagnostic`, and
    `SessionStarted` are ignored. With its own tools off, Claude Code may
    still run read-only tools and Codex runs in its read-only sandbox, so
    tool observations are not an error.
  - `Completed` with non-whitespace text is success. With no such text it
    fails as "advisor returned an empty answer".
  - `Failed { message }` is a tool error.
  - A stream that ends without a terminal event fails as "advisor did not
    finish normally".
- Decision: `MissingProgram` does not fail composition. The tool stays
  registered, and each call returns `advisor failed: ...` naming the
  program. This matches how a child route carries a missing program.
  `/advisor` disables the row, so an override cannot select it.
- Decision: the `/advisor` model rows are built from the `/model` builder
  (`model_entries` + `append_cli_agent_models`) and then re-marked. `active`
  means the session override equals this id. The session's own binding is
  dropped. A row with no usable limits is disabled instead of filtered out.

## Risks / Trade-offs

- A CLI advisor launches a process per call, which is slower than a
  provider call. That is acceptable for an opt-in reviewer.
- Claude Code's read-only tools mean a CLI advisor may read the workspace.
  The advisor prompt still tells it not to claim it ran commands. Reading
  is within the read-only posture the owner already accepts for tools-off
  harness turns.

## Migration Plan

None. This is additive: existing advisor values parse the same way, unless
someone declared a provider literally named `cli` and also used a catalog
kind after it. The catalog rule wins in that case and is documented.

## Open Questions

None.
