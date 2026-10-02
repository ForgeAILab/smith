# Smith structure and UX audit — 2026-10-02

Scope: how the CLI/TUI is put together (`smith-cli`, `smith-tui`, and their
seams with `smith-config`, `smith-runtime`, `smith-host`), and what a user
experiences in the first ten minutes. Follows
[the 2026-09-21 startup audit](../smith-ux-2026-09-21/report.md); the eight
issues fixed there are not repeated.

**Verdict.** The crate graph, the runtime composition path, the persistence
layering, and the headless contract are sound. The weakness is the layer
between the runtime and the screen: facts have several owners, data crosses
crate boundaries as prose that the other side re-parses, and each extensible
thing (commands, dialogs, keys, providers) has several hand-synced lists
instead of one registry. Most of the UX defects below are direct consequences
of those three habits, not independent bugs.

The plan is in two Stage 1 proposals and a roadmap:

- [fix-interaction-defects](../../spec/changes/fix-interaction-defects-2026-10-02/proposal.md)
  — defects that can be fixed now, without restructuring.
- [refactor-client-structure](../../spec/changes/refactor-client-structure-2026-10-02/proposal.md)
  — the structural core: one owner per fact, typed seams, one registry.
- [Roadmap](#roadmap) — the four later changes that depend on those two.

## Method and limits

- Tested the installed `smith 0.2.15` binary under a real PTY (tmux) with an
  isolated `HOME`, a disposable Git project, and the deterministic `fake`
  provider. No real credentials, sessions, or providers were touched.
  Captures are in [captures/](captures/).
- Three read-only audits covered `smith-cli`, `smith-tui`, and crate
  boundaries against `docs/architecture.md`, `docs/spec/project.md`, and
  `docs/spec/specs/code-organization/spec.md`. Nothing was compiled, and no
  source file was changed.
- Evidence levels used below: **live** (observed in the running binary),
  **source** (read directly at the cited lines), **derived** (follows from
  code paths but not exercised). Line numbers are for commit `58bc098` plus
  the working tree.
- Structural findings are at **source** level from the three audits. These
  were re-checked directly: the dead setup arms, the unread exit policy, the
  catalog-revision drift, the approval key handler, the reused confirm copy,
  the glob imports, the unused protocol command half, the 16 tags, the 40
  exported runtime modules, and the two factory shims. The remaining counts
  and line ranges were not independently re-read.
- Four files are being edited by in-flight changes
  (`smith-tui/src/app/{reducer,state,conversation}.rs`,
  `render/transcript.rs`). Their line numbers will move, and the plan does
  not restructure them until those changes land.
- Not assessed: real providers, OAuth, child agents under load, native
  clipboard, screen readers, Linux.

## UX findings

Ranked by how much they cost a user. All paths are under `crates/`.

| # | Finding | Level | Evidence |
| --- | --- | --- | --- |
| U1 | **Two first-run setup entries are dead.** Choosing "Anthropic Messages API" or "Connect ChatGPT (experimental)" and pressing Enter does nothing: no error, no next step. | live | [capture](captures/21-setup-anthropic.txt); the handler has no arm for either id and falls to `_ => {}` (`smith-tui/src/setup.rs:864-908`) |
| U2 | **An approval can be answered by text the user was already typing.** The prompt takes focus the moment it arrives and accepts bare `y`, `a`, `n`; `a` grants the target for the whole session. Enter steers during a turn, so the composer is in use exactly when approvals arrive. Other characters are swallowed. | source | `smith-tui/src/app/prompts.rs:43-44`, `app/input.rs:589-599` |
| U3 | **Changing model, effort, profile, or context window rebuilds the whole application.** The alternate screen is left and re-entered; local results, notices, scroll position, and composer history are lost. Tab on an empty prompt does the same. | derived | `smith-cli/src/tui_driver.rs:521-523, 91, 144`; `smith-cli/src/runtime_host.rs:521-526`; `smith-tui/src/transcript.rs:557-561` |
| U4 | **Cancelling a picker leaves the command in the composer.** `/model`, Esc, then `/diagnostics` produces `unknown command /model/diagnostics`. | live | reproduced twice; see [walkthrough](#live-walkthrough) |
| U5 | **A streaming answer is raw text and reflows on commit.** Markdown markers are visible for the whole stream, then the block is restyled. Links keep the label and drop the URL. No lists, tables, or quotes. | source | `smith-tui/src/render/transcript.rs:466-487` vs `552-688` |
| U6 | **One `!ls -la` produces four transcript rows before its output, and a wall of authority text.** The approval shows a 64-hex id, a grants list, raw JSON arguments, and a run-on line (`action  ls -laHost access: …`). The result repeats the call twice (`shell · $ ls -la`, then `shell(command, cwd, timeout_ms · details unavailable) · ok`) and adds `changes · Smith turn 1 · contains ambiguous changes only; use /diff` for a read-only command. | live | [approval](captures/07-bang.txt), [result](captures/08-bang2.txt); the run-on is a `\n` in `smith-tools/src/shell.rs:189` that the approval renderer flattens |
| U7 | **Informational output is not written for the reader.** `/help` is about 60 unaligned lines with a mid-word wrap (`go` / `al`) and phrases such as "uncommitted steers are resent only after cancellation discards them". `/diagnostics` is a single internal dump (`idle route ?/?/?`). Local results are character-wrapped, not word-wrapped. At 50×16 `/status` shows its tail first. | live | [help](captures/03-help.txt), [diagnostics](captures/13-diag.txt), [narrow](captures/18-narrow.txt) |
| U8 | **Three confirmation dialogs show another dialog's text.** Redo, MCP trust, and skill trust reuse the undo dialog and print "Review the complete reverse patch" and "y apply undo". The two trust prompts are security decisions. | source | `smith-tui/src/render/modal.rs:518-533`; `render/layout.rs:153-166` |
| U9 | **Picker rows lead with metadata.** A model row reads `example-model · current  local/example-model · ctx 128k [project config] · input 124k [project con…`; the name is repeated and provenance fills the line. The `@` picker does the same for agents. | live | [model](captures/10-model.txt), [reference](captures/15-at.txt) |
| U10 | **The composer lacks ordinary editing.** No cursor movement between lines of a multi-line draft (Up/Down are history and child selection), no `Ctrl+A/E/W/U/K`. Tab has five meanings, Esc four. The idle footer shows no key hints. Home/End arms in the composer are unreachable. | source | `smith-tui/src/app/input.rs:292-367, 988-995`; `composer.rs:226` (only called from tests) |
| U11 | **A configured `background.exit_policy` is ignored.** It is documented and resolved, but headless reads only the flag. | source | `docs/configuration.md:936`; `smith-config/src/resolve/provider.rs:1241`; `smith-cli/src/main.rs:244` |
| U12 | **Command-line edges are uneven.** `smith setup --help` is "unknown setup option"; `smith help` is "unexpected argument"; `smith -p --help` sends "--help" as the prompt; `echo hi \| smith` reports `entering the alternate screen: Device not configured (os error 6)`; an unknown option prints the `--help` hint twice; `sessions list` prints `1790935135329ms` with no header; `--help` lists 5 of 26 slash commands and no environment variables. | live | [walkthrough](#live-walkthrough); `smith-cli/src/cli.rs:293-296, 352-355, 370-373, 607-655` |
| U13 | **Headless text mode writes internal state to stderr on every successful run**: `parent: idle`, `provider attempts: 1 committed · 0 discarded`, `activation epoch 0 · tool:registry.search`. Diagnostics on stderr are the documented contract; the content is the problem. | live | [walkthrough](#live-walkthrough) |
| U14 | **During an approval the transcript cannot be scrolled**, so the work being approved cannot be reviewed. Approval diffs stop at 18 rows with no way to expand. | source | `smith-tui/src/app/input.rs:40-53`; `render/approval.rs:30, 162-176` |
| U15 | **First-run setup draws two nested frames both titled "Smith setup"**, and every description is cut at the right edge. Review text says "trusted catalog v2" (the catalog is revision 5) and the menu says "GLM-4.7" (the flow selects `glm-5.2`). | live / source | [capture](captures/20-setup.txt); `smith-tui/src/setup.rs:371, 547`; `smith-config/src/setup.rs:16` |

One direction conflict needed a decision (now made, see the end of this
report): `DESIGN.md` states that the text hierarchy follows **OpenAI Codex CLI 0.145.0**
(`DESIGN.md:11-15`), while the working direction recorded for this project is
to **mirror Claude Code**. U5, U6, U9, and U10 are judged differently under
each.

## Structural findings

| # | Finding | Evidence |
| --- | --- | --- |
| S1 | **Session accounting lives in the terminal crate, and command text lives in the CLI crate.** `smith-tui` owns the cache projection, usage, pricing, cost, and the usage log (with file I/O); headless and the exit report import them, so `smith -p` cannot be built without ratatui. In the other direction, about 1,000 lines of `/status`, `/context`, and `/diagnostics` wording live in `smith-cli`. | `smith-tui/src/cache.rs` (2,174 lines), `status.rs:301-714`, `usage_log.rs:146-161`; `smith-cli/src/headless.rs:32-34`, `runtime_host.rs:349-424`, `local_command.rs:581-1564` |
| S2 | **Data crosses crates as prose and is parsed back.** The host formats `/status`, `/context`, `/help` as strings; the renderer recovers structure from titles, headings, glyphs, and `label: value` splits. Child state is a `String` written as `"running"` by the reducer and `"working"` by the host for the same condition. Whether an agent can be resumed is decided by `detail.contains("resumable")`. Notices are `{ source: String, text: String }` with 30+ free-form sources. | `smith-tui/src/render/transcript.rs:361-365, 690-727, 826-954`; `app/reducer.rs:707`; `smith-cli/src/submission.rs:315-323`; `smith-tui/src/app/resources.rs:788-791`; `transcript.rs:134-139` |
| S3 | **The "one command registry" is four lists.** A command is restated in the `COMMANDS` table, the `CommandAction` enum, the `parse` match, and a hand-written reverse name map; host-backed commands add an arm in a 538-line function. Adding one costs 4 to 8 edits across two crates, guarded by eleven cross-crate `unreachable!` arms. The truth spec already requires "one typed command registry". | `smith-tui/src/commands.rs:9-64, 121-304, 352-442`; `app/resources.rs:455-819`; `smith-cli/src/local_command.rs:42-579`; `docs/spec/specs/client-surfaces/spec.md:278` |
| S4 | **Provider and model knowledge is copied into three crates and has already drifted.** `smith-tui` cannot see `smith-config`, so endpoints, limits, and provider names are retyped as literals; the connectable-provider list is hard-coded a third time in the CLI. One model release edits four source files. U1 and U15 are the visible result. | `smith-tui/src/setup.rs:414-427, 544-549, 866-896`; `smith-config/src/setup.rs:25-77, 159-310`, `catalog.rs:93-100`, `cli_agents.rs:47-56`; `smith-cli/src/resources.rs:135-172`, `connection.rs:177-209`; commit `ed253a8` |
| S5 | **Dialogs, text inputs, and key handling each exist several times.** Three overlay families; nine copy-pasted yes/no confirms dispatched in three places; four text inputs (three are push/pop only); four "selected row" implementations; no keymap (a 268-line and a 242-line key function plus four more). In the CLI, four separate "ask the user" implementations, six terminal event loops, the theme-from-flags block written five times. | `smith-tui/src/app/input.rs:101-367, 138-287, 773-1014`; `render/layout.rs:150-193`; `render/composer.rs:275-343`; `smith-cli/src/setup.rs:127-139, 355-462`, `xai.rs:26-53`, `chatgpt.rs:227-382`, `resources.rs:774-877` |
| S6 | **Application state is 49 hand-synchronised fields.** "A turn is live" is spread over seven fields with resets placed by hand at four sites; the in-flight retry change had to add its reset at all four. The single overlay slot is overwritten by 16 sites, and only approvals and questionnaires queue. | `smith-tui/src/app/state.rs:821-964`; `app/reducer.rs:325-356, 587-604, 947-960`; `app/prompts.rs:23, 43-47`; `app/resources.rs:843-896` |
| S7 | **There is no single write path, and the renderer mutates state.** Five mutating entry points plus about 25 host setters; the host assigns `app.status.*` directly and pushes transcript rows from about 70 sites. `draw_synced` takes `&mut App` and writes scroll state, while tests mostly call the other path, `draw` (115 calls against 14). | `smith-cli/src/tui_driver.rs:99, 143, 910, 963`; `smith-tui/src/render/layout.rs:49-68` |
| S8 | **`smith-cli` is one namespace in ten files.** Nine files open with `use super::*;`, `main.rs` carries a 67-line import block for them, and tests are spliced in with `include!`. Four functions hold most of the behaviour: `run_tui` (682 lines), `handle_local_command` (538), `runtime_resources` (513), `run_with_io` (477). `cli::Selection`, documented as parsed arguments, is mutated to drive session rebuilds. | `smith-cli/src/main.rs:27-100, 253`; `tui_driver.rs:369-1050`; `cli.rs:83-104`; `runtime_host.rs:131-147, 598-657` |
| S9 | **The transcript is rebuilt and wrapped three times per frame with no cache**, unbounded for the root conversation, and redrawn every 100 ms while busy. Scroll arithmetic is `u16`. Two wrap algorithms coexist. | `smith-tui/src/render/layout.rs:57-58`; `render/transcript.rs:29-69`; `render/wrap.rs:46-51`; `render/helpers.rs:72-91` |
| S10 | **Client protocol v1 is half-built.** Events are projected by a JSON round-trip into a hand-mirrored 51-variant enum (a mismatch becomes `Unknown` at runtime). The command half (`SmithInput`, receipts, `submit/steer/cancel`) has no caller. The CLI still drives sessions through the handle the docs call deprecated "for one migration release", 16 tags ago. | `smith-runtime/src/client.rs:57-195`; `smith-cli/src/submission.rs:6-235`; `docs/architecture.md:37-40` |
| S11 | **Stated boundaries are checked by name, not by structure.** `smith-runtime` exports 40 of 42 modules (10 have no consumer), which disables `unreachable_pub`. Two factory "stages" are 7- and 9-line shims beside a 392-line `build()`. The architecture test checks that files exist and that two literal strings are absent. Module docs claim "no second list to drift", "nothing here mutates state", and "no I/O, no clock"; none holds. | `smith-runtime/src/lib.rs:14-55`; `factory/compose.rs`, `factory/delegation.rs`, `factory.rs:1146-1537`; `tests/architecture.rs:90-128`; `smith-tui/src/commands.rs:1-5`, `render/layout.rs:3-5`, `lib.rs:15-19` |
| S12 | **The spec process is behind the code.** 23 change folders are unarchived; 11 are fully checked. All 19 truth specs still say `Purpose: TBD`. One change is marked complete for a migration that did not happen (S10). | `docs/spec/changes/`; `add-modular-harness-boundaries-2026-08-20/tasks.md:53-54` |

## What is sound and should stay

- The crate dependency graph: acyclic, `smith-config` has no sibling
  dependencies, the runtime facade enters only through `smith-runtime`.
- `start_host` as the single session-start path for TUI and headless.
- Headless discipline: stdout-only machine output, versioned envelopes,
  fail-closed approvals, documented exit codes.
- Provider adapters as decorators over the runtime's `Provider` trait, with
  kind-branching confined to `factory/provider/*`.
- Persistence layering and the purity of `cache_lifecycle.rs`.
- The setup reducer as a pure state machine with a transactional,
  rollback-capable publish.
- The shared `Conversation` fold for root and children, journal replay, and
  the live-versus-replay parity test.
- `selection.rs`, `render/wrap.rs`, `terminal.rs`, the dirty-flag redraw.
- The hand-written argument parser's core. Its edges need fixing; replacing
  it is not required.
- "Unknown is not zero" in usage and cache accounting. The rules are right;
  only their crate is wrong.

Out of scope by prior decision: the `agent-runtime` pin and the
simple-summary baseline.

## Live walkthrough

Environment: macOS, `smith 0.2.15` from `~/.local/bin`, isolated `HOME`,
100×30 unless stated.

| Step | Input | Result |
| --- | --- | --- |
| Start | `smith` | Getting-started guide, composer, footer. Clear. |
| Completion | `/` | Five rows, unaligned, long entry cut mid-word; order differs from `/help` ([capture](captures/02-slash.txt)) |
| Help | `/help` | Starts at the top (fixed on 09-21); follow is paused, so the next result needs End ([capture](captures/03-help.txt)) |
| Prompt | a sentence | Answer and "Worked for 6ms". Clear. |
| Shell | `!ls -la` | Approval required for a command the user typed (the current spec requires this); U6 |
| Status | `/status` | Readable card. The previous turn's "Worked for 1s" stays beneath it ([capture](captures/09-status.txt)) |
| Picker | `/model`, Esc | Picker closes, `/model` stays in the composer; U4 |
| Diagnostics | `/diagnostics` | U7 |
| Narrow | 50×16, `/status` | Tail of the card is shown first ([capture](captures/18-narrow.txt)) |
| First run | empty `HOME` | U1, U15 |

Command line:

```text
$ smith setup --help
smith: unknown setup option `--help`; run `smith --help`
Try `smith --help` for usage.

$ smith sesions list
smith: unexpected argument `sesions`; pass a prompt as `smith -p <prompt>`
Try `smith --help` for usage.

$ smith sessions list
session-ba10f547-…	1790935135329ms	2	local/example-model	explain what src/main.rs does

$ echo hi | smith
smith: entering the alternate screen: Device not configured (os error 6)

$ smith -p hi
This session is running Smith's deterministic fake provider; …
smith: parent: idle
smith: provider attempts: 1 committed · 0 discarded
smith: activation epoch 0 · tool:registry.search
smith: resume checkpoint: saved 2026-10-02 05:58:59 -04:00
```

## Roadmap

Three rules drive every step:

1. **A fact has one owner.** Provider descriptors, model limits, prices,
   command names, and state labels are defined once and passed as data.
2. **Data crosses a crate boundary typed.** Wording is chosen where it is
   drawn, never parsed back.
3. **One registry per extensible thing.** Commands, key bindings, dialogs,
   and setup entries are each one table that drives parsing, dispatch, help,
   and hints.

| Order | Change | Contents | Fixes | Depends on |
| --- | --- | --- | --- | --- |
| 1 | `fix-interaction-defects` (proposed) | Dead setup entries, approval keystroke guard, picker cancel, confirm copy, approval text structure, subcommand help, non-terminal refusal, honoured exit policy, terminal session listing | U1, U2, U4, U8, U11, U12, part of U6 | nothing |
| 2 | `refactor-client-structure` (proposed) | Client-neutral crate for session accounting and typed local results; single command registry; single provider descriptor source; explicit imports in `smith-cli` | S1, S2 (commands), S3, S4, S8; enables U7, U9, U15 | nothing; avoids the four in-flight files |
| 3 | `refactor-tui-state` | Typed notices with explicit ephemeral or transcript persistence; `ChildState` enum; one turn-state value reset in one place; one confirm component and an overlay queue; a keymap table that also generates help and footer hints; composer line editing | S2 (state), S5, S6, S7; U10, U14, rest of U6 | change 2; the two in-flight changes landed |
| 4 | `preserve-app-across-reconfigure` | Model, effort, profile, and context changes rebuild the host session only; the application, its transcript, and composer history survive; session controls stop living in `cli::Selection` | U3; rest of S8 | change 3 |
| 5 | `render-transcript-incrementally` | Per-block wrapped-line cache keyed by block revision and width; one layout pass shared by drawing and scrolling; read-only draw; streaming and committed text through one markdown renderer; wide scroll offsets | S9; U5 | change 3 |
| 6 | `settle-client-protocol` | Decide: finish `SmithSession` and migrate the CLI, or remove the unused command half and correct the docs. Add a test that every current runtime event projects to a known variant. Make unconsumed `smith-runtime` modules crate-private. Replace name-based architecture tests with structural ones. | S10, S11 | independent; can run beside 3 to 5 |

Housekeeping that needs no proposal: archive the 11 fully-checked changes,
fill the `Purpose` line of each truth spec, and correct the three module docs
named in S11.

Decisions (owner, 2026-10-02):

- Reference grammar is Claude Code. `DESIGN.md:11-15` still names Codex CLI
  and is rewritten by `adopt-claude-code-grammar`, which replaces the
  wording-and-layout parts of changes 3 and 5 above.
- Approvals keep `y` / `a` / `n` with a 500 ms quiet-window guard.
- ChatGPT stays in first-run setup.
- A user-typed `!command` runs without approval.
- The client-neutral crate is `smith-client`.
- Implementation is delegated to Codex, one change at a time.
