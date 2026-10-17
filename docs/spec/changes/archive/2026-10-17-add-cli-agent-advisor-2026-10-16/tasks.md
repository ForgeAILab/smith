---
created_at: 2026-10-16T20:13:20Z
updated_at: 2026-10-17T20:03:08Z
completed_at: 2026-10-17T20:03:08Z
---

## 1. Configuration
- [x] 1.1 `AdvisorTarget::InstalledAgent { kind, model }`, displayed as
      `cli/<kind>/<model>`. `parse` checks `parse_cli_model_id` before the
      provider split. Update every match on `AdvisorTarget`.
- [x] 1.2 `validate_advisor` accepts an installed agent. `validate_vision`
      rejects one with the image-request reason.
- [x] 1.3 `advisor_route_selection` contributes only `model` for an
      installed agent. The self-advice skip covers a matching main
      `cli/...` model.
- [x] 1.4 `runtime_host`: an installed-agent advisor request sets the
      provider to the main session's resolved provider. The session override
      parses `cli/...` targets.

## 2. Runtime
- [x] 2.1 `CliAgentPlan` can build a backend that carries instructions.
- [x] 2.2 `AdvisorRoute.execution` (`Provider` | `InstalledAgent(plan)` |
      `MissingProgram`), filled by `prepare_advisor_route` from
      `cli_agent::turn_execution` on the advisor config. The
      `matches_selection` check handles the new variant.
- [x] 2.3 `consult` dispatches on the execution. The installed-agent branch
      runs one fresh `ExternalAgentBackend` turn as described in design.md.
      Transcript, budget, cancellation, deadline, output bound, and
      `AdvisorUsage` are shared with the provider branch.

## 3. Client
- [x] 3.1 The `/advisor` model rows reuse the `/model` builder
      (`model_entries` + `append_cli_agent_models`) and are re-marked:
      override-based `active`, the own binding dropped, and unusable rows
      disabled with a reason.
- [x] 3.2 Direct `/advisor cli/<kind>/<model>` is accepted when the row is
      offered and enabled.

## 4. Tests
- [x] 4.1 Config: parse round-trips, top-level and profile installed-agent
      advisors resolve with the main provider, self-advice is skipped, an
      unknown kind is reported, and a vision installed agent is rejected.
- [x] 4.2 Runtime, using a fixture CLI executable as the existing
      `cli_agent` tests do: advice is returned, instructions and transcript
      arrive, own tools are off, there is no resume across calls, usage is
      recorded, failure and empty answers become tool errors, a missing
      program becomes a tool error, and cancellation ends the call.
- [x] 4.3 A profile advisor whose model is `cli/...` runs on the CLI.
- [x] 4.4 Resources: `/advisor` lists the CLI rows with PATH-disabled
      states and excludes the session's own binding. Direct selection
      reaches `SelectionCommand::Advisor`.

## 5. Verification
- [x] 5.1 Full gate (`/Volumes/Data/smith-test-tmp/gate.sh`).
- [x] 5.2 Live check: in the TUI, `/advisor` shows `cli/claude-code/*`;
      choose `cli/claude-code/sonnet`, then ask the main model to call
      `advisor` and see the CLI's advice and the usage in `/status`.
