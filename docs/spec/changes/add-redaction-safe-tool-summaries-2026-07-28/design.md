## Context

`RuntimeEvent::ToolCallRequested` intentionally omits raw argument values by
default. It carries sorted keys and a fingerprint because model-generated
arguments can echo credentials or host data, and events fan out to persistence,
observability, and machine consumers. Smith currently renders that safe event
as:

```text
• Ran read · ok
  └ path · values protected
```

This is secure but not locally actionable. The canonical session history does
contain the validated tool call because the provider needs it for continuation,
and Agent Runtime appends that history entry before emitting
`ToolCallRequested`.

## Goals / Non-Goals

- Goals:
  - Show the built-in tool and its actionable target in one compact TUI row.
  - Preserve raw-argument protection for events, journals, observability, and
    headless machine output.
  - Render live and resumed calls consistently.
  - Keep projection bounded and resistant to terminal/control injection.
- Non-Goals:
  - Display tool-result bodies in the transcript.
  - Display arbitrary tool arguments, edit bodies, commands, or search text.
  - Change Agent Runtime's event vocabulary or enable
    `emit_raw_tool_arguments`.
  - Define summaries for third-party tools without an explicit Smith projector.

## Decisions

### Enrich only the trusted local TUI

On `ToolCallRequested`, the interactive host resolves the matching
`ContentPart::ToolCall` from `SessionHandle::history()` by stable call ID. This
is deterministic because Agent Runtime appends the canonical assistant message
before emitting the request event. The host passes only a typed display
projection into TUI state; the raw arguments never enter `RuntimeEvent`, the
event journal, or stream JSON.

If lookup fails, Smith keeps the current protected key summary. A display
failure never blocks or changes tool execution.

### Put built-in argument semantics beside the built-in tools

`smith-tools` owns a pure projector because it owns the schemas and knows which
fields are target metadata rather than arbitrary content. The projector returns
a typed invocation label, target, and bounded qualifiers rather than a generic
JSON value.

Initial projections are:

| Tool | Display | Excluded |
| --- | --- | --- |
| `read` | path; optional numeric line window | file content |
| `list` | path (default `.`); `recursive`/`all` flags | listing result |
| `search` | path (default `.`); case/limit flags | search pattern, extension, and matches |
| `edit` | path; optional `replace_all` flag | old/new text |
| `shell` | cwd (default `.`); optional timeout | command and output |

Targets and qualifiers are capped, line/control characters are normalized, and
missing or ill-typed allowed fields fall back to the protected row.

### Render a single invocation row

The transcript renders a running or completed call in a Claude-style compact
form:

```text
• Read(src/lib.rs) · ok
• List(. · recursive) · ok
```

The textual status remains present for no-color accessibility. Smith does not
append the tool result beneath the call.

On resume, `Transcript::replace_from_history` uses the same projector before
matching canonical tool results, so historical and live calls expose the same
information.

### Alternatives considered

- Enable raw tool arguments in Agent Runtime: rejected because every event
  subscriber and journal would receive arbitrary values.
- Add a summary string to the shared runtime event: rejected for this change
  because it expands a shared schema and still persists display data to every
  observer.
- Render all canonical arguments directly in the TUI: rejected because edit
  bodies, commands, patterns, and extension-defined values are unbounded and
  may contain secrets.
- Keep only protected keys: rejected because it does not tell the user what
  local resource the agent accessed.

## Risks / Trade-offs

- A path can itself be sensitive. This proposal intentionally exposes only
  built-in local target fields to the local interactive user, never to the
  event fan-out. Projects that persist canonical history already store those
  validated calls under the configured session redactor.
- Reading cloned canonical history for each tool request is linear in bounded
  session history. If profiling later shows material cost, Smith can maintain
  a host-local call index without changing the display contract.
- Third-party tools remain less descriptive until they gain an explicit,
  reviewed projector; generic guessing would recreate the raw-value leak.

## Migration Plan

- No persisted or runtime-event schema migration is required.
- Existing saved sessions gain safe built-in summaries when rendered by the new
  client because projection is derived from canonical history.
- Older or malformed calls continue to render the protected-key fallback.

## Open Questions

- None. Showing command text, search patterns, or opt-in tool results can be
  proposed separately if a stronger redaction and disclosure policy is wanted.
