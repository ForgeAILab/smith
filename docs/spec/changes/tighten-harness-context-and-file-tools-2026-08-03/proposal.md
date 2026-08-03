---
created_at: 2026-08-03T20:21:57Z
updated_at: 2026-08-03T20:21:57Z
---

## Why

Three independent reviews of comparable harnesses — LangChain's Deep Agents
v0.7 release notes, OpenAI's `codex-rs`, and Anthropic's `claude-code` — agree
on findings that Smith currently gets wrong in four places.

**Instructions are shipped for capabilities that are not registered.** Smith's
`DELEGATION` (~187 tokens) and `QUESTIONNAIRE` (~105 tokens) sections live in
`stable_fragments()` and are therefore sent on every request. But
`crates/smith-runtime/src/factory.rs` only registers `QuestionnaireTool` when
the surface is not `HostSurface::Child`, and the delegation tool depends on the
active profile permitting it. A child agent is currently told, in the immutable
instruction prefix, to use an `ask_user` capability it does not have. That is
not primarily a token problem — it is an instruction describing a tool that
does not exist, which is a known source of invalid tool calls. Deep Agents
v0.7 reached the same conclusion from the other direction ("avoid repetition;
do not duplicate instructions across the system prompt and tool descriptions")
and cut its base harness from ~6k to ~2k tokens with no eval regression.

**`write_todos` is unconditional and hard-coded into the workflow prose.**
`factory.rs` always pushes `WriteTodosTool`, and the `WORKFLOW` section always
says "Use write_todos". Deep Agents made `TodoListMiddleware` opt-in after
evals showed negligible gain. Smith should keep the default on — the TUI
renders the plan, which is one of the three cases Deep Agents itself says to
keep it for — but the prose must follow registration rather than assume it,
and read-only postures whose entire output *is* a plan should not carry both.

**There is no whole-file write and no delete.** `edit` creates a file only via
an empty `old_string` with an atomic `create_new` open, so rewriting an
existing file costs reading it and echoing the entire body back as
`old_string`: 2× the file in tokens for a legitimate operation. Deletion has no
narrow tool at all and must route through `shell rm`, which declares write,
spawn, and network. That directly contradicts Smith's own `TOOL_USE` section
("use the smallest activated capability set… do not route around it through a
broader tool"), and `Permission::FsDelete` already exists in the shared
registry vocabulary with no Smith tool using it.

**Semantic summarization triggers on turn count, which is both a bad proxy and
cache-hostile.** `SemanticSummaryPolicy::trigger_turns` is 6. One `shell` with
a large output can exhaust the window at turn two, while six trivial turns fire
a paid model call that reclaims nothing. Worse, every compaction rewrites
history and therefore invalidates the provider prompt cache, so a cheap early
pass costs a full cache miss to reclaim a little slack. Codex measures growth
against an explicit compaction-window baseline
(`AutoCompactTokenLimitScope::BodyAfterPrefix`), warns the model once before
the boundary, and treats compaction as a rare, deliberate cache-resetting
event.

## What Changes

- **Conditional instruction sections.** `DELEGATION`, `QUESTIONNAIRE`, and the
  `write_todos` sentence of `WORKFLOW` move out of the unconditional stable
  prefix and are contributed only when the corresponding capability is
  actually registered for the run. Section identity, per-section revisions,
  and authored order are preserved; the sections that remain unconditional
  keep their existing revisions so cached prefixes are not disturbed for runs
  that register everything.
- **`edit` gains an explicit `operation`.** `replace` (today's exactly-once
  behavior), `create`, `overwrite`, and `delete`. The tool count stays at five,
  so the base tool surface does not grow. Per-operation permissions extend the
  pattern `edit::prepare` already uses, adding `Permission::FsDelete`.
- **Read-before-overwrite precondition.** `overwrite` and `delete` require that
  the session has already read the target in full, and refuse when the file's
  modification time is newer than that read. This is the safety property
  `claude-code` enforces via `readFileState`, and it is what makes a
  destructive whole-file operation as safe as today's exact-match edit. A new
  `ReadRecorder` records reads at the existing `ObservedTool` boundary.
- **Cache-aware tiered context management.** Turn count stops triggering
  semantic summarization and becomes an eligibility floor. A one-shot
  in-context budget notice — appended, so it never invalidates the cached
  prefix — fires when remaining input budget crosses a threshold. Semantic
  summarization fires on a fraction of the input budget, measured after the
  stable prefix, and reduces deeply so it does not retrigger.
- **Session usage report and analytics record.** Exiting the TUI prints the
  session's token totals with their provenance, and each session appends a
  bounded usage record so budget behavior can be analyzed across sessions.
- **Base harness token budget test.** A regression test asserts the assembled
  stable prefix plus the default tool specs stay under an authored ceiling, so
  prompt growth becomes a reviewed decision.

## Impact

- Affected specs: `harness-policy`, `tool-execution`, `runtime-integration`
- Affected code: `crates/smith-runtime/src/prompt.rs`,
  `crates/smith-runtime/src/factory.rs`,
  `crates/smith-runtime/src/summary.rs`, `crates/smith-tools/src/edit.rs`,
  `crates/smith-tools/src/read.rs`, `crates/smith-tools/src/change.rs`, new
  `crates/smith-tools/src/read_state.rs`, `crates/smith-tui/src/status.rs`,
  `crates/smith-cli/src/main.rs`
- Behavior changes users can observe: a child agent no longer receives
  questionnaire instructions; `edit` accepts a new argument; deleting a file no
  longer requires approving a shell command; sessions compact later and less
  often; the TUI prints a usage summary on exit.
- Not in scope: replacing `search` with a regex engine, progressive disclosure
  changes to `registry.search`, and any change to the approval UI itself.
