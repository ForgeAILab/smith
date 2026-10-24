---
created_at: 2026-10-23T00:00:00-04:00
updated_at: 2026-10-24T10:33:43Z
---

# Proposal: Add manual conversation compaction

## Why

Smith has no slash command for requesting semantic compaction before the
automatic pressure or inactivity boundary. Users cannot explicitly summarize
older conversation history while retaining the recent turns.

## What Changes

- Add `/compact` to parsing, help, slash completion, and the command palette.
- Run the existing protected semantic-compaction operation at a safe idle
  boundary, using its existing summary policy, usage accounting, persistence,
  and once-per-interval admission.
- Report completion, insufficient eligible history, an already attempted or
  busy interval, shutdown, or failure without submitting a user/model turn.
- Preserve original canonical history and the last valid summary on failure.
- Follow-up (2026-10-24): name each refusal cause and when retrying works;
  let an explicit request retry a durable gate the runtime has not consumed
  in this process (for example after a restart); report already-covered
  history as no eligible history rather than a failure.

Automatic summary enablement restores behavior already required by the
runtime-integration and prompt-cache specs and is a separate bug fix.

## Impact

- Affected specs: `client-surfaces`.
- Affected code: `smith-client` command registry, `smith-runtime` host boundary,
  `smith-cli` command execution, and focused parser/runtime/TUI tests.
- The shared runtime currently attributes explicit idle-boundary summary
  attempts to `cache_idle_compaction`; reuse that accounting rather than
  inventing a purpose that the pinned runtime cannot persist or restore.
- No new configuration keys, provider adapters, dependencies, or summary-body
  presentation.
