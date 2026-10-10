---
created_at: 2026-10-10T22:30:05Z
updated_at: 2026-10-10T22:36:06Z
completed_at:
---

## 1. Contract

- [x] 1.1 Add `crates/smith-module`: `Module` trait (id, revision,
  description, default, requirements, `mount`), `ModuleContext` (resolved
  settings, user directory, posture, HTTP transport, image binding),
  `Mounted`, and the contribution types. Depends on Agent Runtime crates
  only.
- [x] 1.2 Extend `Contribution` in `smith-runtime/src/harness.rs` with
  `Pipeline` and `StatusItem`, and `ModuleProvenance` with
  `CompiledThirdParty`; update resolution and evidence tests.
- [x] 1.3 Add the mount planner: topological order over requirements, cycle
  and unmet-requirement reporting, mount-failure isolation. Unit tests for
  each outcome.

## 2. Composition

- [x] 2.1 Add the single compiled-in module list in `smith-cli`, empty, and
  pass it into runtime construction.
- [x] 2.2 Feed mounted contributions into the factory: tools to the
  registry, pipeline components to the harness pipeline, observers to the
  observer chain, each recorded as a `ModuleSpec`.
- [ ] 2.3 Rebuild the composition from the mounted set on the existing
  safe-boundary reconfigure; test off-then-on equals never-off.
- [x] 2.4 Confirm existing fixtures pass unchanged with the empty list.

## 3. Configuration

- [ ] 3.1 Add the `modules.<id>.enabled` table to the config model and
  layered resolution with provenance, including the profile layer.
- [ ] 3.2 Reject unknown ids with the known list; report on-but-not-built.
- [ ] 3.3 Alias resolution for ported features, with the same-layer conflict
  error. Unit tests per layer and per spelling.

## 4. Ports

- [ ] 4.1 `crates/modules/image-generation`: move `image_api.rs`,
  `image_history.rs`, and the tool construction from
  `factory/capabilities.rs` into the module; binding resolution stays in
  `factory/provider.rs` and reaches the module through the context; cargo
  feature `module-image-generation`; alias `tools.image_generation.enabled`.
- [ ] 4.2 `crates/modules/budget-notice`: move `budget_notice.rs` and its
  construction out of `factory/construction.rs`; cargo feature
  `module-budget-notice`; default follows semantic summary as today.
- [ ] 4.3 Status item data type in `smith-client`, rendering and truncation
  in `smith-tui`, omitted in headless output; `budget-notice` contributes
  its item.
- [ ] 4.4 Existing image-generation and budget-notice fixtures pass
  unchanged with default features.

## 5. Surfaces

- [ ] 5.1 `/modules` listing with state, deciding key and layer, and
  first-party or third-party.
- [ ] 5.2 `/modules <id> on|off` through the previewed user-config edit and
  safe-boundary apply, with rollback, override reporting, and the not-built
  and unknown-id refusals.
- [ ] 5.3 `smith config modules` headless listing.
- [ ] 5.4 Reducer, render, and command tests for every listed state.

## 6. Structure and verification

- [ ] 6.1 Architecture tests: module crate dependency rules, one cargo
  feature per first-party module, one compiled-in list, no link-time
  registration.
- [ ] 6.2 CI job building and smoke-testing `--no-default-features`.
- [ ] 6.3 Test-only third-party module crate proving the
  `CompiledThirdParty` path end to end.
- [ ] 6.4 Document the contract and the add-a-module steps in `docs/`.
- [ ] 6.5 Full gate with `--no-fail-fast`; update `README.md` feature list.
