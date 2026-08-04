## Context

Smith already has a transcript-first Ratatui surface, local slash-command
interception, interactive approvals, resumable sessions, and direct children.
The current design still models composer, transcript, and inbox as separate
focus targets and duplicates command access between slash input and
`Ctrl+P`. Live use showed that these mechanics are harder to understand than
the coding tasks they support.

Codex is a product reference, not an implementation dependency. The useful
pattern is one composer plus a searchable command surface, with diff/review
and explicit recovery available without leaving the conversation.

Smith must also preserve its stronger existing constraints: provider-neutral
runtime composition, honest status provenance, bounded approvals, user-change
preservation, and safe behavior outside Git repositories.

## Goals / Non-Goals

### Goals

- Make the composer the only persistent interaction focus.
- Make commands discoverable by typing `/`, with one registry shared by slash
  input, completion, help, and `Ctrl+P`.
- Keep transcript navigation available without entering a transcript mode.
- Make child/background activity visible without a focusable inbox region.
- Provide understandable diff, read-only review, last-turn undo, and
  file/hunk revert workflows.
- Preserve pre-existing work and fail closed when Smith cannot prove that an
  automated undo is safe.

### Non-Goals

- Copy Codex's full command catalog, visual design, or product-specific
  integrations.
- Add split panes, mouse-required workflows, staged commit/push/PR controls, or
  an embedded editor.
- Make arbitrary shell side effects automatically reversible when Smith cannot
  attribute them precisely.
- Support non-Git selective review/revert in the first implementation.
- Change the shared runtime delegation contract or provider behavior.

## Decisions

### One persistent focus

The composer owns focus whenever no modal is open. `PageUp`, `PageDown`,
`Home`, `End`, and `Ctrl+L` navigate the transcript globally. Background
notifications become inline transcript notices; the bounded internal inbox
continues to deliver safe-boundary content to the model but is no longer a
focusable visual region.

`Tab` and `Shift+Tab` navigate or complete the command menu only. Outside an
open completion menu they do not move focus. Approval, diff/revert selection,
model selection, and exit confirmation remain explicit modal states with their
own visible key hints.

Alternative considered: keep region focus but skip empty regions. Rejected
because it retains an invisible mode switch and consumes `Tab` for a behavior
unrelated to the primary conversation.

### One command registry

Typing `/` as the first non-whitespace character opens a filterable menu.
Arrow keys select, `Tab` completes without executing, `Enter` executes, and
`Esc` closes the menu while preserving the draft. `Ctrl+P` opens the same
registry with an empty query; it is an alias, not a second parser or command
set.

Commands that require an idle runtime report that constraint locally. The
first implementation does not queue host actions behind an active model turn.
`//` remains the literal-slash escape.

The initial user-facing registry is:

| Command | Behavior |
| --- | --- |
| `/help` | List commands and shortcuts. |
| `/status` | Show model, profile, permissions, context provenance, session, children, and Git/change state. |
| `/new`, `/resume` | Start or resume a session. |
| `/model`, `/provider`, `/profile` | Change runtime selection through the existing safe-boundary rebuild path. |
| `/agent` | List children and open a selected child's status/result details. |
| `/diff` | Inspect the current Git working-tree diff, including untracked files. |
| `/review` | Run a read-only review of a selected scope and report findings in the transcript. |
| `/undo` | Preview and reverse the last fully attributable Smith turn. |
| `/revert` | Select and preview current files/hunks to reverse explicitly. |
| `/quit` | Exit under the existing active-work policy. |

Alternative considered: add the full Codex command catalog immediately.
Rejected because commands without underlying Smith capabilities would create
false affordances and make discovery noisy.

### Change inspection

`/diff` is Git-backed and defaults to all uncommitted changes, including
untracked files. The view may filter to the last Smith turn, staged,
unstaged/untracked, a selected file, or a selected hunk. It is read-only and
available even when no change is attributable to Smith.

Outside a Git repository, `/diff`, `/review`, `/undo`, and `/revert` return a
local explanation without provider spend or workspace mutation.

### Read-only review

`/review` opens a scope selector for the last Smith turn, all uncommitted
changes, a commit, or a base-branch comparison. Review runs through a
read-only child/session surface and may not mutate the checkout. Findings
appear in the transcript with file/line evidence and severity; applying a fix
requires a later explicit user prompt.

### Turn attribution and undo

Smith records a `TurnChangeSet` in the session journal for every completed
turn that performs authorized mutations. Exact `edit` operations are
attributable and reversible. For shell or extension mutations, Smith records
the observed before/after Git delta and whether it can prove the delta belongs
to that tool boundary. A turn containing unattributable changes is visible in
`/diff` but is not eligible for automatic `/undo`.

`/undo` targets only the newest completed, not-yet-undone attributable turn.
It shows the reverse patch first and requires an explicit non-default
confirmation. The reverse applies only when every affected path still matches
the recorded post-image. Any overlap, missing file, external edit, or
ambiguous shell side effect fails closed and points the user to `/diff` and
selective `/revert`.

Alternative considered: implement undo as `git reset --hard` or
`git checkout --`. Rejected because those operations can erase user work and
cannot express turn ownership.

### Explicit revert

`/revert` operates on the current Git diff rather than inferred ownership. The
user selects an exact file or hunk, sees the reverse patch, and confirms with
no default action. Because selection is explicit, the operation may include
user-authored changes, but the UI labels origin as unknown unless a
`TurnChangeSet` proves Smith ownership.

Tracked content is reversed with a bounded patch. An unchanged untracked file
selected for removal is moved into recoverable Smith session storage before it
leaves the workspace. Every revert records its forward and reverse patch,
selection, confirmation, and outcome so a successful revert remains
recoverable during the session.

`Revert all` is deliberately absent from the first implementation.

### Presentation

The transcript remains the product. Command completion is a compact overlay;
diff/review/revert are temporary scrollable views that replace the transcript
area while active, never permanent panes. Child and monitor updates use concise
inline notices; `/agent` provides detail on demand. The bottom hint is static
for the composer except while a modal/view is active.

## Risks / Trade-offs

- Git-only review/recovery excludes non-Git projects in the first release but
  keeps destructive behavior understandable and testable.
- Precise attribution across arbitrary shell commands is inherently weaker
  than the `edit` tool contract. Smith exposes that uncertainty instead of
  pretending every observed diff is owned.
- Removing region focus may reduce keyboard access to a separate notification
  list; inline notices and `/agent` replace that path.
- A reverse patch can become stale quickly in an actively edited checkout.
  Post-image checks intentionally prefer refusal over convenience.
- Review children consume provider quota. The scope selector and confirmation
  must make that spend explicit before dispatch.

## Migration Plan

- Update `DESIGN.md` first, removing region-focus and visible-inbox contracts
  and defining command, diff/review, undo/revert, and modal states.
- Replace region focus state with composer-plus-modal state while retaining
  existing transcript scroll/follow behavior.
- Unify `Ctrl+P`, slash parsing, help, and completion over one registry.
- Introduce read-only Git inspection before any mutating recovery command.
- Add turn change attribution and journal replay compatibility.
- Add `/undo` only for proven-attributable turns, then selective `/revert`.
- Add `/review` after the read-only scope and child-session boundaries pass
  deterministic tests.
- Preserve existing journals: sessions without `TurnChangeSet` entries remain
  resumable but report that historical turns cannot be automatically undone.

## Open Questions

None for proposal approval. Staged/unstaged mutation controls, commit/push/PR
actions, non-Git recovery, queued busy-turn commands, and a broader command
catalog remain explicit follow-up changes.
