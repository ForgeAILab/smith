# Scripting Smith with `smith -p`

`smith -p` runs one turn with the same config, tools, approvals, and
persistence as the TUI, then exits.

```sh
smith -p "summarize the open TODOs"
printf '%s' "run the focused tests" | smith -p -
smith -p "compare these" --image ./before.png --image ./after.png
smith -p "continue" --resume <session-id>
smith -p "inspect" --profile code-plan --effort high
```

## Output

| `--output-format` | stdout |
| --- | --- |
| `text` (default) | The final answer only; diagnostics go to stderr |
| `json` | One schema-v3 result object |
| `stream-json` | Event lines, then one result line |

Read `status`, `output`, `session_id`, and `usage` from the JSON result.
See `smith.headless` for the full schema.

## Exit codes

| Exit | `status` | Meaning |
| ---: | --- | --- |
| 0 | `ok` | Turn completed |
| 4 | `approval_required` | A tool needed approval nobody could give |
| 5 | `interaction_required` | The agent asked a question; resume interactively |
| 1 | `failed`, `cancelled`, `limit_reached` | See `error` or stderr |
| 2 | — | Bad command-line arguments |

## Approvals

No one can answer a prompt during a headless run, so actions that need
approval fail closed with exit 4. Choose one:

- `--approval deny` refuses those actions, and the agent continues without
  them.
- `--profile <read-only profile>` uses a `plan` or `review` posture.
- `--approval allow-all` (or `--yolo`) approves everything. Use it only in
  an isolated sandbox or CI job. It does not widen a read-only posture.
- `[[approval.auto]]` rules in the user config approve exact edit paths or
  fetch hosts (see `smith.configuration`).

## Tips

- Pass a session ID to `--resume` to continue the same conversation across
  runs. Get IDs from the JSON `session_id` or `smith sessions list`.
- `--image` needs `-p` and does not work with `cli/*` models.
- A prompt that starts with `/<name>` expands a file command. Start it with
  `//` to send a literal slash.
- Ctrl-C or `SIGTERM` cancels the turn, saves the session, and exits 1.
