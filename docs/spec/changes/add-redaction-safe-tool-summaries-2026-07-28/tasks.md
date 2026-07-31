---
created_at: 2026-07-29T01:34:31Z
updated_at: 2026-07-29T01:57:16Z
completed_at: 2026-07-29T01:57:16Z
---

## 1. Safe display projection

- [x] 1.1 Add a typed, pure Smith tool-call display projector with explicit
  allowlists for built-in target and numeric/boolean qualifier fields.
- [x] 1.2 Bound projected text, normalize control characters, supply documented
  defaults such as project root `.`, and cover adversarial values.
- [x] 1.3 Keep edit bodies, shell commands, search patterns, unknown argument
  values, and tool results outside the display projection.

## 2. Live and resumed TUI integration

- [x] 2.1 Resolve a live tool call by its stable call ID from the already
  appended canonical in-process history, without enabling raw runtime-event
  arguments or writing the summary to the event journal.
- [x] 2.2 Enrich transcript tool blocks with the typed summary and render a
  compact `Tool(target · qualifiers) · status` row with no result body.
- [x] 2.3 Derive resumed rows through the same projector and retain the
  protected argument-key fallback when no safe summary is available.

## 3. Verification

- [x] 3.1 Add live/replay parity tests for read, list, search, edit, and shell
  targets plus unknown tools.
- [x] 3.2 Add non-disclosure tests proving arbitrary arguments, result content,
  credentials, and control characters do not reach rendered summaries,
  canonical events, journals, or machine output.
- [x] 3.3 Run formatting, strict Clippy, affected crate/workspace tests, Agent
  Runtime argument-redaction conformance, and CodeGraph sync.
