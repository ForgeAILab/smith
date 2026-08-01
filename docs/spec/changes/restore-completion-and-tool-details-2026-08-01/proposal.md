---
created_at: 2026-08-01T20:13:38Z
updated_at: 2026-08-01T20:43:03Z
---

## Why

Smith currently suppresses every successful turn terminal, which removes the
useful completion and elapsed-time evidence the user expects. Its built-in tool
projector already knows how to render a `read` target and line window, but live
enrichment is attempted only when `ToolCallRequested` arrives; if canonical
history is not visible at that instant, the completed row remains the generic
`values protected` fallback and hides ordinary operational arguments.

## What Changes

- Restore one quiet attributed `turn · completed in …` transcript notice for
  every successful terminal while preserving cleanup, canonical events, and
  non-success notices.
- Derive completed duration from the canonical start/completion envelope
  timestamps when valid, retain monotonic time for the live working indicator,
  render sub-second terminals honestly, and never invent a duration when the
  evidence is unavailable.
- Retry reviewed tool-display enrichment at the completion boundary so a
  transient request-time history race cannot leave a known built-in on the
  protected fallback after it finishes.
- Show bounded ordinary built-in operation details—especially `read` path,
  offset, and limit; search pattern and scope; and shell command/cwd—after
  applying Smith's credential-key and registered-secret redaction policy.
- Keep runtime events, journals, and machine output raw-argument-free. Keep
  bulk edit bodies and tool-result bodies out of the compact row for
  readability, and retain an honest fallback for unknown or malformed tools.
- Update the terminal design/security contract and deterministic narrow,
  normal, wide, live, completion-race, resume, and redaction tests.

## Impact

- Affected specs: `client-interaction`, `tool-call-display`
- Affected code: `smith-tools` display projection, `smith-runtime` host-local
  redaction/enrichment, `smith-cli` event presentation, `smith-tui` reducer,
  transcript/render tests, `DESIGN.md`, and security documentation
- Compatibility: no runtime-event, journal, session, or machine-output schema
  change; the installed interactive transcript becomes more informative
- Security: credential-shaped fields and exact registered secrets remain
  redacted before display; raw event arguments remain disabled

## Active Change Coordination

- This change supersedes only the transcript-silent successful-terminal
  requirement added by
  `add-project-instructions-and-quiet-turns-2026-08-01`; project-instruction
  discovery and prompt behavior remain unchanged.
- This change expands the reviewed built-in projection from
  `add-redaction-safe-tool-summaries-2026-07-28` and fixes its request-time
  enrichment race without weakening the shared event/journal boundary.

## Approval Boundary

Approval authorizes restored successful completion notices, canonical elapsed
duration projection, completion-time enrichment retry, and bounded display of
ordinary built-in operation arguments after credential redaction. It does not
authorize raw arguments in runtime events or machine output, unbounded edit or
result bodies, disabling persistence redaction, or guessing values for an
unknown tool schema.
