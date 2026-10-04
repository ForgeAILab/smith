# Live pass on 0.3.5

2026-10-04. Installed `smith 0.3.5` against the real `~/.smith` configuration
(default profile `code` = `zai/glm-5.3`) in a private tmux server at 100x32,
`--approval ask`, in a scratch Git project. Captures are in
[captures/](captures/); the scratch path is replaced by `<project>`.

## What held

- **Older sessions resume.** A session written headless by 0.3.3 with one
  read-only child agent, and one written by 0.3.4 that edited `src/lib.rs`
  and answered with a table, both resume in 0.3.5 with their transcript,
  tool rows, table, and child summary intact (`01-resume-a`, `08-undo`).
- **Cross-provider continuation.** After `/model` to
  `google/gemini-3.8-flash`, a turn over GLM's history (including its tool
  calls) answered correctly (`12-exit`).
- **Reconfigure keeps the screen.** `/model` replaced the identity and added
  two notices; the transcript stayed (`11-model-switch`).
- **Feedback.** `/model` during a turn shows `/model requires an idle turn;
  draft preserved` in the hint row with the identity kept (`05-refused`).
- **Shortcuts, approval keys, undo cancel, exit summary** behave as designed
  (`06-shortcuts`, `09-approval`, `10-undo-confirm`, `12-exit`).

Not exercised: the streaming-table placeholder (GLM sent the table in one
delta; fixture-covered) and setup's collision preview (fixture-covered).

## Findings

| # | Finding | Evidence |
| --- | --- | --- |
| L1 | **Line editing does nothing in a slash draft.** With a draft starting `/`, Ctrl+U, Ctrl+W, Ctrl+A/Ctrl+K (and by code reading Ctrl+E, Alt+B/F, Left/Right, Home/End) are ignored; only Backspace works. After `unknown command /agents` the draft stays, Ctrl+U does not clear it, and typing `/agent` produced `/agents/agent`. Cause: `on_palette_key` ends with `_ => None`. The key-table check passed because it only tests plain drafts. | session notes; `crates/smith-tui/src/app/input.rs` `on_palette_key` |
| L2 | **The child inspector repeats itself and shows raw Markdown.** The `result:` field prints the child's answer with `**` and backticks; the session id, `durable`, and the token count appear in the header and again in a footer line that also says `no activity recorded in this session` beside a shown result; the header has an unlabelled `· 1 ·`; `exact recovery: /agent resume child-1` is offered directly under `no exact checkpoint`. | `03b-inspector-top` |
| L3 | **`/agent` output is headed `/agents`**, a command that does not exist (which is how `/agents` was typed in L1); rows say `1 turns` and `3055 tokens` where the rest of Smith writes `3.1k`. | `02-agent-list` |
| L4 | **The agents panel row shows the child's full session id**, which pushes the useful part off the row (`… · 1 turns …`). | `01-resume-a` |
| L5 | **An internal line appears in the transcript on every tool activation:** `capabilities · activation epoch 2: tool:agent, tool:list, tool:registry.search, skill:smith.security, …`. The `registry.search` call that causes it is already hidden as "a capability bootstrap the user did not ask for" (DESIGN.md). | `04-stream-6` |
| L6 | **Approval and undo boxes print absolute paths**, twice in the approval and as `--- current <abs>` / `+++ restore <abs>` in undo, wrapped mid-path; the tool row beside them says `src/lib.rs`. | `09-approval`, `10-undo-confirm` |
| L7 | **Undo, redo, and revert patches are whole-file**: every line of the file after the turn as `-`, then every line before it as `+`, with no hunks or context. A 4-line addition reads as 22 lines; on a real file the patch is unreviewable. | `10-undo-confirm`; `crates/smith-tools/src/change.rs` `textual_reverse` / `textual_forward` |
| L8 | **Small wording and layout slips:** `… 1 unchanged lines`; the `a` choice reads `Yes, don't ask for \`edit\` within this target, without extra permissions, this session`; the undo box has no inner padding while the approval box does; undo after resume says `historical change records are visible but cannot be automatically undone after resume`; the spawn row's result preview is raw JSON (`{"note":"the result will be delivered when the child completes","spawned":"child-1"}`); `/model` rows mix `1048576 context` with `272k context`. | `01-resume-a`, `08-undo`, `09-approval`, `10-undo-confirm`, `11-model-switch` |

Found while re-checking the fixes on the 0.3.6 release build:

| # | Finding | Evidence |
| --- | --- | --- |
| L9 | **The child-agent approval is a raw field dump.** Its title repeats as the first line; it lists `deadline_ms: null`, `max_tokens: null`, `max_turns: 4294967295` (the unlimited sentinel), `tools.scope: all`, `workspace.policy: shared_project`, the target `child-agent:session-<ID>`, `Warning: host-defined authority`, and offers `don't ask again for \`delegation.spawn\``. | `13-spawn-approval` |

Open question, not a finding yet: the exit summary prices a two-model
session as `$0.012 exact · google/gemini-3.8-flash`, naming only the last
model.

By design and not a finding: modals are centered over the transcript
(DESIGN.md), so transcript fragments show on either side of a box.
