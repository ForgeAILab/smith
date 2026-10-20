# Troubleshooting Smith

Start with evidence, not guesses. These commands are read-only and safe for
you to run:

- `smith config explain <key>` prints the winning value and its source
  file and key, or says no layer set it. Add `--profile <name>` to check a
  profile that is not active. Useful keys: `profile`, `model`, `vision`,
  `advisor`, `reasoning.effort`, `context_window`,
  `tools.image_generation.enabled`.
- `smith config modules` shows each module as mounted or inactive, with the
  reason.
- `smith plugin list` shows each plugin's state.
- `smith sessions list` lists the sessions for this project.

The user can type these inside the TUI: `/status`, `/modules`, `/mcp`,
`/skills`, `/capabilities`, `/diagnostics`, and `/account`.

## Common problems

| Symptom | Check and fix |
| --- | --- |
| A config edit had no effect | A higher layer may override it: `smith config explain <key>`. The session also needs a rebuild (`/profile`, `/model`, `/resume`, or a restart). |
| Startup fails, naming a key | Unknown field, wrong type, or a missing declaration. Every table rejects unknown keys, so fix the key the error names. |
| "Not logged in" or 401 | The credential expired. The user runs `/connect <provider>` or `smith setup credential --provider <name>`. |
| `--effort` or `/think off` refused | The model does not offer that level, or it always thinks; the error lists valid levels. See `smith.guide.models-and-thinking`. |
| The agent cannot see an image | The main model is text-only and `vision` is unset. See `smith.guide.vision`. |
| No `generate_image` tool | The provider is not ChatGPT or the OpenAI Platform, or the posture is read-only. See `smith.guide.setup`. |
| An MCP server's tools are missing | The server is not trusted yet (`/mcp trust <name>`), or it failed; `/mcp` shows its stderr. |
| A project skill or command is ignored | It is untrusted or changed. Trust it with `/skills trust <name>` or `/commands trust <name>`. |
| Headless exits 4 | A tool needed approval. See `smith.guide.headless`. |
| Unexpected cache costs | Check `/status` (the `cache:` block) and `/diagnostics`. |

## Logs

`SMITH_LOG=info smith` (or `debug`) writes a session log. Logs leave out
secrets, but they can still contain prompts and file contents, so ask
before sharing one.

Session journals and checkpoints sit under `~/.smith/sessions`. Never delete
them to get past an error; see `smith.persistence`.
