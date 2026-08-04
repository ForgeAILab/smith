---
created_at: 2026-08-01T21:41:00Z
updated_at: 2026-08-01T22:49:15Z
---

## Why

Smith currently splits one user concept across three registries. A named
`[profiles.<name>]` selects provider, model, and run policy for the main agent;
`[agent_modes.<name>]` changes only the main agent's posture; and
`[child_agents.<name>]` is usable only when spawning a child. The split makes
an agent preset incomplete and prevents users from defining one coherent
configuration that works for either the main agent or a subagent.

Profiles should instead describe an agent: its instructions, behavioral
posture, model/runtime preferences, limits, and where the preset may be used.
The host still owns safety policy, so a profile can narrow behavior and select
declared resources but cannot grant authority.

## What Changes

- **BREAKING** Make `[profiles.<name>]` the single declarative agent-preset
  type for both main-agent selection and explicit child creation.
- Add bounded profile metadata and instructions, an authority-narrowing
  posture, explicit `main`/`child` availability, and single-parent profile
  inheritance for reusable provider/model baselines.
- Apply a profile to the main agent through startup selection, `--profile`,
  `/profile`, setup defaults, and idle profile cycling.
- Apply a child-enabled profile through `@name <task>` while resolving its
  prompt, provider/model preferences, and limits through the same typed profile
  path used by the main agent.
- Compose profile instructions as an independently revisioned developer
  instruction after Smith's stable host policy. They guide behavior but never
  replace Smith's system identity, project trust policy, approval checks, or
  tool authority.
- Replace separate `agent_modes` and `child_agents` declarations with
  compatibility adapters and deprecation diagnostics for one transition
  release. Existing run profiles continue to select the main agent.
- Keep effective child authority equal to the intersection of parent
  authority, the host's child ceiling, and the selected profile. This change
  does not make write-capable or nested children available.

## Impact

- Affected specs: `configuration`, `harness-policy`, `child-agents`,
  `client-interaction`, `prompt-cache`
- Affected code: `smith-config` file/resolved models and provenance,
  `smith-runtime` prompt/factory/delegation composition, `smith-cli` setup and
  host resources, `smith-tui` profile selection and `@` completion, tests, and
  configuration/product/security documentation
- Compatibility: existing `[profiles]` remain valid main-agent selections;
  legacy `[agent_modes]` and `[child_agents]` are accepted with warnings for
  one transition release and are not silently merged across name collisions
- Security: profile text and settings remain non-authoritative; child
  composition cannot widen workspace, trust, permissions, approval, or the
  depth-one/read-only child ceiling
- Cache behavior: a profile instruction or effective profile revision changes
  the exact prompt and child-policy fingerprint without changing independent
  Smith or `AGENTS.md` fragment revisions

## Active Change Coordination

- `add-smith-agent-harness-2026-07-23` remains authoritative for layered
  configuration, model-profile resolution, and one Smith runtime factory.
  This change extends its named run profiles rather than creating another
  provider or context planner.
- `add-agent-first-workflow-ux-2026-07-31` introduced root modes and child
  presets. This change supersedes only that split registry; posture narrowing,
  explicit child confirmation, and the transcript-first interaction remain.
- `integrate-resumable-child-sessions-2026-07-31` remains authoritative for
  durable child identity, follow-up, resume, and exact policy compatibility.
  The selected profile revision becomes additional compatibility evidence.
- `add-project-instructions-and-quiet-turns-2026-08-01` remains authoritative
  for root `AGENTS.md` discovery. Agent-profile instructions are a separate
  fragment with separate source and revision.

## Delivery Slices

1. Extend the typed profile schema, inheritance, provenance, availability,
   migration diagnostics, inventory, and setup output.
2. Replace root-mode prompt composition with one resolved profile instruction
   fragment and profile-derived posture/capability narrowing.
3. Resolve explicit child creation from the same profile registry and carry
   exact profile identity through child policy fingerprints and persistence.
4. Unify main/child selection in the CLI and TUI, then update fixtures,
   documentation, and compatibility coverage.

## Approval Boundary

Approval authorizes one reusable profile model for main and read-only direct
child agents, including additive profile instructions and profile-selected
declared provider/model preferences. It does not authorize arbitrary complete
system-prompt replacement, profile-granted permissions or approvals,
write-capable children, grandchildren, remote profile downloads, executable
profile hooks, or removal of legacy configuration without the documented
transition release.

Approved by the user for implementation on 2026-08-01.
