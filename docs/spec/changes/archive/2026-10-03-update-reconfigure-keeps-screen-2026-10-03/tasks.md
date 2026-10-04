---
created_at: 2026-10-03T22:44:48Z
updated_at: 2026-10-03T23:38:32Z
completed_at: 2026-10-03T23:38:32Z
---

Approved 2026-10-03 ("yes, lets do those please", after the roadmap
summary named this step). Depends on `refactor-tui-state`.

## 1. One terminal per run

- [x] 1.1 Enter the terminal once in `run_interactive_command` and pass it
  to each `run_interactive`. Restore it before any printed exit: quit
  report, active-provider disconnect message, and errors.
- [x] 1.2 `Terminal` gains suspend and resume. Connect and disconnect
  suspend it around their own flows and resume it afterwards, then redraw.

## 2. One app per session

- [x] 2.1 Split `run_interactive`'s setup into seeding a new app and
  rebinding an existing app to a new host, per `design.md`. Both re-derive
  status (fresh, then re-seeded), resources, account, children, usage
  records, and cache events the same way.
- [x] 2.2 `run_tui` returns the app with its exit; per-host locals stay
  inside it and are created for each host.
- [x] 2.3 A same-session rebuild (selection changes, Tab cycling,
  `CapabilitiesChanged`, connect, disconnect of another provider) rebinds:
  transcript, folded state, scroll, `work_details`, composer and history,
  and the inspector logs of children still listed are kept; the live turn is
  reset through the single reset; overlays are cleared; the transcript is not
  rebuilt from history.
- [x] 2.4 A rebind appends only notices for what changed (model, profile,
  thinking or effort, context window, cleared override, host recovery
  notices). No setup notice repeats.
- [x] 2.5 `/new` and `/resume <other>` build a new app for the new session
  and carry composer history across.
- [x] 2.6 The driver refuses a reconfigure exit while busy or while a prompt
  is pending, with the existing requires-idle notice.

## 3. Verification

- [x] 3.1 Tests: a model switch keeps blocks, folded state, scroll, and
  composer history and appends one notice; status after a rebind equals
  status after a fresh resume of the same session; Tab cycling; `/resume`
  of another session replaces the transcript and keeps history; a
  reconfigure while busy is refused.
- [x] 3.2 `final_checks.py` gains a surface: `/status`, then `/model` to
  another model; the `/status` rows, a model notice, and Up recalling the
  earlier prompt are on screen, and the screen was never cleared.
- [x] 3.3 `cargo fmt --all -- --check`, strict Clippy, workspace tests, and
  the PTY checks at 100x32, 80x24, 44x16, and no-colour.
- [x] 3.4 Live check with the owner's config: switch model and effort mid
  session, connect flow cancel, then quit and resume.

## 4. Found in the live check

- [x] 4.1 Resuming a session whose saved effort the startup model cannot
  represent fails to start (also in 0.3.1): `/model` to Gemini, `/effort
  low`, quit, `--resume` on the default `zai/glm-5.3`. The existing recovery
  (clear the override with a notice, `client-surfaces` "Model switch
  invalidates an override") never runs because `HostSessionError::Factory`
  is `#[error(transparent)]`, so `is_reasoning_startup_error` cannot
  downcast to `FactoryError`. Recognise the wrapped error; test with the
  error `start_host` actually returns.
