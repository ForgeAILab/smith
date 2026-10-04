## Context

`run_interactive_command` (`smith-cli/src/runtime_host.rs`) loops: start a
host, call `run_interactive`, act on the `InteractiveExit`, repeat.
`run_interactive` (`tui_driver.rs`) builds a new `App`, seeds it from the
host, enters the terminal, and runs `run_tui`, which owns per-host locals:
the runtime event subscription, the local-outcome channel, local shell
approvals, shell shortcut anchors, child subscriptions, the MCP change
receiver, the interaction surface, and the window-title tracker. Every exit
other than `Quit` loops back through all of it.

Every selection command requires an idle turn, and Tab cycles profiles only
on an idle empty draft, so a rebuild happens at idle today.

## Goals / Non-Goals

- Goals: one terminal per run; one `App` per session identity; a rebuild
  that keeps the session keeps what the user sees and typed.
- Non-Goals: keeping a running turn across a rebuild; changing what a
  rebuild composes; keeping the app when the host fails to start (today's
  error exit stays).

## Decisions

- Decision: split `run_interactive` into "seed a new app" and "rebind an
  existing app to a host". Both re-derive the same host-backed state:
  status (model, provider, price, advisor price, agent, approval mode,
  harness, goal, reasoning hint, account), resources, children from the
  coordinator, usage records, and cache events. Status is rebuilt from fresh
  and re-seeded, never merged.
- Decision: only the new-app path calls `restore_transcript`. A rebind keeps
  the transcript blocks, folded or expanded state, scroll position,
  `work_details`, the composer and its history, and the inspector logs of
  children the new coordinator still lists. It resets the live turn through
  `refactor-tui-state`'s single reset and clears any overlay.
- Decision: a rebind appends only notices that describe a change: the
  existing `provider changed · a → b` notice when the model changed, the
  reasoning notice when thinking or effort changed or a saved override was
  cleared, a context-window notice when the window changed, a profile notice
  when the profile changed, and the recovery notices the new host reports.
  It does not repeat setup notices.
- Decision: per-host locals stay inside `run_tui` and are created for each
  host. Shell shortcut anchors refer to the session's history, which a
  same-session rebuild keeps, so a shortcut finished before the rebuild is
  already saved; none can be in flight because the rebuild requires idle.
- Decision: `/new` and `/resume <other>` build a new app for the new
  session, then move the composer history across.
- Decision: `Terminal` gains suspend and resume. Connect and disconnect
  suspend it before their own flows, which enter their own terminal, and
  resume it afterwards. Quit, an active-provider disconnect, and errors
  restore it before printing. Entering once also stops re-sending the
  `ESC[6n`-sensitive setup that `terminal.rs` warns about on re-entry.
- Decision: the driver refuses a reconfigure exit while `app.is_busy()` or a
  prompt is pending, with the existing "requires an idle turn" notice.

## Risks / Trade-offs

- Status that was accumulated live (not from durable records) would be lost
  by re-seeding. Mitigation: everything status shows is re-derivable from
  the host's records and timeline events, which is what resume already
  relies on; tests compare status after a rebind with status after a resume.
- A long session keeps its whole transcript in memory across rebuilds, as it
  already does within one host.
