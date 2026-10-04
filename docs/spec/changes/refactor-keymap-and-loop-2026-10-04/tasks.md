---
created_at: 2026-10-04T00:00:00Z
updated_at: 2026-10-04T05:25:02Z
completed_at:
---

Approved 2026-10-03 ("i think you can continue with the road map. as we do
need to get this a good ui ux. and strong backend."); audit items S5 (keymap
table) and S8 (`run_tui`). No visible change.

## 1. Key table

- [ ] 1.0 Tests for the ordering rules first: Ctrl+C twice leaves during an
  approval and a confirmation (including inside the quiet window);
  PageUp/Home/Ctrl+L work inside an approval's quiet window; Esc closes the
  shortcuts panel without acting and other keys close it and act; Ctrl+O
  toggles detail with a confirmation open. They pass before 1.1.
- [ ] 1.1 Typed key table in `smith-client` (own chord type, context,
  effect, help text); `help_keys()` and the shortcuts panel render from it
  with unchanged text.
- [ ] 1.2 `smith-tui` test checks every binding against `reduce_key` in its
  context; footer-hint keys are bindings in the table.

## 2. Interactive loop

- [ ] 2.1 `run_tui` state in one struct; one method per `select!` arm and
  one for actions; arms and action arms moved verbatim in their order;
  `run_tui` constructs the state and runs the loop.

## 3. Verification

- [ ] 3.1 `cargo fmt --all -- --check`, strict Clippy, workspace tests,
  `cargo deny`; every fixture byte-identical; `final_checks.py` passes.
