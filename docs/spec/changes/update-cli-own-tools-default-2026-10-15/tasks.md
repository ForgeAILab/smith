---
created_at: 2026-10-16T00:10:07Z
updated_at: 2026-10-16T00:10:07Z
completed_at:
---

## 1. Configuration default

- [ ] 1.1 `resolve/harness.rs`: an absent `allow_own_tools` resolves to
  `true` (built-in source). Update the doc comments in `model.rs`,
  `resolve/types.rs`, and `harness.rs` that say "off unless enabled".
- [ ] 1.2 Config tests: absent resolves on; owner `false` resolves off;
  a project layer setting either value fails naming the key; `smith config
  explain harness.claude-code.allow_own_tools` reports the built-in default.

## 2. CLI invocation

- [ ] 2.1 Claude Code with own tools on passes
  `--permission-mode bypassPermissions` instead of `acceptEdits`. Owner
  `args` stay after every Smith flag.
- [ ] 2.2 Codex with own tools on passes
  `--dangerously-bypass-approvals-and-sandbox` and no `--sandbox`. Off keeps
  `--sandbox read-only`.
- [ ] 2.3 Update the argv unit tests in `cli_agent/invocation.rs` and
  `selection.rs`, the stand-in's flag parser
  (`crates/smith-cli/examples/cli_stand_in`), the contract tests, and the
  live canaries' expected flag lists.

## 3. Narrowing

- [ ] 3.1 `factory/construction.rs`: the root passes
  `!posture.is_read_only()` to `plan.backend` instead of `true`. Children
  keep `write_capable`.
- [ ] 3.2 Tests: a `plan` root and a read-only child each get the off
  invocation with the default setting. An owner `false` is never widened.

## 4. Docs

- [ ] 4.1 `docs/configuration.md`: the installed-agents section says the
  CLI's own tools are on by default, shows `allow_own_tools = false`, and
  shows narrowing Claude Code through `args`.
- [ ] 4.2 `docs/security.md`: installed coding agents run their own tools
  without approval by default; name the off switch and the posture
  narrowing.
- [ ] 4.3 Release notes for the next version flag the widened default.

## 5. Verification

- [ ] 5.1 Gate: `smith-config`, `smith-runtime`, `smith-cli` tests with
  `--no-fail-fast`, clippy, fmt.
- [ ] 5.2 Live (owner config, installed binary): an orchestrator turn on
  `cli/claude-code/opus` runs WebFetch and `python3 -c` without a denial;
  with `allow_own_tools = false` both are denied again.
- [ ] 5.3 Live: `codex_live_flow` passes with the new Codex flag.
