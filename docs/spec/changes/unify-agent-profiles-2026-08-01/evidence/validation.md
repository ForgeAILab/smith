# Unified Agent Profiles Validation

Completed at 2026-08-01T22:49:15Z.

## Configuration and migration

- Profiles resolve bounded descriptions and instructions, posture, `main` /
  `child` placement, `profile_order`, and single-parent inheritance.
- Focused tests cover inherited runtime settings, main/child eligibility,
  invalid cycle placement, inheritance cycles, legacy adapters, and legacy/new
  name collisions.
- Setup writes `profile_order` plus the unified profile metadata and no new
  legacy declaration.

## Prompt, authority, and durability

- The selected profile is a dedicated host developer-instruction fragment
  with its exact effective SHA-256 revision.
- Stable Smith fragments remain independent. Custom instructions are present
  in provider context but absent from profile, runtime-policy, and whole-runtime
  debug representations.
- A plan profile that asks for mutation still receives no edit or shell
  capability.
- Child fingerprints record profile name/revision, child placement, posture,
  provider/model, context and prompt revisions, and the read-only ceiling.
- Existing durable follow-up and exact-resume suites pass with this additional
  compatibility evidence.

## Child and client behavior

- An integration test resolves a child-only review profile on another declared
  model, dispatches it through its preflighted route, and proves the root
  provider/model was not used as a hidden fallback.
- TUI tests cover main-profile cycling only at the empty/idle boundary,
  child-profile confirmation, disabled child placement with draft preservation,
  distinct retained child follow-up, and unified reference-picker rendering.
- `/status`, profile pickers, child confirmation, help, README, configuration,
  design, security, and persistence documentation use the unified profile
  terminology and omit raw instruction bodies.

## Commands

- `cargo fmt --all`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features --quiet`
- Focused reruns for runtime transport, all Smith tools, and all Smith TUI tests
  completed the captured workspace output after the long-running command
  handoff.
- The live-provider quota-spending test remains intentionally ignored unless
  its explicit environment is supplied; alternate-model routing is covered by
  the deterministic fake-provider integration test.

## Post-implementation cycle regression

- A report from an existing configuration exposed two migration gaps: omitted
  `profile_order` produced a one-item cycle, and choosing the visible legacy
  `review` adapter incorrectly rebuilt with `--profile review`.
- Omitted order now derives every real main-enabled profile in deterministic
  order, excluding legacy and child-only entries. Legacy picker/cycle entries
  carry a typed resource identity and rebuild through the legacy agent override
  without replacing the selected runtime profile.
- Guided setup now writes a build profile and inherited `-plan` / `-review`
  profiles with an explicit three-item order.
- Focused resolver, TUI routing, CLI selection-preservation, runtime-resource,
  and setup-shape regression tests cover the reported sequence.
