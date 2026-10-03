---
created_at: 2026-10-03T22:44:48Z
updated_at: 2026-10-03T22:44:48Z
completed_at:
---

Approved 2026-10-03 ("yes, lets do those please", after the roadmap
summary named this step). Depends on `refactor-tui-state`.

## 1. One terminal per run

- [ ] 1.1 Enter the terminal once in `run_interactive_command` and pass it
  to each `run_interactive`. Restore it before any printed exit: quit
  report, active-provider disconnect message, and errors.
- [ ] 1.2 `Terminal` gains suspend and resume. Connect and disconnect
  suspend it around their own flows and resume it afterwards, then redraw.

## 2. One app per session

- [ ] 2.1 Split `run_interactive`'s setup into seeding a new app and
  rebinding an existing app to a new host, per `design.md`. Both re-derive
  status (fresh, then re-seeded), resources, account, children, usage
  records, and cache events the same way.
- [ ] 2.2 `run_tui` returns the app with its exit; per-host locals stay
  inside it and are created for each host.
- [ ] 2.3 A same-session rebuild (selection changes, Tab cycling,
  `CapabilitiesChanged`, connect, disconnect of another provider) rebinds:
  transcript, folded state, scroll, `work_details`, composer and history,
  and the inspector logs of children still listed are kept; the live turn is
  reset through the single reset; overlays are cleared; the transcript is not
  rebuilt from history.
- [ ] 2.4 A rebind appends only notices for what changed (model, profile,
  thinking or effort, context window, cleared override, host recovery
  notices). No setup notice repeats.
- [ ] 2.5 `/new` and `/resume <other>` build a new app for the new session
  and carry composer history across.
- [ ] 2.6 The driver refuses a reconfigure exit while busy or while a prompt
  is pending, with the existing requires-idle notice.

## 3. Verification

- [ ] 3.1 Tests: a model switch keeps blocks, folded state, scroll, and
  composer history and appends one notice; status after a rebind equals
  status after a fresh resume of the same session; Tab cycling; `/resume`
  of another session replaces the transcript and keeps history; a
  reconfigure while busy is refused.
- [ ] 3.2 `final_checks.py` gains a surface: `/status`, then `/model` to
  another model; the `/status` rows, a model notice, and Up recalling the
  earlier prompt are on screen, and the screen was never cleared.
- [ ] 3.3 `cargo fmt --all -- --check`, strict Clippy, workspace tests, and
  the PTY checks at 100x32, 80x24, 44x16, and no-colour.
- [ ] 3.4 Live check with the owner's config: switch model and effort mid
  session, connect flow cancel, then quit and resume.
