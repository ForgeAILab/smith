# Configuration reference

Smith resolves one typed run configuration before it constructs a provider,
runtime, session, journal, approval channel, or terminal surface. Unknown
fields, wrong types, conflicting values in one layer, incomplete provider/model
selection, unsupported adapter options, unsafe credentials, and missing model
limits are errors. Smith does not guess through them.

## Discovery and precedence

Starting at `--project` or the current directory, Smith walks upward to the
nearest `.smith/` directory. Values are layered in this order, from lowest to
highest precedence:

1. built-in policy defaults;
2. `~/.smith/config.toml`;
3. `<project>/.smith/config.toml`;
4. `<project>/.smith/config.local.toml`;
5. the selected `[profiles.<name>]`;
6. `SMITH_*` environment variables;
7. command-line flags;
8. explicit in-session selection, such as `/model`.

Named provider, model, and profile tables are declared in files. Higher layers
select or override their active settings; they do not invent missing
declarations. Inspect the winner and every overridden source with:

```sh
smith config explain model
smith config explain context.output_reserve --profile work
```

Environment names are the uppercased dotted key with `SMITH_` prepended, for
example `SMITH_CONTEXT_REASONING_RESERVE`,
`SMITH_PERSISTENCE_ENABLED`, and `SMITH_APPROVAL_MODE`. List values such as
`approval.auto_approve` are comma-separated. Environment names are matched
case-insensitively; defining two case variants for one key is an ambiguity and
fails.

Agent-profile selection follows the same provenance rules. One named profile
can configure the main agent, an explicit direct child, or both:

```toml
default_profile = "work"
profile_order = ["work", "plan", "review"]

[profiles.work]
provider = "remote"
model = "vendor/model-id"
description = "implementation with bounded mutation"
posture = "build"
use = ["main"]
instructions = "Implement, verify, and report concrete evidence."

[profiles.plan]
extends = "work"
posture = "plan"
use = ["main", "child"]
description = "inspect and propose without mutation"
instructions = "Produce an implementation-ready plan without editing."

[profiles.review]
extends = "work"
posture = "review"
use = ["main", "child"]
description = "read-only independent review"
instructions = "Report prioritized evidence-backed findings."
```

`use` defaults to `["main"]` when omitted. `extends` names one parent; the
child replaces inherited fields rather than concatenating instructions. Smith
rejects missing parents, cycles, inheritance deeper than 16 profiles, invalid
placements, and child-only entries in `profile_order` before credential or
provider construction. Instructions are nonempty UTF-8 text bounded to 32 KiB
and become an additive developer-instruction fragment after Smith's stable
host policy. Profile text and settings cannot grant a tool, credential, trust,
approval, permission, or larger workspace.

When `profile_order` is omitted, Smith derives a deterministic cycle from all
real main-enabled profiles and excludes legacy adapters and child-only
profiles. Guided setup writes an explicit three-profile order: the selected
build profile plus inherited `-plan` and `-review` variants. Legacy entries
shown by `/profile` route through the deprecated mode override and never become
an invalid `--profile` selection.

For one transition release, `[agent_modes.<name>]` is adapted as a deprecated
main-only profile and `[child_agents.<name>]` as a deprecated child-only
profile. Inventory labels these entries `legacy` with their source; migrate
them to `[profiles.<name>]` with an explicit `posture` and `use`. A legacy and
new declaration claiming the same name is an error—Smith never picks one by
map or file order. Existing `[profiles]` without the new fields remain
main-only build profiles.

## Project instructions and context identity

After configuration resolves the canonical project root, a standard TUI or
headless host examines exactly `<project>/AGENTS.md`. It does not search parent
directories, nested directories, or override filenames in this release. An
absent file contributes nothing. A present file must be a regular,
non-symlinked UTF-8 file no larger than 32 KiB; otherwise startup fails before
provider construction, session state, or terminal entry instead of silently
skipping or truncating it.

Smith reads the file once per constructed runtime. The immutable snapshot is
activated as its own required developer-instruction fragment and inherited by
direct children; children and later turns do not reread the workspace. Editing
the file therefore does not mutate an active context. A user can explicitly
ask the agent to read it again through the ordinary workspace tool path, while
a newly constructed runtime captures the new body automatically.

