# Models, effort, thinking, and context windows

## Switch for this session

The user types these. Each one applies at the next idle boundary, keeps the
conversation, and makes no provider request until the next turn.

- `/model` opens the model picker. `/model <provider>` narrows it to one
  provider.
- `/provider <name>` switches provider.
- `/profile <name>` switches to a configured profile (see
  `smith.guide.profiles-and-advisor`).
- `/effort <level>` sets reasoning effort; `/effort default` clears it.
  `/effort` alone opens a picker of the levels this model offers.
- `/think on`, `/think off`, or `/think default` toggles thinking where the
  model has a switch.
- `/context <name>` picks a named context window; `/context default`
  clears it. `/context` alone shows usage and lists the windows.

## Effort levels are per model

Levels are the model's own, taken from the model catalog or from
`[models."<provider>/<model>".reasoning]`. Do not assume a fixed set.
`smith --effort <level>` refuses an unknown level and lists the valid ones.
That makes it a safe way to discover the levels.

Some models always think, so `/think off` is refused for them. This applies
to Z.AI GLM-5.x models, native Gemini models with a thinking ladder, and
Anthropic routes declared as mandatory. Use `/effort` on those instead.

## Make it the default

For one run: `smith --effort high`, `smith --model <id>`, or
`smith --context-window <name>`.

To change the default for good, edit the profile in `~/.smith/config.toml`
(you may do this through your edit tool, with approval):

```toml
[profiles.code.reasoning]
effort = "high"

[profiles.code]
context_window = "128k"
```

The order of precedence, from lowest to highest: config files, `SMITH_*`
environment variables, command-line flags, then in-session `/effort` or
`/model`. Check the winner with `smith config explain reasoning.effort` or
`smith config explain context_window`.

## A model is missing or its limits are wrong

- `smith models sync --force` refreshes catalog metadata now.
- The user runs `smith setup add-model --provider <name>` to add a model
  for a custom endpoint. It offers the IDs the endpoint lists.
- A `[models."<provider>/<model>"]` table overrides catalog limits or input
  modalities. Only declare one when the catalog is wrong or does not list
  the model.

Higher effort costs more tokens and time. Say so when you suggest it.
