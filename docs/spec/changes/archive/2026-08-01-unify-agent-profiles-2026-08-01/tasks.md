---
created_at: 2026-08-01T21:41:00Z
updated_at: 2026-08-01T22:49:15Z
completed_at: 2026-08-01T22:49:15Z
---

## 0. Approval and Baselines

- [x] 0.1 Approve this proposal, design, and all five capability deltas before
  implementation.
- [x] 0.2 Capture current profile precedence, root-mode cycling, child-preset
  invocation, prompt fragments, child policy fingerprints, and setup/inventory
  fixtures.
- [x] 0.3 Record compatibility examples for existing `[profiles]`,
  `[agent_modes]`, and `[child_agents]`, including same-name collisions.

## 1. Unified Profile Configuration

- [x] 1.1 Extend `ProfileSection` with bounded description, posture,
  instructions, `main`/`child` availability, and single-parent inheritance.
- [x] 1.2 Resolve inheritance with cycle/depth/missing-parent checks and
  field-level provenance before provider, credential, session, or terminal I/O.
- [x] 1.3 Replace resolved root modes and child presets with one profile
  inventory while preserving exact active profile and eligible cycle order.
- [x] 1.4 Add one-release legacy adapters, source-explainable deprecation
  diagnostics, and fail-closed name-collision behavior.
- [x] 1.5 Update setup transactions and examples to write only the unified
  profile shape without weakening owner/project trust restrictions.

## 2. Main-Agent Prompt and Runtime Composition

- [x] 2.1 Compose selected profile name, posture, and optional instructions as
  one independently revisioned developer-instruction fragment after stable
  Smith host policy.
- [x] 2.2 Apply profile posture through the existing authority-narrowing
  ability view and prove profile text/settings cannot grant tools or approval.
- [x] 2.3 Route startup, `--profile`, `/profile`, resume, and eligible idle
  cycling through one atomic safe-boundary profile rebuild.
- [x] 2.4 Preserve independent Smith, project-instruction, skill, memory, and
  profile revisions in context plans and cache diagnostics.

## 3. Profile-Based Child Composition

- [x] 3.1 Replace child-only preset lookup with explicit child-enabled profile
  lookup and show profile instructions summary, provider/model, limits,
  read-only posture, and spend in confirmation.
- [x] 3.2 Generalize child preflight to resolve the selected profile's declared
  provider/model and credentials through the standard Smith composition path.
- [x] 3.3 Intersect parent authority, the depth-one/read-only child ceiling,
  and profile posture; reject or explain inapplicable widening settings.
- [x] 3.4 Persist effective profile name/revision/provenance in child policy
  fingerprints and prove spawn, follow-up, resume, and incompatibility remain
  exact and idempotent.

## 4. Client Surfaces and Migration UX

- [x] 4.1 Make TUI/CLI inventory present profiles as agent presets with main,
  child, or both placement labels and active/legacy/provenance summaries.
- [x] 4.2 Use the same profile registry for `/profile`, idle main-profile
  cycling, and `@profile <task>` completion without confusing retained child
  IDs with spawn presets.
- [x] 4.3 Update `/status`, `/help`, setup review, empty states, errors, and
  headless diagnostics to use the unified terminology.
- [x] 4.4 Add narrow/normal/wide render and interaction tests for selection,
  confirmation, disabled placement, legacy migration, and collision failure.

## 5. Validation and Documentation

- [x] 5.1 Add configuration, prompt, authority, alternate-model child,
  persistence/recovery, replay, and interactive/headless parity tests.
- [x] 5.2 Update `DESIGN.md`, configuration/setup reference, README, security
  model, persistence documentation, and examples with the new profile model
  and migration guidance.
- [x] 5.3 Run format, warning-denied Clippy, workspace/all-feature tests,
  strict spec validation, diff hygiene, and any opt-in provider scenarios
  needed to verify alternate-model child composition.
