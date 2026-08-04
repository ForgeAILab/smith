## Context

`TurnCompleted` carries the authoritative finish state, and every event
envelope already carries a millisecond timestamp. The TUI also owns a local
monotonic timer for the active `Working…` row. The recently completed quiet-turn
change removed the successful transcript notice entirely; the user instead
wants the terminal evidence and duration retained without the inaccurate
`reasoning only` interpretation or a rounded `0s` label.

Smith's protected runtime event intentionally carries tool name, argument keys,
and a fingerprint but not raw values. The standard interactive host can resolve
the canonical in-process `ToolCall` and pass it through a reviewed built-in
projector. That projector already renders `Read(path · lines …)`, but the CLI
attempts enrichment only at `ToolCallRequested`. The observed completed
`read(limit, offset, path · values protected) · ok` row proves that lookup can
miss at that boundary and is never retried.

The existing projector also hides search patterns and shell commands even from
the local interactive user. The requested policy is narrower: display bounded
ordinary operation inputs and redact credential-bearing fields/literals, while
continuing to omit bulk bodies/results from the compact transcript row.

## Goals / Non-Goals

### Goals

- Keep a concise successful completion row with honest elapsed-time evidence.
- Make completed Smith built-in rows reliably identify what operation ran,
  where it ran, and the relevant bounds/options.
- Reuse the existing structural credential classifier and registered-secret
  literals before local display.
- Preserve protected runtime events, journals, machine output, replay parity,
  terminal safety, and compact layout at supported widths.

### Non-Goals

- Adding raw arguments to `RuntimeEvent`, JSON/stream JSON, or journal records.
- Printing unbounded edit bodies, tool-result bodies, or artifact contents in
  the normal transcript.
- Treating unknown third-party schemas as safe by guessing field semantics.
- Changing tool execution, approval, workspace, or provider authority.
- Restoring the `without a visible answer (reasoning only)` diagnosis.

## Decisions

### Successful completion uses canonical duration evidence

The reducer stores the current turn's envelope timestamp when it sees
`TurnStarted`, independently of the monotonic `Instant` used for the animated
live working row. On `TurnFinish::Completed`, a valid non-decreasing start/end
timestamp pair becomes a `Duration` and the TUI appends:

```text
• turn · completed in 842ms
• turn · completed in 12s
```

Sub-second duration uses bounded millisecond precision and never rounds to
`0s`; a zero-millisecond interval renders `<1ms`. Longer durations reuse the
existing compact second/minute/hour grammar. If canonical timing is absent or
invalid, the row says only `turn · completed`; it does not substitute local
replay processing time or fabricate a duration. `visible_output` does not add
a `reasoning only` suffix because the finish event does not establish why text
was absent.

Non-success finishes retain their existing attributed wording and live elapsed
behavior. Every terminal still performs cleanup and reconciliation before the
notice is appended.

### Built-in enrichment is retried at completion

The interactive host continues its cheap request-time canonical lookup so a
running row can become informative immediately. After reducing
`ToolCallCompleted`, the CLI asks for the same reviewed projection again. If
the request-time lookup missed, the completed row is enriched by stable call
ID; if it already succeeded, replacement is idempotent. This fixes
presentation ordering without changing Agent Runtime's event contract.

### Display a redacted clone, never raw canonical arguments

Before projection, the host clones the matching canonical argument object and
applies Smith's existing redaction policy:

- structural credential-shaped keys such as API key, authorization, access or
  refresh token, password, private key, credential, bearer, and secret become
  `[redacted]`;
- exact provider credentials and sensitive literals already registered with
  the session redactor are scrubbed wherever they appear in strings;
- the original canonical history remains unchanged.

Only that redacted clone crosses into the display projector. Thus ordinary
paths, limits, patterns, commands, flags, and timeouts are not mislabeled as
secret, while known credential material remains protected.

### Compact built-in grammar

The projector remains typed per Smith-owned schema and produces one bounded,
control-free logical row. The intended grammar is:

```text
• Read(src/lib.rs · offset 4 · limit 20) · ok
• List(crates · recursive · limit 50) · ok
• Search("ToolCallRequested" · crates · extension rs · limit 20) · ok
• Edit(src/lib.rs · replace all) · ok
• Shell(cargo test -p smith-tui · cwd . · timeout 30000ms) · ok
```

The read row names offset and limit directly rather than converting them only
to a derived line interval. Search patterns and shell commands are ordinary
operation inputs after redaction and are shown within strict character bounds.
Edit old/new bodies and all tool-result bodies remain outside this compact row
because they are bulk content already available through diff, approval,
artifact, and explicit detail surfaces; their omission is not described as
secret protection.

Unknown tools, malformed calls, and calls that still cannot be resolved render
the tool plus argument keys and `details unavailable` or `unknown schema`.
They never say all values are protected and never guess values that were not
available to the projector.

### Alternatives considered

- Keep successful terminals silent: rejected because it removes useful finish
  and duration evidence the user explicitly wants.
- Reuse local `Instant` for replayed terminal duration: rejected because it
  measures reducer speed, not turn runtime.
- Enable raw runtime-event arguments: rejected because it discloses values to
  every journal, observer, and machine consumer.
- Keep request-only enrichment: rejected because the observed real row proves
  that presentation boundary can race canonical visibility.
- Show edit/result bodies inline: rejected because it destroys the compact
  transcript and duplicates explicit diff/artifact surfaces.

## Risks / Trade-offs

- Canonical wall-clock timestamps can theoretically move backward; invalid
  pairs omit duration rather than wrapping or claiming a value.
- Search patterns and shell commands can contain project-sensitive text. They
  are visible only in the local interactive surface, bounded and scrubbed with
  registered secrets; shared events and persistence retain their existing
  redaction boundaries.
- A credential unknown to Smith cannot be literally scrubbed from an arbitrary
  command string. Structural sensitive keys and all credentials resolved by
  Smith are covered. This is the same explicit trust boundary as other local
  canonical-history views and must be documented.
- Completion-time lookup adds one bounded history search per completed tool.
  If profiling shows material cost, a host-local call-display index can replace
  it without changing the display contract.

## Migration Plan

- No persisted schema or runtime-event migration.
- Existing saved sessions acquire the expanded display when reconstructed by
  the new client because projections derive from canonical history.
- Completion notices are local presentation records and do not become model
  conversation messages.
- Update `DESIGN.md` first, implement and verify deterministically, then
  reinstall the local release binary.

## Open Questions

- None. The proposal fixes the observed known-built-in fallback and defines a
  bounded credential-redacted operational display without widening machine
  output or bulk-content surfaces.
