## Context

`Composer` stores one `String` and a cursor counted in Unicode scalar values.
Large pastes and clipboard images are stored separately by `App`, while the
composer contains their complete display labels. Rendering recognizes only
registered labels and accents them, and submission uses the same registrations
to expand raw paste text or attach image content. Because editing currently
knows only about characters, the cursor can enter those labels and deletion can
damage only part of one.

The completed composer-history and pending-input changes intentionally retain
the compact editable string plus cloned out-of-band material. Today the same
compact string is also appended to the committed user transcript. This change
must preserve the former model and queue ordering while giving committed text
pastes a separate expanded transcript projection.

## Goals / Non-Goals

- Goals:
  - Give each registered paste or clipboard-image placeholder exactly two
    horizontal cursor stops: immediately before and immediately after.
  - Remove a registered placeholder with one adjacent backward or forward
    deletion.
  - Preserve raw paste bytes after existing newline normalization, image
    payloads, and submission order.
  - Show original pasted text in the committed user transcript while keeping
    image placeholders visible there.
  - Keep typed text, typed paths, and unregistered lookalikes ordinary and
    fully editable.
- Non-Goals:
  - Change when text is large enough to collapse.
  - Change clipboard acquisition, image encoding, limits, or API content-part
    support.
  - Expand text-paste labels in uncommitted queue previews or composer-history
    recall entries.
  - Treat `@file` references, slash commands, or arbitrary bracketed text as
    atomic tokens.
  - Add selection, drag-and-drop, clipboard history, image previews, or a
    general rich-text editor.

## Decisions

### Registered ranges define atomicity

Atomic behavior is derived from placeholder ranges backed by the current
`PastedChunk` and `ImageAttachment` registrations. Matching is performed on
the complete label and converted to character offsets before editing. A typed
path such as `assets/photo.png`, a typed `[Image #1]`, or a stale label whose
material is no longer registered has no matching range and remains ordinary
text.

This retains the existing security and intent boundary: only the explicit
clipboard-image path can create image content, and ordinary text is never
silently promoted because it resembles a path or label.

### Atomic navigation is a logical view over the existing display string

The composer keeps its exact editable string so rendering, history, pending
previews, and reference parsing remain compatible.
Horizontal movement consults registered ranges: `Left` at a range end lands at
its start, and `Right` at a range start lands at its end. Everywhere else each
key moves by one Unicode scalar as it does today.

Any cursor-placement path that would land inside a registered range clamps to
the nearest boundary using a deterministic direction appropriate to that
operation. This invariant prevents later insertion from splitting a live
placeholder even after restoration or positional movement.

Alternative considered: replace the composer string with a full text/token
piece table. Rejected for this bounded change because it would force unrelated
history, parser, renderer, and submission migrations without improving the
required two-boundary interaction.

### Deletion removes one complete registered range

`Backspace` at a placeholder end removes from its start through its end;
`Delete` at its start removes the same range. Deletion elsewhere removes one
ordinary Unicode scalar. Removing a label from the draft is sufficient to
detach it from that submission because preparation already selects only
material whose registered label is present. Prepared queued or accepted input
continues to own its cloned material independently.

### Editable, committed, and provider projections remain explicit

The full label remains visible and accented, so the feature does not compress
its terminal width; only cursor and deletion semantics are atomic. Prepared
input retains three explicit views:

- The editable/pending/history string keeps both text-paste and image labels so
  restoration remains compact and the material can still be edited atomically.
- The committed user-transcript string expands every registered text-paste
  label to its exact stored text but keeps every registered image label.
- Provider input uses the expanded text plus each registered image data part in
  placeholder order.

Keeping a dedicated committed transcript string avoids making transcript
presentation depend on provider materialization or overloading the compact
string used for pending restoration. Manually typed image paths and label
lookalikes contribute text only in every view.

## Risks / Trade-offs

- Character offsets and byte ranges can diverge for Unicode labels or nearby
  text. Central range conversion and mixed-Unicode tests mitigate accidental
  slicing at invalid boundaries.
- Adjacent placeholders have a shared boundary. Directional movement and
  deletion must select the placeholder on the side implied by the key; focused
  tests will lock that behavior.
- A stale unregistered placeholder becomes ordinary text. This is intentional:
  atomic styling and attachment behavior must never imply unavailable payload
  material.
- The cursor visually jumps across the full label width. That is the intended
  one-unit behavior even though the label still occupies multiple terminal
  columns.
- Expanded pasted text can make a committed user transcript block much taller
  than its editable placeholder. This is intentional because the committed
  transcript must show what was actually sent; ordinary transcript wrapping
  and scrolling bounds still apply.

## Migration Plan

No persisted state or provider schema changes are required. Existing live
drafts, history strings, queued submissions, and attachment records keep their
current representations. Prepared submissions gain or derive the committed
user-text projection needed when a whole turn or accepted steer becomes
visible.

## Open Questions

None. Text-paste labels are transient editing tokens; image labels and real
clipboard-image API submission remain intact after send, while typed
paths/lookalikes stay raw text.
