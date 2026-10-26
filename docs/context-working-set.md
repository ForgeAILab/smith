# Bounded tool evidence

Smith keeps the evidence needed to inspect tool work without replaying every
large text result into every subsequent model request.

```toml
[context]
tool_output_inline_bytes = 8192
```

The default threshold is **8192 serialized bytes**, not an exact token count.
Normal configuration layering applies, including
`SMITH_CONTEXT_TOOL_OUTPUT_INLINE_BYTES`; valid values are 256 through 1048576.
When a text-only tool outcome exceeds the effective threshold, Smith stores the
unchanged captured outcome in the existing session-owned artifact store and
returns a small reference/preview. Raising this setting trades larger direct
model views for fewer artifact lookups. Root and child runtimes use the same
resolved policy, including explicitly selected child-profile routes.

## Three different bounds

1. Tool/process capture limits determine how much output was captured in the first
   place. This change neither raises them nor recovers data already discarded.
2. `context.tool_output_inline_bytes` decides when a serialized text outcome
   moves into recoverable artifact storage. Metadata and content both contribute.
3. `limits.tool_output_limit_bytes` is the existing final model-facing rendering
   ceiling; it is not the process capture limit. The effective offload threshold
   is the smaller of the inline setting and this existing ceiling.

A completed child's reply is the exception. An `agent` outcome that carries
one (`result`, or `wait` on an idle child) and an automatic child-completion
delivery stay inline up to `limits.tool_output_limit_bytes`, so the parent reads
a report as text in one step. A longer reply follows the ordinary rule.

A fetched page is the other exception. A `web_fetch` result stays inline up to
`limits.tool_output_limit_bytes` (64 KiB by default). A longer page is stored
whole, and the model sees its beginning up to that ceiling with a note giving
the `artifact.read` offset at which the unshown text starts in the stored
result, so reading on neither repeats the beginning nor guesses.

A preview contains reported status metadata, head/tail excerpts and at most three
explicitly **heuristic** diagnostic lines. At the default threshold its body is
bounded to 1200 Unicode characters, plus the runtime-generated artifact marker.
An `error` line in a successful command does not turn the result into a failure.
This is not a semantic summary or a promise to select every relevant log line.
Inline typed images and opaque continuation are never replaced by this text-only
processor. Existing tool capture/truncation flags are preserved in exact evidence.

## Bounded expansion

`artifact.read` accepts the usual artifact identifier, offset and valid requested
limit. Smith uses a smaller default and clamps larger valid limits during
preparation, before authorization of the prepared call. The raw page bound is
`clamp(effective_inline_bytes / 4, 1, 4096)`, so the default is **2048 bytes**.
If `artifact.read` is not in the current tool list, first call `registry.search`
with query `artifact-read`; the next provider request advertises the authorized
reader. Previews include this guidance rather than eagerly activating every tool.
Follow `next_offset` to inspect more evidence rather than loading everything.
The stored object is the original serialized tool outcome, not a changed raw-text
file format. A text page never splits a character: an edge inside a multi-byte
character moves to that character's start, and `offset` and `next_offset`
report the bytes actually returned. Content that is not UTF-8 comes back as a
byte array using the existing runtime format. Raw page bytes are not rendered
JSON bytes or exact model tokens.

Prepared invocations cannot exceed the current bound. A changed policy may
therefore reject an old incompatible prepared artifact read instead of silently
reusing it. Session ownership, integrity checks and artifact-transfer policy stay
unchanged. Artifact pages are not recursively offloaded into new artifacts.

## Listing artifacts

When Smith's own artifact store backs the session, `artifact.list` sits beside
the reader (discovery query `artifact-list`). It returns metadata only, never
content: id, purpose, media type, byte length, originating turn and call, the
child that produced it (`child_id`), and the child session a copied artifact
came from. Rows are the calling session's own artifacts, newest first, 20 per
page by default and at most 100, sized to fit the inline allowance; follow
`next_offset`. Optional `purpose` and `child_id` filters narrow them; for
example `child_id = "child-1"` with `delegation.child-reply` finds that child's
full reply. Other purposes include
`delegation.child-result` for artifacts a child produced and `tool-output` for
your own offloaded results. Listing needs the same authority as reading.

Artifact ids come only from tool results and host messages; they are never
constructed. When a read names an id that does not exist, the error says so and
points to `artifact.list`.

## Inspection and recovery

`/context` retains its existing last-request category breakdown and confidence
report. It also names the effective offload/page limits and separates request
occupancy from cumulative provider usage. If no artifact store is installed, the
inspector says so; ordinary output limits still apply. This feature does not
create a store for an explicitly ephemeral runtime.

Offloaded evidence survives protected-session restart through the same storage
mechanism as before. Do not delete session files to change these limits. The
separate upgrade-resume fix should ship before this tool-descriptor change so
valid stale unfinished sessions can be interrupted rather than stranded.

## Scope

This is the first stage of the context work: reducing avoidable **new** inline
tool-result growth. It does not compact existing history, change the 85/60
watermark path, enable semantic summarization, guarantee an unlimited single
task, or add `/compact`. Many small results and retained multimodal input can
still fill a window; the final request planner remains authoritative. Durable
semantic compaction and safe mid-task cutovers require the next independent
runtime/host change. No background provider requests or summary spending are
enabled by this stage.
