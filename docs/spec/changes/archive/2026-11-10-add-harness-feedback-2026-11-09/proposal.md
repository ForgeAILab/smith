---
created_at: 2026-11-09T21:37:36Z
updated_at: 2026-11-11T03:43:10Z
---

## Why

When Smith's harness gets in the agent's way, nobody hears about it. A tool
can return something wrong or confusing, a capability can be missing, an
approval can block a reasonable step, or the user can get frustrated with how
Smith behaved. The agent notices these in the moment, but they disappear when
the session ends.

The owner wants the agent to report harness problems as they happen, so they
can be found and repaired. Claude Code's `SendFeedback` tool works this way:
the model drafts a structured report at a high-signal moment, the report is
queued locally, and nothing is sent without the user's approval.

## What Changes

- Add a compiled-in `harness-feedback` module that contributes one
  `harness_feedback` tool. The agent calls it to report a problem with Smith
  itself, not with the user's project. The tool takes a kind (`bug`,
  `missing_capability`, `idea`), an area, a one-line title, and labeled fields
  (what happened, what the user said, repro).
- While enabled, the tool is pinned in every turn without activation, the
  same way `view_image` is when the vision module is mounted. That covers
  root turns, child agents (including read-only children) and headless runs.
  The CLI bridge already serves every tool it does not exclude, so `cli/*`
  turns receive it too. It declares no effects, so Smith never prompts before
  recording, including under `--approval deny`. A CLI's own permission
  settings can still ask before a bridged call.
- Smith attaches the evidence the model cannot be trusted to supply: Smith
  version and commit, OS, profile, provider and model, session, turn and call
  ids, child id, workspace, and the most recent failed tool calls Smith ran
  in the session (name and bounded error text). For `cli/*` turns, Smith can
  see only bridged calls, not the CLI's own tools.
- Reports are Markdown files with frontmatter in an owner-only store at
  `~/.smith/feedback/`. Fields and files are size-bounded. Registered secret
  values are redacted. A repeat of the same issue in the same session raises
  a count instead of adding a file. A session can file at most five new
  reports.
- Tool description and prompt guidance say when to report: a tool or product
  failure that was resolved or abandoned, a missing capability that blocked a
  reasonable request, user frustration with Smith, or the agent noticing that
  the harness pushed it into a mistake. They also say what not to report: the
  user's own code bugs, or task difficulty Smith did not cause. The agent must
  not announce reports or ask the user about them.
- Add `/feedback`. With no argument it lists queued reports inline, newest
  first. `/feedback <id>` shows one report and offers an inline select: file
  on GitHub, dismiss, or keep. After a session starts, a one-line notice
  appears when unviewed reports are queued.
- Filing shows the exact issue title and body first. On confirmation Smith
  opens a prefilled `issues/new` page on `ForgeAILab/smith`; the user submits
  it on GitHub. Smith never posts anything itself. The public body leaves out
  the workspace path and session ids.
- Add `smith feedback list [--json]`, `smith feedback show <id>` and
  `smith feedback dismiss <id>`, so a repair agent can read the queue. The
  JSON output marks report text as untrusted data.
- The existing module switch turns the module off with
  `[modules.harness-feedback] enabled = false`. Off removes the tool and the
  notice. Reports already stored are kept, and `/feedback` still lists them.

## Out of Scope

- Crash capture: panic hooks, crash files and backtraces. That can be a
  separate change.
- Any automatic upload or telemetry endpoint.
- A `smith doctor` command. `/diagnostics` already covers session health.
- Reports Smith files on its own without the agent (for example, after N
  identical tool failures).
- Changes to the versioned headless result envelope.

## Assumptions

- The owner chose "local first, GitHub issues" when this was framed as crash
  reporting. This proposal applies that choice to agent-filed reports: nothing
  leaves the machine until the user submits a GitHub issue themselves.

## Impact

- Affected specs: new `harness-feedback`.
- Affected code: new `crates/modules/harness-feedback` (tool, store);
  `crates/smith-module` (`ModuleContext` gains session identity, a redaction
  handle and recent tool failures); `crates/smith-runtime` (pinned set, a
  tool-failure recorder, redaction backed by the journal's registered-secret
  rules, prompt guidance, module registration);
  `crates/smith-client/src/commands.rs` plus a new `feedback_report.rs`;
  `crates/smith-cli/src/local_command/feedback.rs`, `src/cli.rs` (the
  `smith feedback` subcommand) and `src/browser.rs` (reused);
  `crates/smith-tui/src/render/transcript/` (report rendering); `docs/`.
- Compatibility: additive. The store starts empty. Every request gains one
  small tool schema. It changes the cached prefix once, when the change ships,
  and is stable after that like the other pinned tools.
- Security: report text is written by the model and can quote prompts, code
  and paths, so it stays local and owner-only. Pattern redaction cannot clean
  prose, which is why filing always shows the exact public text first. Stored
  reports are untrusted input for any agent that reads them later.

## Approval Boundary

Approval covers the module and tool, evidence attachment, the store and its
limits, prompt guidance, `/feedback` with its notice and GitHub filing, the
`smith feedback` subcommand, and tests and documentation. It does not cover
crash capture, automatic upload, Smith-initiated reports, or headless
envelope changes.
