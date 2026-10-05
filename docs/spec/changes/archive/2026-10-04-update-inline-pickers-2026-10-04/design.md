## Context

Choosers reach the user through three renderings and six terminal loops:

- In a session, `/connect`, `/resume`, `/model`, `/provider`, and `/profile`
  use `draw_compact_resource_picker`: an inline pane above the composer, at
  most five rows. This is already the target look.
- `smith --resume` (`choose_resume_session`), the ChatGPT account choice
  (`pick_one`, from `connection.rs`), and the ChatGPT login method
  (`choose_login_method`) each enter the terminal, build a theme, and run
  their own loop around `ResourcePicker` drawn in `standalone_picker_area`, a
  centered box.
- Setup (`smith-cli/src/setup.rs`) runs `SetupApp` in its own loop inside a
  fixed 88x30 centered box. `wait_for_login_surface` draws ChatGPT login
  progress in another box with its own loop.
- `/connect <provider>` leaves the TUI loop (`InteractiveExit::Connect`),
  suspends the terminal, runs the setup or login loop standalone, prints
  results to the normal screen (where the alternate screen hides them on
  resume), and rebuilds the host.

`SetupApp` is already a pure state machine (`on_key` → `SetupEffect`) and
`ResourcePicker` a pure filter/selection value; what is duplicated is the
loop around them and their frames.

## Goals / Non-Goals

- Goals: one inline list component; one loop for standalone screens; the
  connection flow drawn inside the session; the findings R1–R4, S1–S6, C1,
  C2, M1 fixed.
- Non-Goals: approvals and confirmation dialogs (unchanged); the in-session
  five-row limit (kept, see Decisions); leaving the alternate screen for
  standalone screens; new setup flows or providers; deleting empty session
  files.

## Decisions

- **Screen value.** `smith-tui` defines a screen as a value with
  `draw(frame, area, theme)` and `on_event(event) -> Step<Outcome, Effect>`,
  plus an optional tick interval for progress text. `ResourcePicker`,
  `SetupApp`, and login progress implement it. Rendering and key handling
  stay pure and are unit-tested without a terminal.
  - Alternatives: a trait object per loop with its own `run` (keeps five
    loops); folding standalone screens into `App` (would make setup depend
    on a session that does not exist yet).
- **One runner.** `smith-cli` gets one `run_screen` that enters the terminal
  once, owns `EventStream`, ticks, and the theme built from `--no-color` /
  `--no-motion`, and races an optional future (the OAuth wait) against
  input. It returns the screen's outcome or an effect for the caller to
  perform and feed back. Every standalone flow uses it; a flow that moves
  from setup into ChatGPT login stays in the same runner, so Esc returns to
  setup.
- **Embedded in the session.** `/connect` is idle-only, so nothing in the
  session needs processing while it runs. The TUI loop still exits with
  `InteractiveExit::Connect`, but instead of suspending the terminal the
  connection runs on an embedded `ScreenSession` that keeps the session's
  terminal and draws the retained `App` (transcript, composer, hint row)
  every frame, with the connection screen in the pane above the composer,
  growing up to the transcript height for review text. The flow's effects
  run where they run today. Its results become notices on the retained
  `App`, and the host rebuild that follows keeps the screen, as `/model`
  does; nothing is printed to the normal screen.
  - Alternative considered: running the flow inside `TuiLoop` with effects
    on a task. Rejected for this change: it duplicates the effect handling
    the standalone flows already have, for no visible difference while the
    session is idle.
- **Each step paints whole.** ratatui writes only the cells that changed, so
  when a flow moves to a new step, text that shares a character with the
  previous step at the same column reaches the terminal in pieces. The runner
  repaints the whole screen when a screen reports a new step (a step key on
  `Screen`), so every step is written out in full. It costs one full redraw
  per step change; found while the PTY tests broke on every wording change.
- **Alternate screen stays.** Standalone screens keep the alternate screen
  because the session that follows uses it; they draw from row 0, column 0
  with the same two-column gutter as the session, sized to content.
- **Numbering.** Fixed choice lists of at most nine entries (setup actions,
  credential methods, login methods, account choice) are numbered and accept
  digits; typing letters does not filter them. Inventories (`/model`,
  `/resume`, `/connect`, `/provider`, `/profile`, `smith --resume`) filter on
  typing and are not numbered, so digits stay usable in a filter.
- **Row counts.** In a session the five-row limit stays (`Accessible shared
  resource picker` keeps the transcript in view). Standalone screens show as
  many rows as fit, and every list shows `n/total` when it scrolls.
- **Empty sessions.** A session is offered for resume only if its snapshot
  holds a user message. A session is hidden only when its listing metadata
  has no user preview and a turn count of 0, so a session whose prompt failed
  before any provider usage, or whose image-only prompt completed, is still
  offered. Snapshots without listing metadata are always offered.
- **Fixtures first.** Before the loops change, terminal fixtures record the
  five standalone screens at 44x16 and 100x32 in their current form; the
  runner merge must leave them byte-identical. The presentation change then
  re-records them.

## Risks / Trade-offs

- Embedding the connection flow runs credential enrollment and OAuth waits
  beside a live session; a slow keychain call must not block rendering. The
  effects run on their own task and the flow shows a working state.
- Removing letter filtering from short numbered lists drops setup's
  `chatgpt` filter. The longest such list has seven entries.
- Hiding sessions without a user message changes the terminal table of
  `smith sessions list`. The piped, tab-separated form is a documented
  contract for the Claude Code plugin and keeps every row.

## Migration Plan

No data migration. Fixtures for the affected screens are re-recorded and
reviewed in the change. `DESIGN.md` is updated in the same change.

## Open Questions

- None blocking. Whether the trusted catalog's quick start should move from
  `glm-5.2` to `glm-5.3` is tracked separately.