The fragment records `AGENTS.md` plus a content-derived revision separately
from Smith's built-in prompt revisions, optional retrieval context, and
canonical conversation history. A changed snapshot creates a different exact
prompt/cache-plan identity, while unchanged Smith policy fragments keep their
own revisions. Repository instructions are sent to the selected provider and
may guide work, but cannot grant a tool, permission, approval, credential,
executable trust, or wider workspace. A direct embedder's complete
`system_prompt` override remains a complete replacement and receives no
implicit project-instruction fragment.

## Complete example

```toml
default_profile = "work"
profile_order = ["work", "review"]

[profiles.work]
provider = "remote"
model = "vendor/model-id"
description = "implementation profile"
posture = "build"
use = ["main", "child"]
instructions = "Implement the requested change and verify it."
max_output_tokens = 4096

[profiles.review]
extends = "work"
description = "read-only independent review"
posture = "review"
use = ["main", "child"]
instructions = "Report prioritized findings with concrete evidence."

[profiles.work.reasoning]
enabled = true
effort = "high"

[profiles.work.context]
output_reserve = 4096
reasoning_reserve = 0
capability_budget = 12000
max_estimated_slack = 256
compaction_high_watermark_percent = 85
compaction_low_watermark_percent = 60
idle_compaction_ms = 3600000

[profiles.work.limits]
max_retries = 2
max_tool_steps = 64
turn_time_limit_ms = 600000
tool_output_limit_bytes = 65536

[profiles.work.approval]
mode = "ask"

[profiles.work.background]
exit_policy = "error"
max_children = 4
max_monitors = 8

[providers.remote]
kind = "openai-compatible"
base_url = "https://provider.example/v1"
credential = "env:PROVIDER_API_KEY"

[providers.remote.response]
reasoning_only = "reasoning"

[providers.remote.headers]
x-client-name = "smith"

[models."remote/vendor/model-id"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096

[models."remote/vendor/model-id".reasoning]
toggle = true
mandatory = false
efforts = ["none", "low", "medium", "high"]
default_enabled = true
default_effort = "medium"
dialect = "openai-effort"

[persistence]
enabled = true
sessions_dir = "/absolute/user-owned/state/sessions"
journal_events = true
checkpoint_key_credential = "env:SMITH_CHECKPOINT_SECRET"
```

Policy tables may be top-level or profile-scoped as shown. Providers and models
are shared declarations and cannot be nested under a profile. Every TOML
structure rejects unknown fields.

## Providers, credentials, and models

`providers.<name>.kind` is `openai-compatible` for the production adapter or
`fake` for deterministic local development. `base_url` is required only where
the adapter requires it. Extra headers are sent unchanged, but authorization
header names such as `Authorization`, `X-API-Key`, and `Api-Key` are refused;
credentials belong in the credential broker.

Choose exactly one credential source:

- `credential = "keychain:service/account"` uses the operating-system
  credential service;
- `credential = "env:VARIABLE"` reads a process environment variable;
- `credential = "file:..."` is reserved for an externally unlocked encrypted
  backend and is not treated as plaintext;
- `api_key = "..."` is accepted only in an owner-only, regular, non-symlinked
  user config and is mutually exclusive with `credential`.

Project and project-local files cannot contain `api_key`. Setup masks secret
input and reviews only `[redacted]` before an owner-only atomic write.

Checkpoint protection has explicit, mutually exclusive sources:

- the default OS credential service;
- `SMITH_CHECKPOINT_KEY`, containing exactly 64 hexadecimal characters;
- owner-only `[persistence].checkpoint_key`, containing the same encoding; or
- `[persistence].checkpoint_key_credential = "env:VARIABLE"`, which resolves
  that variable without opening a credential service.

Checkpoint-key settings are forbidden in project configuration. All
explanation/debug surfaces show only `[redacted]` and source provenance. Run
`smith setup checkpoint-key` to choose `Store in config (no future prompts)`,
an environment reference, or OS protected storage. Inline/environment
selection never initializes Keychain or Secret Service. If protected
checkpoints already exist under the selected session root, changing sources
refuses before modifying config; retire or resume that state deliberately
before choosing another key.

The selected `provider/model` must resolve exact `context_tokens`,
`max_input_tokens`, and `max_output_tokens` from configured values, trusted
embedded metadata, or a validated endpoint-bound catalog. Explicit fields win
independently. `profiles.<name>.max_output_tokens` is the per-request ask and
cannot exceed the model's resolved ceiling.

