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

### Directories beside the configuration

Two locations are read by their fixed paths rather than through the layered
settings, because a setting that relocates them would be a setting that
relocates what Smith trusts:

| Path | Holds |
| --- | --- |
| `~/.smith/skills/<name>/SKILL.md` | User skills, available in every project |
| `<project>/.smith/skills/<name>/SKILL.md` | Project skills, gated by project trust |

Smith creates neither directory. See [Skills](skills.md) for the file format,
the bounds, and how a project skill comes to be trusted.

Environment names are the uppercased dotted key with `SMITH_` prepended, for
example `SMITH_CONTEXT_REASONING_RESERVE`,
`SMITH_PERSISTENCE_ENABLED`, and `SMITH_APPROVAL_MODE`. List values such as
`agent_order` are comma-separated. Structured `[[approval.auto]]` rules are
documented in TOML rather than encoded as a comma-separated tool-name list.
Environment names are matched case-insensitively; defining two case variants
for one key is an ambiguity and fails.

## Prompt-cache miss notices

Local cache-miss notices are controlled by the explainable layered Boolean
`cache.miss_notices`. It defaults to `false`, so cache state and machine output
remain available without adding transcript or stderr notices. Enable it in a
file or for one run with the matching environment variable:

```toml
[cache]
miss_notices = true
```

```sh
SMITH_CACHE_MISS_NOTICES=true smith -p "inspect this repository"
```

The setting changes only local human-facing presentation. It does not change
provider requests, canonical cache events, usage accounting, or retention.
Smith emits at most one notice for a completed root turn when canonical
re-billed tokens reach 20,000 or known derived extra cost reaches $0.10. Any
idle duration in that notice is factual start-to-start context; it does not
claim that a cache entry expired or became unavailable. In particular,
`after Nm idle` is correlation, not an expiry diagnosis. Smith has no generic
pre-request cache-alive probe: it classifies cache state only after the
provider returns usage/cache evidence for an attempt.

## Adaptive cache lifecycle and idle compaction

Cache mechanism policy is profile-scoped. The built-in mode is `off`; Smith
still records provider cache plans and evidence, but sends no synthetic cache
request. `observe` exposes the same lifecycle without dispatch. `adaptive`
requests bounded keepalive or handoff work, but becomes effective only when the
selected model/adapter has passed the matching conformance fixture and the
trusted host was started with `--allow-synthetic-cache-spend`:

```toml
[profiles.work.context.cache]
maintenance = "adaptive"             # off | observe | adaptive
inactivity_limit_ms = 3600000
max_hold_while_child_ms = 3600000
max_maintenance_calls = 1
max_maintenance_input_tokens = 0      # use the exact plan/model input budget
max_maintenance_output_tokens = 256
maintenance_deadline_ms = 30000
keepalive_margin_ms = 120000
keepalive_jitter_percent = 10
handoff_checkpoint = true
idle_compaction = true
resume_capsule = true

[profiles.work.child_agents]
wait_default_timeout_ms = 300000
wait_max_timeout_ms = 300000
```

The authority flag is a host decision, not a TOML or environment setting. A
project can request or narrow adaptive behavior but cannot authorize provider
spend. Without the flag, `adaptive` resolves to `observe` and explain/status
output names the narrowing reason. Unsupported models, custom or unverified
endpoints, rotating credential pools, missing exact identity, exhausted
budgets, and contract violations also fail closed for maintenance while
ordinary provider turns remain available. Cost or price estimates are
presentation-only and never grant dispatch.

The numeric bounds and zero meanings are:

