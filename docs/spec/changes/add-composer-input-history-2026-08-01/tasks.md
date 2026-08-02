---
created_at: 2026-08-01T22:42:33Z
updated_at: 2026-08-01T23:00:46Z
completed_at: 2026-08-01T23:00:18Z
---

## 0. Approval and baseline

- [x] 0.1 Approve the proposal, interaction design, and capability delta.
- [x] 0.2 Preserve the existing uncommitted profile/reasoning work in the TUI
  and record focused composer, reducer, and render baselines.

## 1. Shared composer history

- [x] 1.1 Refactor interrupted-draft recall into one bounded history model for
  accepted submissions and first-`Ctrl+C` stashes, retaining exact Unicode and
  multi-line text while suppressing adjacent exact duplicates.
- [x] 1.2 Preserve the pre-navigation composer draft, implement oldest/newest
  boundaries, and restore that draft when `Down` advances past the newest
  history entry.
- [x] 1.3 Add newest-first, case-insensitive substring matching with stable
  cycling and focused unit coverage for capacity, duplicates, Unicode, and
  empty history.

## 2. Keyboard and search interaction

- [x] 2.1 Record input only after composer validation accepts a provider
  prompt, local command/action, or confirmation flow; keep rejected input in
  place without adding a history entry.
- [x] 2.2 Wire no-overlay `Up`/`Down` navigation and retain existing arrow-key
  ownership inside pickers, approvals, questionnaires, and confirmations.
- [x] 2.3 Add a bounded reverse-search overlay for `Ctrl+R`, including query
  editing, repeated-`Ctrl+R` cycling, `Enter` acceptance, `Esc` restoration,
  and first-`Ctrl+C` stash/clear behavior.

## 3. Presentation and documentation

- [x] 3.1 Render reverse search in the existing anchored interaction area with
  accessible labels and narrow-terminal bounds; add reducer and snapshot
  coverage.
- [x] 3.2 Update local help and `DESIGN.md` with history scope, controls,
  precedence, capacity, and non-persistence.

## 4. Validation

- [x] 4.1 Run `cargo fmt --check`, focused `smith-tui` tests, workspace tests,
  and warning-denied Clippy without disturbing unrelated worktree changes.
- [x] 4.2 Re-run strict spec validation and document any release gate that
  cannot be exercised locally.

## Validation evidence

- `cargo fmt --all -- --check`
- `cargo test -p smith-tui` — 207 unit tests and 4 end-to-end tests passed
- `cargo test --workspace` — workspace, integration, PTY, and doc tests passed
- `cargo clippy --workspace --all-targets -- -D warnings`
- Strict `add-composer-input-history` spec validation
- The opt-in live-provider test remains ignored because it requires explicit
  credentials and spends provider quota; no hosted release gate was requested
  for this local TUI interaction change.