For the endpoint-bound Z.AI Coding Plan `glm-5.2` catalog entry, Smith keeps
the provider model ceiling (`131072`) separate from its product request budget
and defaults the latter to `32768`. An owner-controlled profile, environment,
CLI, or session source may request a lower value. `smith config explain
max_output_tokens --profile zai-glm-5-2` identifies the winning value and every
overridden source.

`providers.<name>.response.reasoning_only = "text"` is a narrow compatibility
mode for a provider that returns a successful answer solely as non-redacted
reasoning. The omitted/default value, `"reasoning"`, preserves the provider's
classification.

Reasoning presence is separate from reasoning control. A catalog
`reasoning = true` value means only that reasoning is present and fixed; it
does not create a toggle or effort selector. Rich
`[models."provider/model".reasoning]` metadata must name the exact wire
dialect (`openai-effort`, `openrouter`, or `zai-thinking`), switch behavior,
and ordered effort values. Unknown OpenAI-compatible endpoints expose no
inferred controls.

`[reasoning]` and `[profiles.<name>.reasoning]` select `enabled` and/or one
advertised `effort`. Omission sends no reasoning option and preserves provider
behavior. The exact Z.AI Coding Plan binding exposes its documented thinking
toggle but no general effort ladder. `/think [on|off|default]` and
`/effort [LEVEL|default]` apply session overrides at an idle boundary; omitted
arguments open local bounded selectors and make no provider request. Invalid
or mandatory/fixed choices fail before credential lookup. Higher effort can
increase latency, token use, and cost.

## Policy keys and defaults

Smith's defaults claim no provider, model, model limit, model-dependent output
reserve, capability budget, or estimated-count slack.

| Key | Built-in default | Meaning |
| --- | ---: | --- |
| `context.reasoning_reserve` | `0` | Continuation/reasoning reserve |
| `context.compaction_high_watermark_percent` | `85` | Pressure trigger |
| `context.compaction_low_watermark_percent` | `60` | Post-compaction target |
| `context.idle_compaction_ms` | `3600000` | Idle compaction interval |
| `limits.max_retries` | `2` | Retries after the first provider attempt |
| `limits.max_tool_steps` | `64` | Tool-loop ceiling per turn |
| `limits.turn_time_limit_ms` | `600000` | Whole-turn deadline |
| `limits.tool_output_limit_bytes` | `65536` | Inline output/offload threshold |
| `persistence.enabled` | `true` | Save project-partitioned sessions |
| `persistence.sessions_dir` | `~/.smith/sessions` | User-owned state root |
| `persistence.journal_events` | `true` | Write redacted JSONL events |
| `approval.mode` | `"ask"` | Fail closed until a decision exists |
| `background.exit_policy` | `"error"` | Refuse to orphan active work |
| `background.max_children` | `4` | Concurrent child capacity |
| `reasoning.enabled` | provider/model default | Explicit thinking state when supported |
| `reasoning.effort` | provider/model default | Exact advertised effort when supported |
| `background.max_monitors` | `8` | Reserved process-monitor capacity |

The compaction low watermark must be below the high watermark. Reserves must
leave a positive input budget. Numeric limits are validated before any provider
request.

Approval modes are `"ask"`, `"deny"`, and `"allow-all"`. Background exit modes
are `"error"`, `"wait"`, and `"stop"`. `approval.auto_approve` is a list of
tool names; it is still bounded by preparation, central authorization, and the
workspace.

## Command-line selection

The run surface accepts:

```text
--project PATH
--profile NAME
--agent MODE                  # deprecated legacy mode compatibility
--provider NAME
--model ID
--approval ask|deny|allow-all
--yolo                       # explicit alias for --approval allow-all
--background-exit error|wait|stop
--resume [SESSION_ID]
--output-format text|json|stream-json
```

An omitted `--resume` ID opens a local picker only in an interactive terminal.
Headless or piped use must pass the exact ID. `--output-format` requires `-p`.

Project-controlled configuration cannot grant `allow-all`, populate
`auto_approve`, disable/redirect persistence, or choose the user session root.
Those settings require a higher-trust source. Opening a repository is never
authority. `--yolo` is only a shorter explicit spelling of
`--approval allow-all`; it does not add tools or override a profile's
read-only posture.
