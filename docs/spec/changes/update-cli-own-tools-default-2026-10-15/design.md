## Context

A harness turn runs `claude -p ...` or `codex exec ...` with no terminal
attached. Neither CLI can show a permission prompt there, so any action its
policy would ask about fails. The owner wants an installed agent to work out
of the box and wants limits to be opt-in.

## Goals / Non-Goals

- Goals: a `cli/...` profile with no `[harness]` table can fetch, search,
  edit, and run commands. One switch turns that off. Read-only postures
  never gain write or command authority through the CLI.
- Non-Goals: routing the CLI's permission prompts into Smith's approval
  policy; mapping `--approval` onto the CLI's own tools; changing the
  bridge.

## Decisions

- **On is the CLI's no-prompt mode.** Claude Code gets
  `--permission-mode bypassPermissions`. Codex gets
  `--dangerously-bypass-approvals-and-sandbox` instead of `--sandbox`. Codex
  0.162.1 accepts both flags together, but one flag that states the mode is
  clearer. Codex rejects a repeated `--sandbox`, so owner `args` cannot
  narrow Codex the way they narrow Claude Code.
  - Alternative: `acceptEdits` plus a pre-allow list (`WebFetch`,
    `WebSearch`, `Bash`). It ends up as broad as bypass, but Smith would own
    a list that drifts with each CLI release. Rejected.
  - Alternative: Claude's `auto` mode (a classifier approves safe actions).
    It depends on the account and model, so a fresh install would not behave
    predictably. Rejected for the default; owners can pick it through
    `args`.
  - Alternative: `--permission-prompt-tool mcp__smith__...`, which routes
    prompts into Smith's approval UI. Owner: too much for now.
- **Off keeps today's flags exactly.** The recorded fixtures, contract
  tests, and the "no-own-tools mode" scenarios stay valid.
- **Narrowing is one boolean at both call sites.** Root uses
  `!posture.is_read_only()`. Children keep `write_capable`. Both are
  combined with the owner's setting by the existing `allow_own_tools &&
  narrowed`, so nothing can widen the owner's choice.
- **Owner `args` stay last for Claude Code.** Commander keeps the last value
  of a single-value flag. Checked on 2.1.301: a later
  `--permission-mode default` overrides an earlier `bypassPermissions`. That
  ordering is the documented way to narrow.

## Risks / Trade-offs

- Upgrading silently widens what existing `cli/...` profiles may do. Release
  notes must say so.
- Claude Code refuses `bypassPermissions` when run as root or when an
  organization's managed settings disable it. The turn then fails with the
  CLI's own error, which Smith already reports as a failed turn. No fallback
  mode is attempted.
- A `plan` root that relied on `allow_own_tools = true` to run commands loses
  that ability. This is intended.

## Migration Plan

No config migration. Owners who want today's behavior set
`[harness.claude-code] allow_own_tools = false` (or `codex`).

## Open Questions

- None blocking. Whether project layers may *disable* (narrow) the setting
  is left for later; it stays owner-only here.
