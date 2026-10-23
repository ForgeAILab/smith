## Context

The TUI already folds root and child events through `ConversationMut` and
renders the selected child through `conversation_lines`. This is the correct
foundation. However, `subscribe_to_child` joins only after the parent observes
`ChildSpawned`; `DelegationCoordinator::child_events` explicitly does not
replay events from before subscription. The inspector report contains status
and the last result, not a history snapshot. Its bounded client log therefore
cannot serve as the complete transcript source.

The model-facing `AgentAction` offers spawn/list/wait/result/follow-up/resume/
stop. Its result path returns the completed task outcome, not the child's
conversation. Adding more status fields alone would not satisfy this request.

Provider-exposed reasoning is buffered under the same speculative/commit
policy for both conversations. A transcript reader must respect that policy;
an absence of visible text is not evidence of a particular internal activity.

## Decisions

### One canonical conversation, two readers

Project child-owned canonical history into the local child screen and the
parent's on-demand transcript tool. Keep lifecycle/status separate from
conversation content, but return the observed lifecycle state with each read.
Do not create a second execution loop or infer messages from lifecycle labels.

For each accessible message, preserve its role and order. Include the
delegated task, assistant text, permitted provider-exposed reasoning, reviewed
tool invocation details, complete accessible tool results, and final reply.
Internal system/developer prompts, credentials, and protected reasoning remain
outside this projection, as they do for ordinary user transcript access.

### Read history and attach a live tail without a gap

The read seam must expose a stable watermark and continuation cursor. Attach
the live subscription before capturing the historical boundary, buffer live
events during hydration, and reconcile by canonical identities/watermarks.
Do not merge a history read and an unrelated live subscription by timing or
text comparison. Add an upstream seam if the existing public APIs cannot
provide the required boundary.

Persisted children must be readable without rebuilding a provider. Reuse the
existing protected session/checkpoint/history storage and journal composition
where sufficient. If that storage lacks a retained transcript segment, add
the missing durable history seam rather than interpreting the root's child
lifecycle journal as the child's full record. Apply root-equivalent retention;
explicitly identify unavailable segments and never present a summary as their
verbatim replacement.

### Bounded pages preserve full access

Use `agent` action `transcript` with `child_id`, an optional opaque cursor,
and a bounded page size. Responses identify the child/session, snapshot
watermark, committed messages, current state, and next cursor. Paging an
existing snapshot must not shift because a child appends a new message.
Large individual entries use existing artifact-backed output with references
the requesting parent can actually read. UI paging may evict rendered rows
from memory but must retain access to older stored content.

### Completion remains the coordination mechanism

The parent should continue independent work or end its turn and park after
spawning a child. Existing completion admission supplies the final outcome,
including artifacts for replies outside the inline budget. A transcript read
is diagnostic and does not acknowledge or consume that outcome. Existing
`wait` and `result` acknowledgement semantics remain authoritative.

Capability guidance should explicitly discourage repeated status/transcript
polls and distinguish child stop from shell-task stop. No new shell-task wait
API is part of this change.

## Risks / Trade-offs

- Loading a complete child conversation can be large. Page it and preserve
  readable output artifacts rather than loading it all into parent context.
- A late subscriber can miss a committed attempt even when the child later
  appears live. Tests must exercise the boundary, not just normal streaming.
- Child compaction, retention, or unavailable protected storage can limit
  recovery. Report these limits explicitly and match the main transcript's
  retained-history behavior.
- Live text may later be discarded on a provider retry. Keep any speculative
  local presentation separate from committed transcript pages.
- A provider may expose hosted search activity without assistant text or
  reasoning. Show the actual observations without inventing a transcript.

## Migration Plan

Additive child-tool action and transcript projection. Preserve existing child
IDs, result delivery, stop semantics, and persisted session compatibility.
Existing sessions with unavailable historical segments remain inspectable with
an explicit limitation. No session files are rewritten as part of planning.

## Implementation Notes

Smith pins Agent Runtime commit
`bba8996c213f2f65287d84befc1c7dc6ced3241a`. Its `child_history` and
`subscribe_child_history` APIs return immutable canonical history, an exclusive
event watermark, pending output seeds, and retained hosted/native activity.
Sensitive activity survives ordinary-store redaction through the protected
checkpoint overlay. Native assistant prefixes are committed at tool
boundaries, preserving commentary/tool/reply order after restart.

The local child view loads the available canonical conversation into its
existing scrollable renderer. The parent-model tool pages the same source,
retaining up to 16 immutable snapshots and returning explicit cursor-expiry
errors. Oversized accessible entries are credential-redacted and stored as
parent-owned artifacts. Neither reader acknowledges task completion.
