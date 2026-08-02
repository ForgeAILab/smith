## Context

`Composer` currently owns a 100-entry `VecDeque` containing only drafts
cleared by the first `Ctrl+C`. `App::reduce_key` gives `Up` and `Down` to this
recall buffer only when the composer is empty or already recalling; otherwise
the keys scroll the transcript. Accepted input is cleared through several
provider, command, shell, and confirmation paths, so history recording must
occur at the shared acceptance boundary rather than on every `Enter` press.

The existing active `add-agent-first-workflow-ux` change defines interrupted
draft recall as bounded process-local UI state. This change extends that model
without making composer history part of canonical runtime history or durable
session recovery.

## Goals / Non-Goals

- Goals:
  - Recall accepted prompts, slash commands, shell shortcuts, and interrupted
    drafts from one history.
  - Browse history without losing the draft that preceded navigation.
  - Search history locally with familiar reverse-search controls.
  - Preserve overlay ownership, double-`Ctrl+C` exit, Unicode safety, and
    bounded memory.
- Non-Goals:
  - Persist composer history across Smith processes or session resumes.
  - Import canonical model messages into composer history.
  - Synchronize history across projects or redact input beyond existing
    process-memory behavior.
  - Add fuzzy ranking, regex syntax, or a second focusable input surface.

## Decisions

### One bounded history with a navigation scratch draft

The composer keeps at most 100 exact, non-blank entries, oldest first. Both an
accepted submission and the first `Ctrl+C` use the same insertion operation.
An entry identical to the newest entry is not inserted again. Input rejected
by local validation remains in the composer and is not recorded.

When no overlay owns input, the first `Up` saves the current composer text as a
scratch draft and selects the newest history entry. Repeated `Up` walks older
entries; `Down` walks newer entries and restores the scratch draft after the
newest. Editing a recalled entry exits navigation while leaving the selected
text editable. Starting a later navigation captures that edited text as a new
scratch draft but does not record it until accepted or stashed.

This intentionally changes non-overlay `Up` on a non-empty composer from
transcript scrolling to history navigation. `PageUp`, `PageDown`, and explicit
scroll commands remain available for transcript movement.

### Reverse search is an overlay over the same state

`Ctrl+R` opens a compact reverse-search overlay only when no other overlay owns
input. The original composer draft is retained for cancellation. Ordinary
typing and `Backspace` edit a case-insensitive substring query; the newest
match is selected and repeated `Ctrl+R` cycles toward older matches, wrapping
after the oldest match. `Enter` places the selected exact entry in the
composer at its end without submitting it. `Esc` restores the original draft.
An empty or unmatched query displays an explicit local state and never mutates
history.

Because `Ctrl+C` remains globally checked before overlays, its first press
while reverse search is open restores the original draft, records that draft
through the ordinary stash operation, closes search, and clears the composer.
A second press within the existing one-second window exits.

### Record at accepted-input boundaries

The application records the exact pre-normalization composer text immediately
before an accepted flow clears or transfers ownership of it. This includes
literal escapes, provider prompts, prepared shell shortcuts, parsed local
commands, and valid child confirmation flows. Blank input, parse failures,
unresolved references, unavailable commands, and busy-state rejections are
not recorded because the user still owns the unchanged draft.

Alternative considered: record every non-blank `Enter`. Rejected because it
would duplicate drafts that remain visible after validation errors and make
history imply an action was accepted when it was not.

## Risks / Trade-offs

- Accepted input can contain sensitive text in process memory. The existing
  interrupted-draft buffer already has this property; keeping the buffer
  bounded and non-persistent avoids expanding the storage boundary.
- Multiple submission branches can drift if they record history separately.
  A shared acceptance helper and branch-focused reducer tests mitigate this.
- Reassigning non-overlay `Up` from transcript scrolling may surprise existing
  users. Existing page scrolling remains intact, and help text will state the
  new ownership explicitly.
- Substring search is predictable and inexpensive at 100 entries, but less
  flexible than fuzzy search. Fuzzy ranking is intentionally deferred.

## Migration Plan

No persisted state or schema changes are required. Existing stashed drafts are
represented by the same bounded in-memory queue during the process lifetime;
the implementation refactor only expands which accepted inputs enter it.

## Open Questions

None. Durable, cross-process history can be proposed separately if product
usage shows that process-local recall is insufficient.