| Key | Default | Accepted values |
| --- | ---: | --- |
| `context.cache.inactivity_limit_ms` | `3600000` | `1000..=86400000`; zero is invalid |
| `context.cache.max_hold_while_child_ms` | `3600000` | `0..=86400000`; zero disables child holding |
| `context.cache.max_maintenance_calls` | `1` | `0..=8`; zero disables synthetic maintenance |
| `context.cache.max_maintenance_input_tokens` | `0` | zero uses the exact resolved plan/model budget; otherwise no greater than the model input limit |
| `context.cache.max_maintenance_output_tokens` | `256` | `1..=4096`, then narrowed to the model output limit |
| `context.cache.maintenance_deadline_ms` | `30000` | `1..=120000` |
| `context.cache.keepalive_margin_ms` | `120000` | `0..=inactivity_limit_ms`; zero removes the early margin |
| `context.cache.keepalive_jitter_percent` | `10` | `0..=50`; zero is deterministic |
| `child_agents.wait_default_timeout_ms` | `300000` | `0..=300000`; zero is an immediate status check |
| `child_agents.wait_max_timeout_ms` | `300000` | `1..=300000`; must be at least the default |

`agent.wait` accepts the optional `timeout_ms`. With no override it waits in the
foreground for up to five minutes, then returns a successful `running` result
with `timed_out = true`; the child is left running in the background and its
terminal result is delivered automatically. A shorter valid timeout can be
requested, zero performs an immediate status check, and a value above the
resolved maximum is rejected before waiting. A parent is
`parked-awaiting-child` only while a direct child is still nonterminal; no
provider stream or tool call is kept open after the foreground wait boundary.
Terminal child outcomes are preserved and automatically admitted through an
ordinary, attributed continuation at the next safe boundary. Child progress
and child provider/tool work do not reset the parent's inactivity or cache-touch
clocks.

`context.idle_compaction_ms` remains a deprecated one-release alias for
`context.cache.inactivity_limit_ms`. Equal values in one layer are accepted;
different values in the same layer are ambiguous and fail preflight. Across
layers, normal precedence selects the winner and `config explain` retains the
source. Use the canonical key for new configuration.

At the inactivity limit Smith persists exact state and may attempt semantic
compaction once for that real-turn interval. This is ordinary summary work,
not a cache keepalive, and it has its own `cache_idle_compaction` usage
purpose. Failure keeps the original canonical history and is not retried.
Keepalive and same-model handoff use `cache_keepalive` and
`cache_handoff_checkpoint`; all provider-reported usage counts toward provider
and session limits while remaining separate from user, parent, and child turn
usage.

Status and machine output retain a bounded per-attempt projection for this
work: typed purpose, provider and model, optional exact cache identity,
disjoint usage counters with provider-reported provenance, cost with explicit
provenance, latency, and bounded completion status. A missing provider bill is
reported as unknown rather than derived from an unrelated price table; exact
same-model presentation may calculate cost only when compatible catalog rates
are available.

Cache status separates structural eligibility from provider evidence:
`eligible` does not mean warm, an explicit zero read can prove a miss, omitted
evidence remains unknown, and elapsed time never proves expiry. A provider's
typed expiry/resource evidence can suspend work; Smith does not infer it from
an idle duration. Identity changes retire the old lease instead of transferring
warmth.

When enabled, the versioned resume capsule is stored inside the existing
canonical snapshot and protected checkpoint rather than a sidecar database.
The highest compatible committed watermark wins, with protected exact state
ahead of a same-boundary projection. Generated summary text cannot override
exact state or schedule work. After a process restart live child work is
reconciled as interrupted, provider warmth is unknown, and Smith sends no
prewarm request; the next real continuation creates cache naturally. Human and
machine projections contain only bounded, redaction-safe capsule metadata, not
summary text, prompts, credentials, private instructions, or cache contents.

Interactive `/status` renders one canonical `cache:` block containing the
latest completed root-turn state, `CH`, expected/observed/missed tokens,
confidence, cumulative miss count, re-billed tokens, and known derived extra
cost. Its separate `cache read (session)` line is retained as the cumulative
raw provider-read total for compatibility; it is not the latest-turn `CH`.

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
delegation = false
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

