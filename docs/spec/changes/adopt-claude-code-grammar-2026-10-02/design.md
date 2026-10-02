## Context

This is the brief an implementer works from. It describes the Claude Code
interaction grammar as it applies to Smith, surface by surface, with the
current Smith output beside the target. Where Smith keeps its own decision,
that is stated and is not to be "fixed".

Read with `DESIGN.md` open. This change rewrites `DESIGN.md` first (task
1.1); after that, `DESIGN.md` is the contract and this file is its rationale.

## Goals / Non-Goals

- Goals:
  - A reader can tell at a glance who said what, what ran, and whether it
    worked, without reading metadata.
  - Detail is one key away, never in the way.
  - The surface is the same while streaming and after commit.
- Non-Goals:
  - A pixel copy of Claude Code. Smith keeps the decisions in "Kept".
  - New commands, new runtime behaviour, new provider requests.
  - Colour themes beyond the 16 ANSI names.

## The grammar in one screen

```text
> fix the flaky test in retry.rs

● I'll look at the retry policy first.

● Read(src/retry.rs)
  ⎿  Read 212 lines

● Bash(cargo test -p smith-tui retry)
  ⎿  running 3 tests
     test retry::backoff ... ok
     test retry::cancel ... FAILED
     … +14 lines (ctrl+o to expand)

● Update(src/retry.rs)
  ⎿  Updated src/retry.rs with 4 additions and 1 removal
       118    let delay = policy.delay(attempt);
       119 -  sleep(delay).await;
       119 +  tokio::select! {
       120 +      _ = sleep(delay) => {}
       121 +      _ = cancel.cancelled() => return Err(Cancelled),
       122 +  }

✻ Working… (41s · ↓ 2.3k tokens · esc to interrupt)

────────────────────────────────────────────────────────────────────────
> also cover the cancellation race▏
────────────────────────────────────────────────────────────────────────
  ? for shortcuts                                    gpt-5.3 · dev · ask
```

Rules that produce it:

1. **One marker per speaker.** `>` user. `●` everything Smith or the model
   did: prose, tool call, notice. `⎿` output that belongs to the row above.
   Continuation lines hang under the text, not under the marker.
2. **One row per tool call.** `● Name(the argument a person would say)`.
   The bullet is dim while running, green on success, red on failure; the
   word `failed` or `denied` follows a non-success so colour is never the
   only signal.
3. **Results nest and are bounded.** Up to four lines under `⎿`, then
   `… +N lines (ctrl+o to expand)`. A tool with nothing useful to preview
   gets a one-line summary (`Read 212 lines`, `Updated … with 4 additions`).
4. **Detail is behind one key.** `Ctrl+O` toggles the expanded view of tool
   output, approval detail, and anything else that was folded.
5. **Progress is one line.** Spinner, verb, then in parentheses: elapsed,
   token flow, the interrupt key. It sits directly above the composer and
   is never part of the transcript.
6. **The composer is always in the same place**, between two rules, with one
   hint row beneath it.

## Surface by surface

### Tool rows

Current, for a user-typed `!ls -la`:

```text
• shell · $ ls -la
• shell(command, cwd, timeout_ms · details unavailable) · ok
• changes · Smith turn 1 · contains ambiguous changes only; use /diff
/shell
total 8
…
```

Target:

```text
! ls -la
  ⎿  total 8
     drwxr-xr-x  6 user  staff  192 Oct  2 05:58 .
     drwx------  6 user  staff  192 Oct  2 05:58 ..
     … +4 lines (ctrl+o to expand)
```

- A model-requested call is `● Bash(ls -la)`; a user shortcut echoes as
  `! ls -la` with the result nested the same way.
- The display name is the reviewed tool label (`Bash`, `Read`, `Update`,
  `Search`, `List`, `Agent`), not the registry id. Keep the redaction rules
  in `tool-call-display`: the argument shown is the reviewed summary, and a
  protected argument stays protected. `details unavailable` is replaced by
  showing only what is reviewed; do not print the list of argument names.
- No separate "changes" notice unless the turn changed files. When it did,
  the edit rows already say so.
- Suppressed rows (`write_todos` and the reviewed delegation set) stay
  suppressed.

### Working row and turn end

