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

Root-agent selection follows the same provenance rules. Smith provides
`build`, `plan`, and `review`, ordered that way by default, plus read-only
`explore` and `review` child presets. Owner-controlled configuration may
select/reorder or describe modes:

```toml
default_agent = "plan"
agent_order = ["plan", "build", "review"]

[agent_modes.plan]
posture = "plan"
description = "inspect and propose without mutation"

[child_agents.review]
posture = "review"
description = "read-only independent review"
```

Mode and preset declarations contain no permission, credential, trust, or
approval fields. Project definitions are validated and intersected with the
authoritative run policy; they cannot grant authority.

## Complete example

```toml
default_profile = "work"

[profiles.work]
provider = "remote"
model = "vendor/model-id"
max_output_tokens = 4096

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
--agent build|plan|review
--provider NAME
--model ID
--approval ask|deny|allow-all
--background-exit error|wait|stop
--resume [SESSION_ID]
--output-format text|json|stream-json
```

An omitted `--resume` ID opens a local picker only in an interactive terminal.
Headless or piped use must pass the exact ID. `--output-format` requires `-p`.

Project-controlled configuration cannot grant `allow-all`, populate
`auto_approve`, disable/redirect persistence, or choose the user session root.
Those settings require a higher-trust source. Opening a repository is never
authority.
