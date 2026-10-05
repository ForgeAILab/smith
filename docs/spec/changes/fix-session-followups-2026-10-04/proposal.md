---
created_at: 2026-10-05T01:34:57Z
updated_at: 2026-10-05T01:34:57Z
---

## Why

The 0.3.7 work left five small inconsistencies, listed for the owner after
the release prep and approved to fix before pushing ("i think lets fix
everything then push", 2026-10-04):

- The exit report's usage line counts provider requests as turns and writes
  `turn(s)`: a one-prompt session with three tool calls printed `4 turn(s)`
  while `/resume` called it `1 turn`.
- A resumed session that used two models prints no cost line, because its
  restored usage records carry no model identity. The usage log already
  records each session's per-model counters (schema v5).
- The piped `smith sessions list` still lists sessions where nothing was
  typed, so the Claude Code plugin shows them; the terminal table and the
  plugin call the last column the opening prompt, but it is the latest one.
- Empty sessions are hidden but their files stay on disk forever.
- Setup's GLM quick start proposes `glm-5.2`; `glm-5.3` is current on the
  Z.AI Coding Plan with the same published limits. The truth spec still
  describes the quick start as `glm-4.7`.

## What Changes

- The usage line counts conversation turns the user started and writes
  `1 turn` / `N turns`.
- On resume, restored usage is attributed per model from the project's usage
  log when its last record for the session matches the restored totals;
  otherwise the existing manifest rule applies.
- Both forms of `smith sessions list` omit sessions without a user message.
  The terminal column reads `LATEST PROMPT`, and the plugin's skill and
  command docs say latest prompt.
- When the interactive surface ends a session with no user message, its
  files are removed.
- User-visible counts drop `(s)` (`1 agent`, `1 compaction`), and the
  catalog model list in setup and `/connect` uses compact sizes; both were
  found in the live check of these follow-ups.
- The GLM quick start proposes `glm-5.3` from a new trusted catalog record
  (same limits as 5.2; catalog revision bumped); 5.2 and 4.7 records stay
  for existing configurations.

## Impact

- Affected specs: client-surfaces, usage-accounting, configuration.
- Affected code: `smith-client` `status.rs`, `usage_log.rs`; `smith-cli`
  `runtime_host.rs`, `tui_driver.rs`, `resources.rs`; `smith-runtime`
  session paths; `smith-config` `setup.rs`; `plugins/claude-code`.
- Fixtures that show the usage line or the setup quick start are
  re-recorded. No configuration or session format changes; the usage log
  schema stays at 5.