Current: `⠴ Working… · 1s` inside the transcript; after the turn a dim
`Worked for 1s` that stays under whatever is printed next, including a
`/status` card.

Target: `✻ Working… (12s · ↓ 1.2k tokens · esc to interrupt)` above the
composer. Token flow is omitted until the provider reports it and carries
`~` when estimated. Retry and backoff text from
`show-provider-retry-progress` replaces the verb and keeps its wording.
With reduced motion the spinner is a static `●`.

At a successful end the row is replaced by a dim `✻ Worked for 1m 12s`
attached to that turn's last block. It is not a transcript row, is not
replayed as history, and is hidden as soon as any later block is appended.
Non-success ends keep their attributed notice.

### Streaming text and Markdown

Streamed and committed text go through one renderer. While streaming, an
unclosed construct renders as far as it is known (an open code fence is a
code block to the end of the buffer; an unclosed `**` is literal).

Supported: headings, bold, italic, inline code, fenced code with its
language as a dim label, ordered and unordered lists with nesting, block
quotes, tables (falling back to stacked `key: value` rows when the table is
wider than the pane), horizontal rules, and links. A link shows its label
underlined and, when the label is not the URL, the URL after it in dim
parentheses. Emit an OSC 8 hyperlink only when the terminal is known to
support it. `2 * 3 * 4` stays literal: emphasis needs a non-space on the
inside of each marker.

### Approvals

Current: a block that opens with a 64-hex id, a grants list, a run-on
action line, and raw JSON.

Target:

```text
╭ Bash command ────────────────────────────────────────────────────────╮
│                                                                      │
│   cargo publish --dry-run                                            │
│   in ~/work/api · up to 10 min                                       │
│                                                                      │
│   ⚠ Runs outside the sandbox with your files, environment, and       │
│     network.                                                         │
│                                                                      │
│   Do you want to proceed?                                            │
│     y  Yes                                                           │
│     a  Yes, and don't ask again for `cargo publish` this session     │
│     n  No                                                    (esc)   │
│                                                                      │
│   ctrl+o details · 1 more waiting                                    │
╰──────────────────────────────────────────────────────────────────────╯
```

- Order: what will run, where and for how long, the one warning that
  matters, the question, the choices. Everything the truth spec requires
  (exact target, material arguments, permissions, broad-authority warning,
  deadline) is present; the permission list, identity hash, and raw
  arguments are in the `Ctrl+O` detail.
- For an edit the body is the diff, with the same expand key instead of the
  fixed 18-row cut.
- The transcript scrolls with the usual keys while the prompt is open.
- Kept: `y` / `a` / `n`, Esc denies, Enter never answers, the 500 ms guard,
  the FIFO queue.

### Command menu, pickers, references

Current: `/goal [OBJECTIVE|edit …|budget N|pause|resume|clear]  inspect or
control a persistent multi-turn g`.

Target:

```text
  /help        List commands and keys
❯ /goal        Inspect or control a multi-turn goal
  /context     Show context usage or choose a window
  /status      Show session, usage, and workspace status
  /model       Switch model
               [OBJECTIVE | edit … | budget N | pause | resume | clear]
```

- Two columns. The name column is as wide as the longest visible name. The
  description is dim, starts with a capital, and is cut at a word with `…`.
- Argument grammar appears only for the selected row, on the line beneath.
- Order is the same in the menu, the palette, and `/help`.
- Kept: the menu sits above the composer and shows five rows.

Model picker, current: `example-model · current  local/example-model · ctx
128k [project config] · input 124k [project con`.

Target:

```text
  Choose model · type to filter                                   1/10
❯ example-model       local · 128k context                  ✓ current
  sonnet              Claude Code CLI · 200k context
  opus                Claude Code CLI · 200k context
               limits from project config · input 124k · output 4k
```

- Name, then one short dim description, then state at the right edge.
- Provenance and full limits for the selected row only, on a detail line.
- The `@` picker lists files first, then agents with a dim `agent` tag and
  no model metadata.

### Informational results

`/help`, `/status`, `/context`, `/diagnostics` stay inline (kept). They
change in form only:

- Aligned label and value columns; values word-wrap with a hanging indent.
  No mid-word breaks. A long path is shortened from the left with `…`.
