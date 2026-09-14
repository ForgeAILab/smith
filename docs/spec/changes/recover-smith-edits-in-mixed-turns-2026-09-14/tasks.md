# Tasks: Recover Smith's own edits in mixed turns

## 1. Attribution model

- [x] 1.1 Add `TurnChangeSet::exact_mutations()`, `has_exact_mutations()`, and
      `ambiguous_tools()`; leave `is_fully_attributable()` semantics intact for
      the "exact end to end" label and the journal field.
- [x] 1.2 Label a mixed turn `mixed` (not `ambiguous`) in the change timeline.

## 2. Recovery paths

- [x] 2.1 Extract `undo_preview_text` and `ambiguous_note`; render the preview
      once so `undo_preview` and `record_undo_cancelled` journal the same
      fingerprint.
- [x] 2.2 Gate `undo_preview`/`undo_latest` on `has_exact_mutations()` plus
      `!undone`; keep the per-path post-image conflict check and the
      all-or-nothing application over the exact set.
- [x] 2.3 Make `redo_direction`/`redo_preview_text`/`redo_latest` operate on
      the exact subset so a partial undo is reversible; drop the
      `unreachable!` that assumed a fully exact set.
- [x] 2.4 Refuse with a self-explaining message when a turn has no exact
      mutation at all.

## 3. Local surfaces

- [x] 3.1 Transcript notice: keep "contains ambiguous changes", add that
      `/undo` covers Smith's own edits and `/diff` shows the rest; distinguish
      the ambiguous-only case.
- [x] 3.2 `/status` attribution line: undone / no exact mutation / fully
      attributable / mixed.
- [x] 3.3 Default `cache.miss_notices` to `true` in the built-in layer and
      update its precedence tests.

## 4. Validation

- [x] 4.1 Unit tests: mixed turn undoes the exact edit and leaves the shell
      path byte-identical; preview names the unattributable tool; redo restores
      it; a mixed turn whose edited path was overwritten refuses without
      writing; an ambiguous-only turn has no undo candidate.
- [x] 4.2 `cargo fmt --check`, `cargo clippy --workspace` (warnings are
      errors), and the `smith-tools`, `smith-config`, `smith-runtime`,
      `smith-tui`, `smith-cli` test suites.
- [ ] 4.3 Live check in the TUI: run a turn that edits a file and then runs a
      shell command, confirm the notice wording, `/undo` preview contents, and
      that the shell-written path survives the undo.
