# Tasks

## 1. Runtime projection and bounded summary view

- [ ] 1.1 Expose a completed-idle-compaction marker (interval id, outcome,
  artifact identity, provenance) in the cache-controller snapshot as
  redaction-safe consumer metadata, without summary text.
- [ ] 1.2 Add one bounded, verification-checked summary-body view over the
  existing protected artifact read (`restore_summary_artifact`), reusing
  provenance fields; missing/corrupt references report missing without prose
  fallback.
- [ ] 1.3 Unit tests: completed/failed outcomes, missing and corrupt
  artifacts, bounded metadata without summary text, idempotent replay.

## 2. TUI presentation

- [ ] 2.1 Distinct idle-compaction completion notice in the transcript;
  failure variant; neither enters canonical history.
- [ ] 2.2 One-per-resume on-return presentation when the last idle interval
  completed compaction, naming the `/summary` command and the
  continues-from-summary fact.
- [ ] 2.3 `/summary` command rendering the verified body plus provenance,
  labeled non-authoritative, with a bounded failure rendering.
- [ ] 2.4 Reducer and render tests for each surface, including
  no-compaction sessions.

## 3. Headless surfaces

- [ ] 3.1 Text mode: one bounded stderr pointer after a completed turn that
  included idle compaction, honoring the miss-notices style.
- [ ] 3.2 JSON/stream-JSON: bounded `idle_compaction_completed` and
  `summary_available` metadata only.
- [ ] 3.3 Headless output tests proving no summary text in machine output.

## 4. Documentation and verification

- [ ] 4.1 Document the surfaces in `docs/configuration.md` or the surfaces
  doc where compaction is described today.
- [ ] 4.3 `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace`.