- `/help` leads with a short "start here", then commands in the menu's
  order and format, then keys as a two-column table in plain words
  ("Enter while working: send now" rather than "steers").
- `/diagnostics` is grouped under headings (Context, Cache, Recovery,
  Session) with one fact per line. An unknown value reads `unknown`, not
  `?/?/?`.
- A result longer than the pane opens at its top.

### Composer

- A dim rule above and below; `>` prompt; placeholder text when empty.
- `!` as the first character switches the prompt to `!` and the hint row to
  `bash mode`. `@` and `/` open their pickers as today.
- Up and Down move between lines of a multi-line draft; they reach history
  only from the first or last line. `Shift+Enter` and `\` then Enter insert
  a newline.
- Line editing: `Ctrl+A` / `Ctrl+E` line start and end, `Ctrl+W` delete
  word, `Ctrl+U` delete to start, `Ctrl+K` delete to end, `Alt+B` / `Alt+F`
  word left and right. Home and End act on the draft when it is not empty,
  and on the transcript when it is.
- Hint row, left: `? for shortcuts` when idle and empty; the busy keys
  while a turn runs. Right: model, profile, approval mode. Hints are the
  last thing dropped when the terminal is narrow, not the first.
- `?` on an empty draft opens a shortcuts panel in the anchored pane. Any
  key closes it. It is not a transcript entry.
- Kept: Enter steers and Tab queues while busy; Tab cycles profile when
  idle and empty; `Ctrl+C` twice exits; paste and image placeholders.

### First-run setup

One frame, titled once. Each entry is a name line and a wrapped dim
description line. The footer keys are always visible.

## Kept: Smith decisions that do not follow Claude Code

| Decision | Reason |
| --- | --- |
| Sixteen ANSI colours by name, no background fills | Smith cannot know the palette |
| No header; transcript owns the screen | `DESIGN.md` principle 1 |
| Informational commands print inline and need no dismissal | truth spec |
| Enter steers, Tab queues while busy | truth spec |
| `y` / `a` / `n` approvals with a quiet-window guard | owner decision 2026-10-02 |
| Command menu above the composer, five rows | truth spec |
| `●` rather than `⏺`; ASCII todo marks | emoji-capable code points change width between terminals |
| Pointer selection implemented by Smith | truth spec |
| Estimated numbers carry `~`; unknown is stated | `DESIGN.md` principle 3 |

## Decisions

- Decision: glyph set is `>`, `●`, `⎿`, `✻`, `❯`, `✓`, `…`. Each must
  report width 1 from `unicode-width` and must not be an emoji-capable code
  point. `⏺`, `✳`, and `✔` are excluded for that reason.
  - Alternatives considered: keep `›`, `•`, `└` (rejected: the owner chose
    the Claude Code grammar, and the nested-result glyph is its most
    recognisable part).
- Decision: `Ctrl+O` is the single expand key. `/details` remains as the
  command form of the same toggle.
- Decision: the working row leaves the transcript and sits above the
  composer, so appending to the transcript never moves it.
- Decision: Markdown stays hand-written unless the implementer shows that
  streaming-safe list, table, and quote support costs more than adopting
  `pulldown-cmark`. A new dependency needs the owner's approval.

## Risks / Trade-offs

- Almost every render test asserts on current strings. Expect a large,
  mechanical test update; do it per surface, not at the end.
- Live-versus-replay parity must hold for every changed row. The parity
  test is the gate for sections 2 and 3.
- Narrow terminals: each target above must be checked at 44 columns.
- `DESIGN.md` has uncommitted edits from an in-flight change. Rewrite the
  reference paragraphs and affected sections; leave the retry and idle
  compaction text as it is.

## Migration Plan

1. `DESIGN.md`, then specs, then code, one surface per commit.
2. Order: tool rows, working row and turn end, Markdown, approvals; then,
   after `refactor-client-structure`, menus, pickers, and informational
   results; then the composer.
3. Each surface lands with its PTY captures at 100×32, 80×24, and 44×16 in
   `docs/qa/`.

## Open Questions

- Should the working row show a varying verb, or always `Working…`?
  Proposed: always `Working…`, with retry text when retrying.
- Should `Ctrl+O` expand only the newest folded block, or toggle all?
  Proposed: toggle all, as `/details` does today.
