---
created_at: 2026-10-03T00:00:00Z
updated_at: 2026-10-04T03:45:36Z
completed_at: 2026-10-04T03:45:36Z
---

Approved 2026-10-03 ("i think you can continue with the road map. as we do
need to get this a good ui ux. and strong backend."). Rest of roadmap step 3.

## 1. Confirmations

- [x] 1.1 `Overlay::Confirm(ConfirmDialog)` replaces the ten confirm variants
  and the exit confirmation; one renderer and one key handler (`y` accept,
  `n`/Esc cancel, Enter nothing, arrows and PageUp/PageDown scroll, 500 ms
  guard). Titles, warnings, and accept labels keep today's wording.
- [x] 1.2 Bodies scroll to their end; undo, redo, and revert patches are no
  longer cut at a fixed line count.

## 2. Overlay policy

- [x] 2.1 `App::open_overlay` is the only writer of the overlay slot.
  Prompts queue FIFO with approvals and questionnaires; transient overlays
  close when a prompt arrives and cannot open over one.

## 3. Notices

- [x] 3.1 `NoticeKind` in `smith-client` fixes each notice's label
  (today's wording) and persistence (`Transcript` or `Feedback`); every
  `push_notice` call site uses a kind.
- [x] 3.2 Feedback notices show in the hint row until the next keypress and
  never enter the transcript. List every Feedback kind and its call sites
  for review.

## 4. Verification

- [x] 4.1 Tests: each confirmation opens, scrolls, accepts, and cancels;
  queue order across approval, questionnaire, and confirmation; a prompt
  closes a picker and a picker cannot open over a prompt; feedback appears in
  the hint row, not the transcript, and clears on the next key.
- [x] 4.2 `cargo fmt --all -- --check`, strict Clippy, workspace tests; review
  every re-recorded fixture; `final_checks.py` passes; update DESIGN.md.
