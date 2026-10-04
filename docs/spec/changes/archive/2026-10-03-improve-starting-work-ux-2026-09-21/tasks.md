---
created_at: 2026-09-21T00:00:00Z
updated_at: 2026-09-22T03:52:10Z
completed_at: 2026-09-22T03:52:10Z
---

## 1. Evidence and design
- [x] 1.1 Run the baseline feature suite and reproduce startup UX issues.
- [x] 1.2 Write the checklist, scoped proposal, and acceptance scenarios.
- [x] 1.3 Obtain approval of the startup presentation/interaction changes (user: “Approve and implement”).
- [x] 1.4 Update DESIGN.md before UI implementation.

## 2. Implementation
- [x] 2.1 Restore highlighted-command execution and improve command search.
- [x] 2.2 Correct picker empty states and keep state labels visible.
- [x] 2.3 Add the empty-session guide and make help open from its beginning.
- [x] 2.4 Preserve non-secret field values when navigating backward in setup.

## 3. Verification
- [x] 3.1 Add regression coverage for the reproduced integration failures.
- [x] 3.2 Walk all local commands and the setup-to-first-prompt journey.
- [x] 3.3 Inspect 100×32, 80×24, and 44×16 captures and no-color behavior.
- [x] 3.4 Run formatting, Clippy, relevant suites and the final workspace suite.
  Includes the baseline formatting differences and two behavior-preserving
  `collapsible_if` cleanups required by the repository's warnings-as-errors check.
- [x] 3.5 Complete the feature checklist and record remaining external checks.

Verification: 1,653 workspace tests and 222 runtime conformance tests passed;
6 workspace tests remain intentionally ignored. All 40 startup checks and
26 command smoke checks passed. Final evidence and external-service/device
limitations are in [the audit report](../../../qa/smith-ux-2026-09-21/report.md).
