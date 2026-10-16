---
created_at: 2026-10-16T20:13:20Z
updated_at: 2026-10-16T20:13:20Z
---

## Why

An installed coding agent (`cli/claude-code/opus`, `cli/codex/gpt-6-sol`)
can run main and child turns, but it cannot advise. `/advisor` hides every
`cli/...` row that `/model` offers. Typing one directly fails too:
`AdvisorTarget::parse` reads `cli` as a provider name, and validation rejects
it as undeclared. A profile advisor whose model is `cli/...` resolves, but
then fails on every call. The advisor route always calls the resolved
provider with a `cli/...` model id that the provider has never heard of.

The owner asked on 2026-10-16 for a CLI agent as advisor. The advisor should
use the same execution abstraction as main and child turns: the runtime
library's `ExternalAgentBackend`, selected by `cli_agent::turn_execution`.
Agent turns and advisor calls then run through one code path. The CLI's own
settings (`[harness.<kind>]`) apply to both.

## What Changes

- An advisor target can be an installed agent, `cli/<kind>/<model>`, in
  configuration (`advisor = "cli/claude-code/opus"`), in the session override,
  and in `/advisor`. A `cli/` value whose kind is in the catalog is always an
  installed agent, never a provider named `cli`.
- An installed-agent advisor resolves like a model advisor: top-level
  settings and no profile's settings or instructions. It keeps the main
  session's resolved provider for model identity and limits, the same way
  `/model` keeps the profile's provider when it selects a `cli/...` model.
  `[harness.<kind>]` executable, arguments, and environment apply.
- The advisor route picks how it executes the way main and child turns do:
  through `turn_execution` on the advisor's resolved config. This also covers
  a profile advisor whose model is `cli/...`. An installed-agent route runs
  one fresh `ExternalAgentBackend` turn per call. That turn has no resume, no
  bridge, no dispatched tools, and the CLI's own tools off, which is read-only
  as defined by "CLI-owned tools are on by default". It carries the reviewer
  prompt as instructions and the transcript as input. A provider route is
  unchanged.
- The CLI's text is the advice. Its reported usage is recorded under the
  advisor attribution, and cost is reported as unknown. Cancellation, the
  deadline, the output bound, and the error rules match the provider path. A
  missing program is reported when the advisor is called; it does not block
  session start.
- `/advisor` lists the same model rows as `/model`, including installed agents
  with their PATH check. The session's own binding is left out. A row the
  advisor cannot use is shown disabled with the reason instead of being
  hidden.
- A vision target cannot be an installed agent. It is refused with a clear
  reason.

## Impact

- Affected specs: `advisor`, `configuration`, `client-surfaces`
- Affected code:
  - `crates/smith-config/src/resolve/{types.rs,agent.rs,load.rs}`: the
    target variant, parsing, validation, and route selection
  - `crates/smith-cli/src/runtime_host.rs`: the advisor request inherits the
    main provider for an installed-agent target
  - `crates/smith-runtime/src/factory/provider/children.rs`:
    `prepare_advisor_route` chooses the execution
  - `crates/smith-runtime/src/advisor.rs`: `AdvisorRoute` execution and
    `consult` on a backend
  - `crates/smith-runtime/src/cli_agent/selection.rs`: a plan backend with
    instructions
  - `crates/smith-cli/src/resources.rs`: `/advisor` rows reuse `/model` rows
  - `crates/smith-tui/src/app/resources.rs`: direct `/advisor cli/...`
