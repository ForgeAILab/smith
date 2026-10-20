# Driving Smith: guide index

Use these guides when the user asks how to set up, configure, or operate
Smith itself. Activate the one guide that matches the task.

| Task | Guide |
| --- | --- |
| First run, providers, credentials, image generation | `smith.guide.setup` |
| Switch model, effort, thinking, context window | `smith.guide.models-and-thinking` |
| Let a text-only model see images | `smith.guide.vision` |
| Profiles, `profile_order`, the advisor | `smith.guide.profiles-and-advisor` |
| Resume, new session, undo, goals, child agents | `smith.guide.sessions` |
| Skills, file commands, plugins, MCP servers, modules | `smith.guide.extending` |
| Scripting `smith -p` | `smith.guide.headless` |
| Something is wrong or a setting has no effect | `smith.guide.troubleshooting` |

## Who does what

- **The user** types slash commands (`/model`, `/profile`, `/connect`) and
  answers pickers and approval prompts. You cannot type them. Tell the user
  the exact command to type.
- **You** may run read-only commands such as `smith config explain <key>`,
  `smith config modules`, `smith sessions list` and `smith plugin list`
  through your shell tool, and edit `~/.smith/config.toml` or
  `.smith/config.toml` with your edit tool. Edits go through normal approval.
- **Never** read, write, or repeat a credential. The user enters secrets
  through `smith setup credential` or `/connect`.

## Config changes take effect on rebuild

Smith reads its config files when a session is built. After you edit one,
the change applies once the session is rebuilt: the user runs `/profile` or
`/model` (the conversation is kept), `/resume`, or restarts Smith. Check the
result with `smith config explain <key>`.
