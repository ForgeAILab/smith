# Profiles and the advisor

A profile is a named preset: provider, model, posture, reasoning, limits,
and instructions. Profiles live in `[profiles.<name>]` tables in
`~/.smith/config.toml` (or a project's `.smith/config.toml`).

## Inspect

- `smith config explain profile` shows which profile is active and why.
- The user types `/profile` to list profiles and pick one, or presses Tab
  in an empty composer to cycle through `profile_order`.

## Add a profile

```toml
default_profile = "code"
profile_order = ["code", "code-plan"]

[profiles.code]
provider = "zai"
model = "glm-5.3"
description = "implementation"
posture = "build"

[profiles.code-plan]
extends = "code"
posture = "plan"
use = ["main", "child"]
description = "inspect and propose without editing"
```

- `posture` is `build`, `plan`, or `review`. `plan` and `review` are
  read-only.
- `use` is `["main"]` by default. Add `"child"` so the agent can delegate
  to the profile as a child agent.
- `extends` inherits every unset field from one parent profile.
- `instructions` adds text to the system prompt. It cannot grant tools or
  permissions.
- `delegation = false` removes the `agent` tool from a main profile.

Smith rejects unknown profiles, cycles, and undeclared providers when the
config loads. Run `smith config explain profile --profile <name>` after an
edit to catch mistakes before the user switches.

## Model fallback

Keep `provider` and `model` as the primary, then add ordered alternatives:

```toml
[profiles.code.fallback]
first_response_timeout_ms = 30000
models = [
  { provider = "chatgpt", model = "gpt-6.1-sol" },
  { provider = "anthropic", model = "claude-sonnet-4-6", first_response_timeout_ms = 60000 },
]
```

Providers must already be declared. A switch sends the conversation context
to the selected provider. Instructions, posture, permissions, and completed
tool results stay with the profile.

The timeout covers waiting for a response to start, including connection,
retries, and backoff. Local setup and account-rotation prompts finish before
this timer starts. Text, reasoning, or tool activity stops it. Smith does
not switch because an answer stalls after it starts. Omit the timeout to use
failure fallback alone; its triggers are quota exhaustion, unavailable local
credential sources, and exhausted transient failures. Authentication rejection
and invalid requests remain errors.

Each model is tried in order at most once per turn. A successful fallback
serves the rest of that turn; the next user turn starts with the primary.
An unfinished turn saved by an older version finishes with its original
execution policy; new turns can use fallback.
Entries can override `reasoning`, `max_output_tokens`, and `context_window`.
An explicit `reasoning` table replaces inherited profile reasoning controls;
`reasoning = {}` uses the model's defaults. Invocation overrides still win.
Unsupported explicit controls fail preflight. A smaller model is skipped if
the pending request cannot fit without losing protected history.

The list inherits through `extends`; a new `models` list replaces it, and
`models = []` disables fallback and its timer. An explicit `/model`,
`--model`, or `--provider` selection disables the chain until the profile is
selected again. Installed `cli/...` agents do not support this policy.
Inspect with `smith config explain fallback.models --profile code` and
`smith config explain fallback.first_response_timeout_ms --profile code`.

## The advisor

The advisor is a stronger model the main agent can consult. Set it at the
top level for every profile, or per profile:

```toml
advisor = "sol"              # a profile name

[profiles.sol]
advisor = false              # no advisor while sol is the main profile
```

The value can also be `"<provider>/<model>"`. The advisor receives the
conversation, including tool output, so it is disclosed to the advisor's
provider. Tell the user this before you enable one on a different provider.
The user can type `/advisor off`, or `/advisor` to choose, for the current
session. Check with `smith config explain advisor`.

## Vision follows the same pattern

The `vision` key takes the same values as `advisor`; see
`smith.guide.vision`.

Profile changes apply when the session is rebuilt (`/profile`, `/model`,
`/resume`) or Smith restarts.
