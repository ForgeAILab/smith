---
name: smith-cli-runtime
description: How to invoke the local `smith` CLI headlessly and read its result envelope. Load before dispatching work to Smith from Claude Code.
---

# Driving `smith -p`

One dispatch is one `Bash` call. Smith owns the whole run — its own provider,
model, tools, approval policy, and session — and returns a single JSON envelope.

```bash
smith -p "<task>" --output-format json --approval deny
```

## Flags that matter here

| Flag | Use |
| --- | --- |
| `-p <PROMPT\|->` | The task. `-` reads it from stdin, which is the right form for anything containing quotes or newlines. |
| `--output-format json` | Exactly one result object on stdout. Always pass it; the default `text` gives you prose with no session id. |
| `--approval deny` | Read-only dispatch. Every mutating tool fails closed at its approval boundary. |
| `--approval allow-all` | Write dispatch. Smith may edit files and run commands unattended. |
| `--resume <SESSION_ID>` | Continue a previous Smith session with its history intact. |
| `--profile <NAME>` | Select a configured agent profile. A profile with a read-only posture never registers `edit` or `shell` at all, which is stronger than denying them. |
| `--model` / `--provider` | Override the configured selection. Leave unset unless asked. |
| `--project <PATH>` | Start project discovery somewhere other than the working directory. |

Pass long or quote-heavy tasks through stdin rather than fighting shell quoting:

```bash
printf '%s' "$TASK" | smith -p - --output-format json --approval deny
```

## Reading the result

Stdout is one schema-v3 object. Stderr carries diagnostics and is not part of
the contract.

```json
{
  "schema_version": 3,
  "type": "result",
  "status": "ok",
  "session_id": "session-...",
  "model": "...",
  "output": "the committed assistant answer",
  "usage": { "session": {}, "session_provenance": "provider_reported" }
}
```

- `status: "ok"` — `output` is the answer. Report it verbatim.
- `status: "approval_required"` — the run *stopped* at a mutating tool with no
  one to ask. This is the `--approval ask` case, not the `deny` case.
- `status: "error"` — report the message. Do not retry with a broader approval
  policy to make an error go away.

Under `--approval deny` a refused write does **not** stop the run and does not
change the status. The tool call is declined, Smith carries on, and it reports
the refusal inside `output` — status stays `ok` and the exit code stays `0`.
So a read-only dispatch that needed to write looks like a successful run whose
answer explains what it could not do. Read the output; do not infer from the
status that everything was applied.

Exit codes: `2` is a CLI parse error, `1` is a startup or protocol failure that
produced no result. Both mean nothing ran.

## Session continuity

`session_id` is the handle for follow-up work. Passing it to `--resume` gives
Smith its prior conversation, which is much cheaper and more accurate than
restating context in a fresh prompt. Prefer resuming over re-describing.

`smith sessions list` prints one tab-separated row per session for this project:
id, last-updated, turn count, model, and the opening prompt.

## What not to do

- Do not read files, grep, or investigate the repository yourself in order to
  build the prompt. Smith has its own tools and its own view of the workspace;
  duplicating that work costs tokens twice and can hand it a stale picture.
- Do not escalate `--approval deny` to `allow-all` on your own initiative. A
  denied action is a real constraint and the user chose the default.
- Do not paraphrase Smith's output. Its answer is the deliverable.
