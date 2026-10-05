---
created_at: 2026-10-05T20:06:57Z
updated_at: 2026-10-05T20:06:57Z
completed_at:
---

## 1. Registry

- [ ] 1.1 Add the built-in feature registry in `smith-config` with id,
  description, controlling key, default, and apply timing per entry.
- [ ] 1.2 Register `idle-compaction`, `resume-capsule`, `handoff-checkpoint`,
  and `image-generation` against their existing keys.
- [ ] 1.3 Resolve each entry to an effective value with provenance from the
  existing layered resolution; unit tests per entry and per layer.
- [ ] 1.4 Architecture test: every registry key exists in the config model and
  no two entries share a key.

## 2. Surfaces

- [ ] 2.1 `/features` lists every entry with on/off, key, and provenance.
- [ ] 2.2 `/features <id> on|off` prepares a user-config edit, shows the
  preview in the shared confirmation, commits, and applies through the
  safe-boundary reconfigure; rollback on a failed apply.
- [ ] 2.3 Report, without writing, when a higher-precedence layer overrides the
  user value; refuse unknown ids with the list of valid ones.
- [ ] 2.4 `smith config features` prints the same listing headlessly.
- [ ] 2.5 Reducer, render, and command tests for list, toggle, override,
  unknown id, and rollback.

## 3. Module records

- [ ] 3.1 Add one `BuiltIn` `ModuleSpec` per enabled feature with its
  contributions through `HarnessSpec::with_module`.
- [ ] 3.2 Test that a disabled feature contributes no module and that resuming
  a session saved by 0.3.9 still succeeds after the composition revision
  changes.

## 4. Documentation and verification

- [ ] 4.1 Document `/features` and the registry in `docs/configuration.md`.
- [ ] 4.2 `cargo fmt --all --check`, workspace Clippy with `-D warnings`
  (stable and 1.88), `cargo test --workspace --locked --no-fail-fast`.
