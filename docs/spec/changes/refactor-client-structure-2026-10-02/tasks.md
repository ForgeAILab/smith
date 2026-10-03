---
created_at: 2026-10-02T10:35:00Z
updated_at: 2026-10-02T10:40:03Z
completed_at:
---

## 1. Preconditions

- [x] 1.1 Obtain approval of this proposal and a crate name (owner,
  2026-10-02: approved; `smith-client`).
- [x] 1.2 Record fixtures: every local command at 100 and 44 columns, and
  headless text, JSON, and stream-JSON output for the existing flow tests.
  Baseline (owner, 2026-10-02): the last release, `58bc098` (v0.2.15 plus
  docs-only commits), in worktree `../tui-refactor-client` on branch
  `refactor/client-structure`; uncommitted. 425 fixtures under
  `crates/smith-cli/tests/fixtures/{local-commands,headless}/`; recorder in
  `src/main_tests/fixtures.rs` and `src/headless/tests/fixtures.rs`.
  Record: `SMITH_UPDATE_FIXTURES=1 cargo test -p smith-cli --locked --bin smith fixtures_`;
  compare: the same without the variable. Three recordings byte-identical;
  compare mode 30 of 30. Details and the fields the normalizer masks:
  [baseline notes](../../../qa/smith-structure-2026-10-02/refactor-client-structure-baseline.md).
  These values are valid for sections 2 and 3 only: `fix-interaction-defects`
  changes visible text. Before section 4, in this order: (a) rebase the
  branch onto main with that change committed; (b) re-record at the new base
  commit with only the recorder applied, before any section 2 or 3 commit,
  and confirm every changed fixture is explained by `fix-interaction-defects`;
  (c) run compare mode at the branch tip, so sections 2 and 3 are checked
  against the fresh recording. Re-recording at the tip instead would bake a
  rebase mistake into the baseline.
  Done 2026-10-02 before any section 2 work: recorder committed (`ea1f896`),
  branch rebased onto main `99ad421`, re-recorded with only the recorder
  applied (`a8d95a3`); all 12 changed fixtures are explained by
  `show-provider-retry-progress` (headless attempt fields) and
  `fix-interaction-defects` 3.3 (redo and trust confirmation copy). Compare
  mode at the new base: 22 of 22 recorder tests pass.
- [x] 1.3 Confirm the two in-flight TUI changes are committed before stage 4.
  Committed on main as `51bf7c6` (retry progress and interaction defects);
  `show-idle-compaction-summary` has no code.

## 2. Client-neutral accounting

- [x] 2.1 Create the crate with no ratatui or crossterm in its dependency
  closure; add a test that asserts this.
- [x] 2.2 Move the cache projection, usage, pricing, cost, and usage log out
  of `smith-tui`; keep module-internal names.
- [x] 2.3 Replace the mirrored price table with a conversion from the
  `smith-config` catalog type.
- [x] 2.4 Point headless, exit reporting, and local commands at the new
  crate; no headless module imports from `smith_tui`.
- [x] 2.5 Merge the duplicated token and currency formatters.

## 3. Command registry

- [x] 3.1 Define the command table and the routed command type in the new
  crate; move parsing beside it.
- [x] 3.2 Drive completion, `Ctrl+P`, and `/help` from the table.
- [x] 3.3 Dispatch by route; delete the reverse name map and the
  `unreachable!` arms on both sides.
  The four `*-host-boundary` fixtures (`/context 256k`, `/account`,
  `/connect`, `/disconnect` sent straight to the host) recorded the deleted
  runtime guard; that call no longer compiles, so the cases and their 12
  files were removed and a `compile_fail` doctest in
  `smith-client/src/commands.rs` replaces them.
- [x] 3.4 Move `/agent` and `/diff` sub-argument parsing out of the CLI and
  into the table's grammar; remove the dead `accounts` alias.
- [x] 3.5 Add a test that each entry parses its own usage example.

## 4. Typed local results

- [ ] 4.1 Add `LocalResult` and the transitional `Text` variant; carry it in
  the transcript block.
- [ ] 4.2 Convert `/status`, then `/context`, `/help`, `/timeline`, `/goal`,
  `/agent`, `/mcp`, `/skills`, `/diff`, `/review`, the undo, redo, and revert
  previews, and `/diagnostics`; one commit each, fixtures unchanged.
- [ ] 4.3 Render headless goal, child, and restore text from the same
  reports; remove the duplicate formatters.
- [ ] 4.4 Delete renderer branches that inspect titles, headings, glyphs, or
  label text; delete the `Text` variant.
- [ ] 4.5 Provide one label function for child state and durability; use it
  in headless output, local commands, and submission.

## 5. Provider descriptors

- [ ] 5.1 Extend `smith-config` descriptors with the setup flow and the
  connectable flag; remove the duplicate endpoint constant.
- [ ] 5.2 Build `SetupEntry` values in the CLI and pass them to the setup
  surface; remove endpoint, limit, and name literals from `smith-tui`.
- [ ] 5.3 Derive the CLI's connectable-provider list and dispatch from the
  descriptors.
- [ ] 5.4 Use one preflight-request constructor for setup, connect, and
  session start.

## 6. CLI module hygiene

- [ ] 6.1 Replace `use super::*` with explicit imports in production
  modules; enable `wildcard_imports` for the crate.
- [ ] 6.2 Split `handle_local_command` into one report function per command.
- [ ] 6.3 Convert `include!` test splicing to ordinary test modules.

## 7. Verification

- [ ] 7.1 Fixture comparison for every local command and headless format.
- [ ] 7.2 `cargo fmt --all -- --check`, strict Clippy, workspace tests,
  runtime conformance.
- [ ] 7.3 PTY command sweep and startup sweep.
- [ ] 7.4 Update `docs/architecture.md` with the client-side ownership rules.
