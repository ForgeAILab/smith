# Setup: providers, credentials, image generation

## First run

The user runs `smith setup`. It is interactive: it picks a provider, takes
the credential with masked input, writes `~/.smith/config.toml`, and makes
a build profile with `-plan` and `-review` variants. You cannot drive the
prompts, so tell the user to run it.

## Add a provider

| Want | The user runs |
| --- | --- |
| ChatGPT subscription | `/connect chatgpt` |
| xAI browser login | `/connect xai` |
| OpenRouter, Google AI Studio | `/connect openrouter`, `/connect google` |
| Any other endpoint | `smith setup add-provider` |
| A model on a custom endpoint | `smith setup add-model --provider <name>` |
| New or rotated key | `smith setup credential --provider <name>` |

`/connect` and `/disconnect` work only while Smith is idle. They keep the
session going.

You may also write the provider table yourself, as long as you use a
credential *reference* and not a secret:

```toml
[providers.remote]
kind = "openai-compatible"
base_url = "https://provider.example/v1"
credential = "env:PROVIDER_API_KEY"
```

References are `env:VAR`, `keychain:service/account`, and
`session-json:<path>#/<field>`. Never put an `api_key` value in a file
yourself, and never put one in a project file. Common `kind` values are
`openai-compatible`, `openai-responses`, `anthropic-messages`, and
`gemini-interactions`. Known endpoints (OpenAI, OpenRouter, xAI, the Z.AI
Coding Plan, Google) take their model limits from the built-in catalog.
Other endpoints need `smith setup add-model` or a `[models]` table.

Verify with `smith config explain model --profile <name>`. It fails with
the reason when a provider or model is incomplete, and it never reads a
credential.

## Installed coding agents

If Claude Code or Codex is installed, its models appear in `/model` as
`cli/claude-code/<alias>` and `cli/codex/<model>`. They need no provider
table.

## Image generation

`generate_image` exists only on the `chatgpt` provider, or on a provider
whose `base_url` is exactly `https://api.openai.com/v1`. It is also left out
in read-only postures. With another main provider, the user can switch
profile or model to one of those for the image work. To check, run
`smith config explain tools.image_generation.enabled`. `/modules` shows why
`image-generation` is inactive. To turn it off:

```toml
[modules.image-generation]
enabled = false
```

Changes apply when the session is rebuilt (`/profile`, `/model`, `/resume`)
or Smith restarts.
