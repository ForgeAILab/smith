---
created_at: 2026-10-05T20:06:57Z
updated_at: 2026-10-05T20:06:57Z
completed_at:
---

## 1. Fork write

- [ ] 1.1 Find where parent-scoped approval grants and installed-agent CLI
  session ids live (`extension_state` namespaces or host memory) and list
  every namespace as copied or cleared before writing code.
- [ ] 1.2 Write the forked snapshot: new id, copied history and
  history-derived state, fresh usage ledger and identity counters, no
  checkpoint, journal, task spool, or change journal; copy the shell sidecar.
- [ ] 1.3 Write the lineage file (parent id, parent turn count) beside the
  snapshot, owner-only, removed with the session's other files.
- [ ] 1.4 Refuse while a turn runs, a child runs, or a child result is
  undelivered, and for installed-agent sessions; each refusal names its reason.
- [ ] 1.5 Tests: copy/fresh split field by field, refusals, the original is
  unchanged and still resumable, a crash mid-fork leaves no half session.

## 2. Artifacts

- [ ] 2.1 `SmithArtifactStore::read` accepts the owner or an ancestor from the
  requesting session's lineage chain; bounded chain length.
- [ ] 2.2 Tests: fork reads parent tool output and the parent's idle summary
  through `artifact-read` and resume; a sibling fork and an unrelated session
  are refused.
- [ ] 2.3 Test that the fork's first request has the same history bytes as the
  parent's next request would.

## 3. Surfaces

- [ ] 3.1 `/fork` in the client command list; the TUI switches to the new
  session and reports both ids.
- [ ] 3.2 `--fork-session` with `--resume [<id>]`, TUI and headless; rejected
  without `--resume`.
- [ ] 3.3 "forked from <id>" in the header and the `/resume` row.
- [ ] 3.4 Command, CLI parsing, render, and headless tests.

## 4. Documentation and verification

- [ ] 4.1 Document `/fork` and `--fork-session`.
- [ ] 4.2 Live check: fork after a few turns with offloaded tool output, ask
  the fork to read that output, confirm the original is unchanged.
- [ ] 4.3 `cargo fmt --all --check`, workspace Clippy with `-D warnings`
  (stable and 1.88), `cargo test --workspace --locked --no-fail-fast`.
