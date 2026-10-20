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