`delegation` defaults to `true` and is inherited like the other profile fields.
Setting it to `false` on a main profile removes both the model-facing `agent`
tool and its delegation instructions. Child surfaces never delegate, even when
their effective profile has `delegation = true`.

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
absent file contributes nothing. A present file must resolve to a regular UTF-8
file no larger than 32 KiB inside the canonical project root; a symlink such as
`AGENTS.md -> CLAUDE.md` is followed, one leaving the project is not. Otherwise
startup fails before provider construction, session state, or terminal entry
instead of silently skipping or truncating it.

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

[profiles.work.context.cache]
maintenance = "observe"
inactivity_limit_ms = 3600000
max_hold_while_child_ms = 3600000
max_maintenance_calls = 1
max_maintenance_input_tokens = 0
max_maintenance_output_tokens = 256
maintenance_deadline_ms = 30000
keepalive_margin_ms = 120000
keepalive_jitter_percent = 10
handoff_checkpoint = true
idle_compaction = true
resume_capsule = true

[profiles.work.child_agents]
wait_default_timeout_ms = 300000
wait_max_timeout_ms = 300000

[profiles.work.limits]
max_retries = 2
max_tool_steps = 0
turn_time_limit_ms = 0
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

`providers.<name>.kind` selects the wire protocol:

| kind | protocol | `base_url` |
| --- | --- | --- |
| `openai-compatible` | OpenAI Chat Completions | required |
| `openai-responses` | OpenAI Responses, stateless | required |
| `anthropic-messages` | Anthropic Messages | optional, defaults to the public endpoint |
| `chatgpt-responses` | ChatGPT Codex Responses | optional, defaults to the ChatGPT endpoint |
| `xai-responses` | Responses, authenticated by a stored xAI login | required |
| `gemini-interactions` | Google Gemini Interactions, stateless | fixed native endpoint; not configurable |
| `fake` | deterministic local development | not used |

`openai-responses` is generic over the Responses protocol rather than tied to
one vendor, so the endpoint decides which deployment is being talked to. xAI's
Grok is the first fixture-verified one, and `smith setup` offers it as
**Connect xAI Grok**.

Which kind to write depends on how you sign in, because the two differ in what
the credential *is*. `openai-responses` sends its credential verbatim as the
bearer, which is what an API key is. A browser login is not a bearer but a
bundle — access token, refresh token, and an expiry a few hours out — so it
needs `xai-responses`, which unwraps the bundle and renews it before expiry.
Pointing `openai-responses` at a stored login puts the whole JSON bundle in the
`Authorization` header, and xAI answers `incorrect API key provided`; Smith
refuses that combination at startup and names the fix.

`/connect xai` performs the browser login and writes:

```toml
[providers.xai]
kind = "xai-responses"
base_url = "https://api.x.ai/v1"
credential = "authfile:xai"
```

An API key from console.x.ai — what `smith setup` collects — uses the generic
kind instead. So does reading the session the separate `grok` CLI already
manages, because the JSON pointer extracts the token rather than the bundle:

```toml
[profiles.grok]
provider = "xai"
model = "grok-4.5"

[providers.xai]
kind = "openai-responses"
base_url = "https://api.x.ai/v1"
# An xAI API key from console.x.ai, or the token field of the `grok` CLI's own
# session file — see the credential list below.
credential = "session-json:~/.grok/auth.json#/access_token"

```

No `[models]` block is needed here: Smith's embedded Models.dev catalog
supplies xAI's limits, and the same is true for OpenAI, OpenRouter, and the
Z.AI Coding Plan at their exact published endpoints. Declare `[models]` only
for a model the catalog does not carry, or to override what it says.

A catalog entry is bound to an exact adapter kind *and* endpoint pair. A
provider you happen to name `xai` pointing somewhere else inherits nothing,
because a catalog describes a model as one deployment serves it. Extra headers are sent unchanged, but authorization
header names such as `Authorization`, `X-API-Key`, and `Api-Key` are refused;
credentials belong in the credential broker.

Choose exactly one credential source:

- `credential = "session-json:<path>#/<field>"` reads one field out of a JSON
  credential file another tool owns, such as the browser-login session the
  `grok` CLI writes to `~/.grok/auth.json`. Smith reads it and never writes
  it: the owning tool refreshes on its own schedule and a second writer would
  race it. The file is re-read on each resolution, so a refreshed token is
  picked up without restarting Smith.
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

## Runtime connections

`/connect [PROVIDER]` and `/disconnect [PROVIDER]` are idle-only local TUI
actions. They preserve the current Smith session and apply changes only after
the direct runtime has shut down at its safe boundary. Reconnecting a declared
provider changes only `providers.<name>.credential` or its user-only
`api_key`; endpoint, model, limits, profiles, and defaults are not rewritten.
Disconnect removes only those authentication leaves and rolls the config back
if protected-credential cleanup fails.

`openrouter` is a reviewed built-in connection descriptor with fixed provider
identity, `openai-compatible` adapter, and
`https://openrouter.ai/api/v1`. A fresh connection selects exact model limits
from Smith's embedded/last-good OpenRouter catalog without network or provider
I/O during the credential ceremony. Custom endpoints remain a `smith setup
add-provider` operation.

Google AI Studio is a native, stateless Gemini connection. Create an API key in
[Google AI Studio](https://ai.google.dev/gemini-api/docs/api-key), then run
`/connect google` or use this minimal user configuration:

```toml
default_profile = "gemini"

[profiles.gemini]
provider = "google"
model = "gemini-3.6-flash"

[providers.google]
kind = "gemini-interactions"
credential = "env:GEMINI_API_KEY"
```

Smith owns the fixed Google Generative Language endpoint for this adapter and
rejects `base_url`, custom headers, hosted-tool settings, and guessed model
limits. Model capabilities and limits come from the same embedded/last-good
Models.dev snapshot used by `/model` and setup; no `[models]` entry is needed
for catalog-backed Gemini models. The provider sends native Gemini
`thinking_level` values when the selected catalog model advertises them.

ChatGPT is a fixed experimental direct provider. `/connect chatgpt` writes only
the trusted declaration below and stores the renewable bundle in the
`chatgpt` entry of the fixed plaintext `~/.smith/auth.json` file:

```toml
[providers.chatgpt]
kind = "chatgpt-responses"
base_url = "https://chatgpt.com/backend-api/codex"
credential = "authfile:chatgpt"

[models."chatgpt/gpt-5.6-terra".reasoning]
mandatory = true
efforts = ["low", "medium", "high", "xhigh", "max", "ultra"]
default_enabled = true
default_effort = "medium"
dialect = "openai-effort"
```

The endpoint, credential reference, OAuth issuer/client/scopes, callback ports,
and account header are product constants; project layers cannot redirect them.
Smith creates `~/.smith` as mode `0700` and `auth.json` as a regular mode `0600`
file, uses a cross-process lock and atomic replacement, and refuses symlink,
non-regular, malformed, or oversized storage. This avoids Keychain/Secret
Service prompts but is not encryption at rest: same-user processes and backups
can read or retain the tokens.

Legacy `keychain:smith/chatgpt` entries are not read, migrated, or deleted.
Reconnect from a session running another configured provider to create the new
auth-file entry, then remove the legacy item manually if desired.
Trusted metadata supplies a 272,000-token context window and Smith enforces a
conservative 16,384-token output cap. The underlying subscription-token API is
not a supported public OpenAI Platform contract. No Codex installation or auth
cache is used.

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

Reasoning presence is separate from reasoning control. On an unknown
OpenAI-compatible endpoint a catalog `reasoning = true` value means only that
reasoning is present and fixed; it does not create a toggle or effort
selector, and rich `[models."provider/model".reasoning]` metadata must name
the exact wire dialect (`openai-effort`, `openrouter`, or `zai-thinking`),
switch behavior, and ordered effort values. Four exact endpoints normalize
controls themselves and need no per-model metadata: the OpenAI endpoint
speaks `reasoning_effort`, the xAI Responses endpoint grants the same
OpenAI-effort dialect for catalog-backed Grok reasoning models, the
OpenRouter endpoint speaks the unified `reasoning` object with an on/off
switch, and the Z.AI Coding Plan endpoint speaks its documented thinking
toggle; the native Gemini endpoint speaks catalog-advertised
`thinking_level` values. On those endpoints the frozen Models.dev snapshot
supplies each model's advertised control shape — its exact effort ladder
(for example `none…xhigh` on newer OpenAI families, `low…high` on Grok and
older OpenAI ones) or a bare toggle. On the OpenAI and xAI endpoints, `off`
exists only where the ladder advertises `none`. A reasoning model the
snapshot has not annotated falls back to the endpoint's universal
`low`/`medium`/`high` ladder. Explicit per-model metadata still overrides
everything; token-budget options are not yet consumed.

`[reasoning]` and `[profiles.<name>.reasoning]` select `enabled` and/or one
advertised `effort`. Omission sends no reasoning option and preserves provider
behavior. Native Gemini reasoning uses the catalog's `thinking_level` values
and is mandatory when the model advertises a ladder; `/think off` is rejected
for that model. The exact Z.AI Coding Plan binding exposes its documented thinking
toggle but no general effort ladder. `/think [on|off|default]` and
`/effort [LEVEL|default]` apply session overrides at an idle boundary; omitted
arguments open local bounded selectors and make no provider request. Invalid
or mandatory/fixed choices fail before credential lookup. Higher effort can
increase latency, token use, and cost.

`--effort NAME` selects one advertised effort for a single invocation, so an
effort tier does not have to be deployed as a named profile. The names are the
binding's own, not a fixed set: `smith --effort <NAME>` refuses an unadvertised
value before any credential lookup or provider request and lists what the
binding does advertise. A binding with no adjustable reasoning, and a
toggle-only binding with no ladder, both refuse the flag rather than ignore it.
The flag ranks at the command-line layer — above `[profiles.<name>.reasoning]`,
above `[reasoning]`, above `SMITH_REASONING_EFFORT`, and below an in-session
`/effort` — and `smith config explain reasoning.effort` names it as the source.
It selects an effort only; it never turns reasoning on or off.

On `--resume`, an explicitly supplied `--effort` answers for that run and
*shadows* the session's saved `/effort` choice without replacing it: the saved
value is neither applied nor overwritten, so resuming later without the flag
restores the session's own effort. The saved thinking state (`/think`) is
unaffected either way.

`SMITH_REASONING_EFFORT` addresses the same setting from the environment, one
layer lower; a flag on the same run wins.

## Policy keys and defaults

Smith's defaults claim no provider, model, model limit, model-dependent output
reserve, capability budget, or estimated-count slack.

| Key | Built-in default | Meaning |
| --- | ---: | --- |
| `context.reasoning_reserve` | `0` | Continuation/reasoning reserve |
| `context.compaction_high_watermark_percent` | `85` | Pressure trigger |
| `context.compaction_low_watermark_percent` | `60` | Post-compaction target |
| `context.cache.maintenance` | `"off"` | Requested cache maintenance: `off`, `observe`, or `adaptive` |
| `context.cache.inactivity_limit_ms` | `3600000` | Shared parent inactivity/idle-compaction boundary |
| `context.cache.max_hold_while_child_ms` | `3600000` | Maximum parent cache hold while a child remains active |
| `context.cache.max_maintenance_calls` | `1` | Synthetic calls per parked interval |
| `context.cache.max_maintenance_input_tokens` | `0` | Synthetic input ceiling; `0` uses the exact plan/model budget |
| `context.cache.max_maintenance_output_tokens` | `256` | Synthetic output ceiling |
| `context.cache.maintenance_deadline_ms` | `30000` | Per-synthetic-call deadline |
| `context.cache.keepalive_margin_ms` | `120000` | Early margin before a declared retention boundary |
| `context.cache.keepalive_jitter_percent` | `10` | Bounded scheduling jitter |
| `context.cache.handoff_checkpoint` | `true` | Permit a conformance-gated same-model handoff |
| `context.cache.idle_compaction` | `true` | Attempt ordinary semantic compaction once at inactivity |
| `context.cache.resume_capsule` | `true` | Persist the redaction-safe cold-continuation projection |
| `child_agents.wait_default_timeout_ms` | `300000` | Default five-minute foreground `agent.wait` boundary; zero is an immediate status check |
| `child_agents.wait_max_timeout_ms` | `300000` | Maximum accepted foreground `agent.wait.timeout_ms`; timeout leaves the child running |
| `limits.max_retries` | `2` | Retries after the first provider attempt |
| `limits.max_tool_steps` | `0` | Tool-loop ceiling per turn; `0` removes it |
| `limits.turn_time_limit_ms` | `0` | Whole-turn deadline; `0` removes it |
| `limits.tool_output_limit_bytes` | `65536` | Inline output/offload threshold |
| `persistence.enabled` | `true` | Save project-partitioned sessions |
| `persistence.sessions_dir` | `~/.smith/sessions` | User-owned state root |
| `persistence.journal_events` | `true` | Write redacted JSONL events |
| `approval.mode` | `"ask"` | Fail closed until a decision exists |
| `approval.auto` | `[]` | Versioned prepared-call grants; user-controlled configuration only |
| `background.exit_policy` | `"error"` | Refuse to orphan active work |
| `background.max_children` | `4` | Concurrent child capacity |
| `reasoning.enabled` | provider/model default | Explicit thinking state when supported |
| `reasoning.effort` | provider/model default | Exact advertised effort when supported |
| `background.max_monitors` | `8` | Reserved process-monitor capacity |

The compaction low watermark must be below the high watermark. Reserves must
leave a positive input budget. Numeric limits are validated before any provider
request.

Approval modes are `"ask"`, `"deny"`, and `"allow-all"`. Background exit modes
are `"error"`, `"wait"`, and `"stop"`. Non-empty legacy
`approval.auto_approve` lists are rejected because a tool name is not an
authority boundary. A user-owned configuration may instead grant exact,
revisioned prepared `edit` operations:

```toml
[[approval.auto]]
revision = 1
tool = "smith/edit"
operations = ["replace", "create"]
permissions = ["fs.read", "fs.write", "fs.create"]
max_risk = "medium"
mount = "workspace"
paths = ["src/**", "tests/**"]
expires_at = "2026-12-31T23:59:59Z" # optional
max_uses = 50                        # optional
```

Every field is matched against the immutable prepared call. Operation, mount,
path, permissions, and derived risk must all fit. Host filesystem, arbitrary
process, network, credential, external-service, data-egress, and unclassified
authority are never eligible. There is deliberately no scoped host-shell rule;
already-isolated automation must opt into explicit `allow-all`.

## Command-line selection

The run surface accepts:

```text
--project PATH
--profile NAME
--agent MODE                  # deprecated legacy mode compatibility
--provider NAME
--model ID
--effort NAME                 # one provider-advertised reasoning effort
--approval ask|deny|allow-all
--yolo                       # explicit alias for --approval allow-all
--allow-synthetic-cache-spend # trusted-host authority for bounded maintenance
--background-exit error|wait|stop
--resume [SESSION_ID]
--output-format text|json|stream-json
```

An omitted `--resume` ID opens a local picker only in an interactive terminal.
Headless or piped use must pass the exact ID. `--output-format` requires `-p`.

Project-controlled configuration cannot grant `allow-all`, add automatic
approval rules, disable/redirect persistence, or choose the user session root.
Those settings require a higher-trust source. Opening a repository is never
authority. `--yolo` is only a shorter explicit spelling of
`--approval allow-all`; it does not add tools or override a profile's
read-only posture. Likewise, only the explicit
`--allow-synthetic-cache-spend` host flag grants adaptive cache-maintenance
spend; no repository setting or environment variable can manufacture it.
