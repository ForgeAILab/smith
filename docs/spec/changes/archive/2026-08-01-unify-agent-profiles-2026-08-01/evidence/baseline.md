# Unified Agent Profiles Baseline

Captured at 2026-08-01T21:51:22Z before implementation.

## Current configuration shapes

```toml
default_profile = "work"

[profiles.work]
provider = "remote"
model = "vendor/model-id"
agent = "build"
```

`[profiles]` currently selects main-run settings only. It has no description,
instructions, posture, placement, or inheritance fields.

```toml
default_agent = "plan"
agent_order = ["plan", "build", "review"]

[agent_modes.audit]
posture = "review"
description = "Evidence-only audit"
```

`[agent_modes]` currently supplies root-only posture/description entries and
the TUI cycles `agent_order` without changing the selected runtime profile.

```toml
[child_agents.inspect]
posture = "plan"
description = "Inspect without mutation"
```

`[child_agents]` currently supplies child-only posture/description entries.
Resolution rejects build posture for a child. Main profiles, root modes, and
child presets occupy different maps, so the same spelling can currently exist
in more than one registry without a collision diagnostic.

## Current runtime and client behavior

- The main prompt contributor receives an `AgentModePrompt` containing only
  the active mode name and posture. Mode behavior uses a fixed host-authored
  developer-instruction fragment with revision
  `smith-prompt-agent-mode-1`.
- Composer child invocation prefixes the task with an ad hoc
  `You are the <preset> read-only child preset` sentence. The child factory
  otherwise reuses the parent's provider, model profile, context policy,
  prompt contributor, and limits.
- The durable child policy fingerprint includes the child spec, provider/model,
  model profile, context-policy revision, prompt revisions, skill names,
  workspace, and read-only flag. It does not carry a resolved agent-profile
  identity.
- Runtime resources expose three separate collections: `profiles`,
  `agent_modes`, and `child_agents`. `/profile` reads the first, idle `Tab`
  cycles the second, and `@` agent completion reads the third.

## Passing baseline checks

- `cargo test -p smith-config agent_modes_are_typed_ordered_and_child_presets_stay_read_only -- --exact --nocapture`
- `cargo test -p smith-runtime prompt::tests:: -- --nocapture` (8 prompt tests)
- `cargo test -p smith-tui app::tests::tab_cycles_only_an_empty_idle_root_agent -- --exact --nocapture`
- `cargo test -p smith-tui references::tests::resolves_files_agents_and_literal_escapes -- --exact --nocapture`

All listed baseline checks passed before implementation.
