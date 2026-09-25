---
created_at: 2026-09-22T14:21:25-04:00
updated_at: 2026-09-22T14:45:32-04:00
completed_at: 2026-09-22T14:45:32-04:00
---

# Tasks: Add Anthropic adaptive effort controls

## 1. Configuration contract

- [x] 1.1 Add the additive `anthropic-effort` reasoning dialect and include it
      in parser diagnostics and serialization coverage.
- [x] 1.2 Document exact mandatory-on Fable 5.1 reasoning metadata and preserve
      model/provider-specific provenance.

## 2. Runtime policy and request path

- [x] 2.1 Resolve explicitly configured Anthropic effort ladders and defaults
      without endpoint or model-name inference.
- [x] 2.2 Preserve the neutral reasoning selection through the Smith dialect
      wrapper for Agent Runtime's native Anthropic adapter.
- [x] 2.3 Add deterministic tests for mandatory-on behavior, the five-level
      effort ladder, supported and unsupported selection, and request
      pass-through.

## 3. Client behavior

- [x] 3.1 Confirm `/effort` advertises exactly the configured ladder plus
      provider default and `/think off` remains unavailable.
- [x] 3.2 Confirm `--effort` and config explanation retain existing validation
      and source labeling for the Anthropic dialect.

## 4. Validation and local activation

- [x] 4.1 Run formatting, focused config/runtime/CLI/TUI tests, workspace
      Clippy with warnings denied, and workspace tests.
- [x] 4.2 Keep live Fable 5.1 inference out of the validation path; rely on the
      deterministic wire contract and the already completed healthy-route
      compatibility probe.
- [x] 4.3 After the new binary passes, add the reviewed reasoning metadata to
      the user's dddai Fable 5.1 config and validate local resolution without
      issuing a Fable 5.1 request.
