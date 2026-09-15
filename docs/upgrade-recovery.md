# Session upgrades and readable diagnostics

Smith keeps durable conversation history separate from operational tool activation.
A completed conversation is re-authorized against the current registry on resume.
An unfinished ordinary turn with an incompatible registry/scoped view is stopped
without replay, using Agent Runtime's explicit `ResumeOrInterrupt` host policy.
The conversation, usage ledger, existing results, and identity counters are retained.
Unanswered headless interactions remain deferred for an interactive host. Active
goals are paused when upgrade recovery interrupts a turn.

Smith displays a recovery notice. Review earlier edits before retrying the last
action: an interrupted external effect may have happened even without a committed
result. This mechanism never treats an unknown effect as success and never reuses
an old approval. Do not delete session files to repair an activation mismatch.

Malformed/future activation schemas, authenticated checkpoint failures, and cache
operation idempotency failures remain errors. This is not a blanket ignore-errors
mode and does not promise exact replay across incompatible executable definitions.

## Normal output

`/status` shows session, provider, usage, saved-checkpoint time, and workspace state.
`/diagnostics` (also `/status --verbose`) retains the detailed cache, scheduler,
resume-capsule, and runtime metadata. The normal exit summary does not dump inactive
maintenance or capsule internals. Synthetic attempts with actual activity remain
visible, and headless JSON retains its typed numeric timestamp fields.

`prompt cache: 72% of input read from cache` is a provider usage observation. It is
not a guarantee that the next request will hit, and it does not require support
for explicit cache-maintenance APIs. Missing usage is not reported as zero. An
absent optional semantic summary is not a broken conversation.

## Time

Human timestamps use the local OS timezone with an explicit UTC offset, resolved
at the represented instant. Milliseconds remain appropriate for elapsed latency,
not for displaying Unix-epoch persistence or deadline timestamps. Named-timezone
subprocess tests cover Toronto winter/summer offsets under a multithreaded host.
