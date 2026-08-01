---
created_at: 2026-08-01T20:13:38Z
updated_at: 2026-08-01T20:43:03Z
completed_at: 2026-08-01T20:43:03Z
---

## 0. Approval and Baselines

- [x] 0.1 Approve this proposal, design, and both capability deltas before
  implementation.
- [x] 0.2 Freeze the current transcript-silent successful terminal, sub-second
  duration rendering, request-only tool enrichment, and built-in/fallback
  display fixtures.
- [x] 0.3 Record the observed live failure where a completed built-in `read`
  remains `read(limit, offset, path · values protected) · ok` even though the
  reviewed projector can render its exact path and line window.

## 1. Honest Successful Completion

- [x] 1.1 Track the canonical `TurnStarted` envelope timestamp beside the
  monotonic live timer and clear both at every terminal/session boundary.
- [x] 1.2 Restore one attributed successful completion notice and render its
  canonical elapsed duration when the timestamp interval is valid.
- [x] 1.3 Render sub-second duration without the misleading `0s` label, omit
  duration when evidence is unavailable, and remove the `reasoning only`
  interpretation from the completion text.
- [x] 1.4 Preserve streaming closure, speculative discard, activity/todo/work
  reconciliation, usage, non-success notices, journals, and timeline evidence.

## 2. Informative Credential-Redacted Tool Rows

- [x] 2.1 Retry a reviewed built-in projection when the matching completion
  event arrives if request-time canonical lookup produced no display.
- [x] 2.2 Expand the typed built-in projector with bounded ordinary operation
  fields: read path/offset/limit, list scope/flags, search pattern/scope/filter,
  edit target/replace mode, and shell command/cwd/timeout.
- [x] 2.3 Apply credential-shaped key redaction and exact registered-secret
  scrubbing before any canonical argument reaches the local projector; never
  enable raw event or journal arguments.
- [x] 2.4 Keep edit bodies and result bodies outside the compact row, normalize
  controls and bounds, and replace misleading blanket wording with an honest
  unavailable/unknown-schema fallback.
- [x] 2.5 Prove live request/completion races and resumed history converge on
  the same final row without exposing API keys, auth tokens, passwords,
  credentials, registered literals, or control sequences.

## 3. Documentation and Validation

- [x] 3.1 Update `DESIGN.md` and security/tool-display documentation before UI
  implementation to define completion evidence and argument visibility.
- [x] 3.2 Add deterministic reducer and render coverage at narrow, normal, and
  wide widths for visible-answer, tool-only, reasoning-only, sub-second,
  unavailable-duration, enriched, fallback, and secret-redacted states.
- [x] 3.3 Run formatting, warning-denied workspace/all-feature Clippy, full
  workspace/all-feature tests, targeted host/TUI/tool tests, strict spec
  validation, CodeGraph sync, and diff hygiene.
- [x] 3.4 Commit, reinstall `smith` from `crates/smith-cli`, and verify the
  installed release binary after all deterministic gates pass.
