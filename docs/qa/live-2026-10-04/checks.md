# Live findings fixed — checks

`fix-live-findings`, branch `fix/live-findings`, 2026-10-04.

- Gate with the macOS runner's temp path and `--no-fail-fast`: fmt, strict
  Clippy, 2,070 workspace tests, `cargo deny --locked check all`.
- Re-recorded fixtures reviewed: `/agent` list, inspector, next/previous;
  undo, redo, revert, and `/diff` patches (hunks, relative paths); MCP and
  skill trust and review confirmations (inner padding). `/help` and the
  headless fixtures are unchanged.
- `../grammar-2026-10-03/final_checks.py`: 67/67 on the debug and release
  builds.

## Live re-check on the 0.3.6 release build

Same setup as [findings.md](findings.md).

- L1: after `unknown command /agentx`, Ctrl+U empties the draft and typing
  `/agent` gives `/agent`.
- L2–L4: `/agent` is headed `/agent`; the inspector states each fact once,
  renders the child's result as Markdown, and offers no `/agent resume`
  without a checkpoint; the panel row is `child-1  idle · durable · 1 turn
  · 3k tokens`.
- L5: no `activation epoch` line in the transcript after tool activation.
- L6: the edit approval names `src/main.rs` once, first.
- L7: `/undo` after adding one doc-comment line shows one hunk with three
  lines of context and `src/lib.rs`; applying it restored the file.
- L9: the child-agent approval reads `Start a child agent`, the task, `tools
  all`, `workspace shared`, a worded warning, and `don't ask again for child
  agents this session`; denying it created no file.

## Cache, 0.3.5 against 0.3.6

`../grammar-2026-10-03/cache_ab.py` ([results](cache-ab.json)). All three
providers matched within provider variance (xAI served 192 cached tokens on
0.3.5's turn 3 this time). No miss or re-billed tokens on any turn.
