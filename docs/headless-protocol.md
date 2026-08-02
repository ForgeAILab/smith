# Headless protocol

`smith -p` uses the same resolved runtime, session, tools, authorization,
checkpoints, and persistence as the TUI. Ordinary prompts retain one-turn
behavior. If that turn explicitly creates an active persistent goal, the host
remains subscribed through attributed internal continuations until the goal
stops.

## Input and output modes

```sh
smith -p "inspect this repository"
printf '%s' "run the focused tests" | smith -p -
smith -p "summarize" --output-format json
smith -p "inspect" --output-format stream-json
smith -p "continue" --resume session-...
```

Prompt stdin is read once, as bounded UTF-8, before the turn. It is never used
as an asynchronous approval or questionnaire channel.

| Format | stdout | stderr |
| --- | --- | --- |
| `text` | Final committed assistant text only | Bounded lifecycle/authority diagnostics |
| `json` | Exactly one schema-v3 result plus newline | Empty after successful protocol startup |
| `stream-json` | Schema-v3 runtime-event lines, then one terminal result line | Empty after successful protocol startup |

Machine stdout has no ANSI sequences, setup UI, prompt text, progress prose, or
unversioned diagnostics. CLI parsing errors exit 2; startup/protocol failures
that cannot produce a result exit 1.

## Result schema v3

Schema 3 retains existing field meanings and adds optional goal fields:

```json
{
  "schema_version": 3,
  "type": "result",
  "status": "ok",
  "session_id": "session-...",
  "turn_id": "turn-...",
  "provider": "configured-name",
  "model": "model-id",
  "output": "committed assistant answer",
  "usage": {
    "current_turn": {},
    "session": {},
    "current_turn_provenance": "provider_reported",
    "session_provenance": "provider_reported"
  },
  "lifecycle": {
    "attempts_committed": 1,
    "attempts_discarded": 0,
    "activation": {
      "epoch": 1,
      "capabilities": ["tool:read", "tool:search"]
    },
    "children": [
      {
        "child_id": "child-1",
        "child_session_id": "session-child-1",
        "durability": "durable",
        "state": "idle",
        "resumable": false,
        "turns_used": 2,
        "max_turns": 4,
        "tokens_used": 1200
      }
    ]
  },
  "goal": {
    "id": "goal-...",
    "generation": 4,
    "objective": "ship the requested change",
    "status": "complete",
    "token_budget": 20000,
    "usage": {
      "charged_tokens": 18342,
      "provenance": "provider_reported",
      "active_elapsed_ms": 9123
    },
    "created_at": 0,
    "updated_at": 0
  },
  "goal_continuation_turns": 3
}
```

`usage` uses `unknown`, never a fabricated zero, when no provider observation
exists. `output` is selected only from assistant history created after this
turn was accepted; an older answer cannot be reused for a reasoning-only or
failed turn.

`lifecycle.plan`, when present, contains:

```json
{
  "revision": 4,
  "sensitivity": "public",
  "counts": {
    "pending": 1,
    "in_progress": 1,
    "completed": 2,
    "cancelled": 0
  },
  "items": []
}
```

Item text is emitted only for `public`. A sensitive projection is counts-only
even if a corrupt event attempts to attach items.

`artifacts` is omitted when empty. Each value is Agent Runtime's typed
`ArtifactRef`: opaque ID, SHA-256 digest, media type, byte length, sensitivity,
retention, and producing provenance. The reference does not grant another
session read authority.

`lifecycle.children` is omitted when empty. It is a metadata-only snapshot of
the parent coordinator at the result boundary: stable child/session IDs,
durability, lifecycle state, exact-resume availability, cumulative turn/token
usage, and a bounded incompatibility reason when blocked. It never includes a
task, child history, raw result, tool arguments, or checkpoint content.

`goal` and `goal_continuation_turns` are omitted for ordinary runs. Goal usage
charges provider-reported uncached input plus output; cached input is excluded.
Unknown evidence remains `unknown`, never zero. A token budget is an observed
post-response limit, so reported usage may overshoot it by one request.

`recovery` is metadata-only:

```json
{
  "reason": "process_exit",
  "interrupted_children": ["child-1"],
  "interrupted_monitors": ["monitor-1"]
}
```

It reports prior process-owned work and never claims that it restarted.

## Stream envelopes

Every nonterminal line is:

```json
{
  "schema_version": 3,
  "type": "runtime_event",
  "event": {
    "schema_version": 10,
    "seq": 12,
    "id": "event-...",
    "session": "session-...",
    "turn": "turn-...",
    "timestamp": 0,
    "payload": {
      "event": "turn_started"
    }
  }
}
```

Runtime events keep their independent event schema. Smith preserves canonical
sequence order, includes `session_shutdown`, and writes the result last. A
sequence gap turns the result into a failure rather than presenting incomplete
stream output as complete.

Text/reasoning events are attempt-scoped. Failed partial output may appear in
stream events for observability, followed by
`provider_attempt_output_discarded`; only committed output enters the terminal
result and canonical transcript. Attempt usage remains accounted.

Tool events contain sorted argument key names and a fingerprint by default,
not argument values. Prepared approval details travel in the result below, not
as raw runtime arguments.

## Non-success results

| `status` | Exit | Additional field | Meaning |
| --- | ---: | --- | --- |
| `ok` | 0 | — | Turn completed |
| `approval_required` | 4 | `approval_required` | Exact action lacked unattended authority |
| `interaction_required` | 5 | `interaction_required` | Task input must come from an interactive host |
| `failed` | 1 | optional `error` | Turn or lifecycle failed |
| `cancelled` | 1 | — | Turn was cancelled |
| `limit_reached` | 1 | — | A configured limit stopped the turn |

An approval-required payload contains only redaction-safe prepared evidence:

```json
{
  "call_id": "call-...",
  "tool": "edit",
  "argument_keys": ["new_string", "old_string", "path"],
  "mutates": true,
  "requires_authorization": true,
  "permissions": ["fs.read", "fs.write"],
  "resource": {
    "resource_kind": "filesystem",
    "mount": "/canonical/project",
    "segments": ["src", "lib.rs"]
  },
  "authority_warnings": [],
  "deadline_at_ms": 1750000000000,
  "preparation_fingerprint": "..."
}
```

Argument values, edit bodies, commands, credentials, and tool output are not
included. The call is denied/fails closed unless the process was launched with
an explicit trusted automation policy such as `--approval allow-all`.

An interaction-required payload contains `request_id` and `question_count`
only. Prompt text, choices, staged answers, and sensitive content remain in the
protected checkpoint. A restored pending request is returned without accepting
a new prompt turn or advancing that checkpoint.

## Compatibility

Consumers must dispatch on `schema_version` and `type`, tolerate additive
fields inside a supported schema, and treat unknown status/resource/event
variants as unsupported rather than guessing. Schema-v1 fixtures remain only
as migration evidence; new integrations should implement schema 2.
