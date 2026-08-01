## Context

Smith's current startup path is:

```text
run_command
→ start_host
→ prepare
→ smith_config::resolve
→ smith_runtime::host::start
→ terminal::enter
```

`smith_config::resolve` requires a selected declared provider and model.
Consequently a fresh install exits at `provider is not set` before Smith can
render guidance. A usable custom model also needs enforceable context, input,
and output limits, and the current credential boundary resolves references but
does not enroll a secret.

Live compatibility verification against the Z.AI Coding endpoint exposed one
additional readiness requirement. GLM-4.7 enables compulsory thinking by
default and returned user-facing greetings only as `reasoning_content`; Smith
normalized that wire field as a reasoning event and then reported no visible
answer. A control request through OpenRouter produced ordinary `content` and
Smith emitted the expected visible text, isolating the behavior to the Z.AI
response convention rather than the shared adapter generally. Disabling GLM
thinking also produced ordinary content, but removes a model capability the
coding preset should retain. The quick start must therefore configure response
normalization as well as endpoint, credential, model, and limits.

Two reference clients illustrate useful, different patterns:

| Client | Empty-install behavior | Persistence |
| --- | --- | --- |
| Claude Code (`a93016d`) | Blocks the first interactive launch with preflight, theme, auth/API-key approval, security notes, and optional terminal setup; non-interactive runs skip onboarding | Stores auth separately and marks onboarding complete |
| OpenCode (`a45c2b`) | Opens an empty shell, suggests `/connect`, then funnels provider → auth method/key → model | Stores auth under user state and model recents separately |

Smith is provider-neutral like OpenCode, but its runtime cannot exist without
an enforceable model profile. The selected design is therefore a hybrid:
automatically open a focused setup flow only when an interactive launch is
genuinely unconfigured, while also keeping an explicit `smith setup` entry
point. Readiness is derived from current configuration rather than a separate
"onboarding complete" bit that can become stale.

## Goals / Non-Goals

### Goals

- Let a new interactive user reach a valid first session without hand-writing
  TOML.
- Explain and collect every value required for provider composition and model
  enforcement.
- Let users discover locally configured runtime choices and resumable sessions
  without memorizing identifiers.
- Keep secrets out of configuration, diagnostics, render state, transcripts,
  and journals.
- Preserve the same complete resolver and runtime preflight as every normal
  launch.
- Make cancellation, failure, and non-interactive use predictably
  non-mutating.

### Non-Goals

- Add a general-purpose editor for every Smith setting.
- Modify project `.smith/` configuration during setup.
- Add an OAuth protocol or provider adapter that the pinned Agent Runtime does
  not support.
- Probe a provider with a paid inference request to validate credentials.
- Fetch remote provider model catalogs merely to populate a selector.
- Generate session titles with a provider request or expose tool arguments,
  reasoning, or assistant content in session previews.
- Guess model limits or silently accept an unknown model.
- Expose an arbitrary JSON request-body editor or allow setup data to replace
  Smith-owned request fields such as messages, tools, model, or streaming.
- Add theme, telemetry, security-note, or terminal-settings onboarding in this
  first flow.
- Implement the encrypted-file credential fallback; setup offers platform
  credential storage or an external environment reference.

## Decisions

### Readiness is a typed pre-resolution result

`smith-config` will expose an inspection path that performs discovery, file
parsing, and declarative validation before extracting every required run
field. It returns one of:

```text
Ready(Resolution)
Unconfigured(SetupContext)
Invalid(ConfigError)
```

`Unconfigured` means there is no effective provider/model setup intent in the
user file, project files, environment, or CLI/session overrides. A malformed
file, an unknown key, a half-declared provider, an unknown provider reference,
or an invalid model limit is `Invalid`, not an invitation to overwrite the
user's work. `Ready` contains the same `Resolution` the existing resolver
would return.

There is no persisted completion flag. Removing or invalidating the only
usable setup makes a later interactive launch derive the appropriate state
again.

Alternative considered: catch any `MissingSetting` and start the wizard.
Rejected because it would turn typos and partial configuration into an
implicit rewrite path.

### Interactive auto-entry, explicit re-entry, headless refusal

An unconfigured `smith` invocation with an interactive terminal enters the
setup surface before `start_host`. `smith setup` enters it explicitly and may
update user-scoped setup after showing the exact non-secret diff. Normal
configuration errors remain ordinary diagnostics.

`smith -p`, piped input, machine-output modes, `sessions`, and `config explain`
never prompt or write setup state. An unconfigured run returns a stable
non-success result explaining that the user can run `smith setup` in a TTY or
supply complete configuration through existing layers.

Alternative considered: OpenCode's passive empty shell with only a
`/connect` hint. Rejected as the sole first-run path because Smith cannot
construct its ordinary app/session state until provider and model limits are
valid. The explicit `smith setup` command preserves re-entry without pretending
the agent runtime is available.

### A short provider-to-model funnel

The setup state machine is:

```text
action
→ provider
→ authentication
→ model
→ review
→ persist and preflight
```

Every step supports Back; Cancel restores the terminal and commits nothing.
The review screen shows the destination user-config path, provider, adapter,
endpoint, credential reference (never its value), model, and the provenance of
all model limits.

Provider choices come from Smith-owned setup descriptors. A descriptor contains
presentation metadata, its shared adapter kind, an optional default endpoint,
supported credential methods, and model catalog records. The list is filtered
against the adapters compiled into the pinned runtime. The initial flow also
includes:

- a GLM quick start named `glm`, using provider `zai`, adapter
  `openai-compatible`, endpoint
  `https://api.z.ai/api/coding/paas/v4`, model `glm-4.7`, a 200,000-token
  context window, 196,000 maximum input tokens, 131,072 maximum output tokens,
  an 8,192-token default request/output reserve, and provider response setting
  `reasoning_only = "text"`; and
- a custom OpenAI-compatible path that asks for an endpoint and model.

A catalog-backed model may omit limits from user TOML because the immutable
runtime profile will retain trusted catalog provenance. A custom model absent
from every trusted catalog must collect all three enforceable limits and write
them explicitly. The review screen labels explicit versus catalog values.

Alternative considered: discover models from the provider before saving
credentials. Rejected for the first version because provider APIs vary, can
incur network/security concerns, and still do not necessarily supply
enforceable limits.

### Reasoning-only response compatibility is typed and reviewed

OpenAI-compatible providers do not all use `reasoning_content` with identical
terminal semantics. This change adds a narrow typed provider response policy:

```toml
[providers.zai.response]
reasoning_only = "text"
```

`smith-runtime` applies this policy through a provider-stream decorator. It
buffers only non-redacted reasoning deltas for one provider attempt:

- if ordinary text arrives, the buffered events remain reasoning and the text
  remains the visible answer;
- if a tool call arrives, the buffered events remain reasoning and tool
  execution proceeds unchanged;
- if the attempt stops successfully with reasoning but no ordinary text or
  tool call, the buffered text is emitted once as ordinary visible text; and
- redacted reasoning is never promoted.

The default and an omitted setting preserve the existing reasoning
classification. Invalid values and use with an incompatible adapter fail
during configuration resolution, before credentials or network access.

The GLM descriptor selects `text` because the Coding endpoint demonstrably
uses a reasoning-only stream for user-facing answers. Setup review shows this
compatibility policy alongside the endpoint and model. The generic
OpenAI-compatible path leaves it unset unless the user selects the explicitly
labeled option.

Disabling GLM thinking was rejected as the primary fix because it changes model
behavior to compensate for response interpretation. Immediately remapping
every reasoning delta was also rejected because a model that later emits
ordinary text or a tool call may have supplied genuine reasoning first. The
attempt-level fallback preserves that distinction and never promotes redacted
material. The decorator remains in Smith; the shared Agent Runtime request
model and adapter do not change.

### Setup is also the additive configuration command

First-run setup chooses the GLM quick start or adds a provider with its first
model. Re-entering `smith setup` begins with these actions:

```text
Quick start with GLM
Add provider
Add model to existing provider
Change default profile/model
```

The stable direct forms are:

```text
smith setup add-provider
smith setup add-model --provider <name>
```

`add-provider` collects the provider declaration, authentication, and at least
one usable model. `add-model` selects an existing provider, collects a model
ID plus trusted or explicit limits, and writes only the corresponding
`[models."<provider>/<model>"]` record unless the user also confirms making it
the selected default. If no provider is supplied, the interactive command
opens a provider picker. A model for a different provider is never attached to
the current provider implicitly.

Alternative considered: make `/model <id>` create missing model records.
Rejected because `/model` is a fast runtime selector and has no safe place to
collect endpoint, credential, and limit provenance. It continues to select
configured models; setup owns durable additions.

### Omitted selector arguments open a shared resource picker

The slash-command registry keeps direct argument forms, but selection
arguments become optional:

```text
/model [PROVIDER/MODEL]
/provider [NAME]
/profile [NAME]
/resume [SESSION_ID]
```

Submitting a selector without its argument transitions from command discovery
into a searchable resource picker. This is a distinct temporary interaction
state backed by typed entries, not a second command registry. In the runtime
surface it temporarily replaces the todo presentation in the compact pane
directly above the fixed composer, shows at most five matching rows, and
scrolls the selected window through larger inventories without covering the
transcript. Closing the picker restores the unchanged todo projection.
Up/Down changes selection,
typing filters, Enter confirms, and Escape returns to the composer without
changing the current runtime or session. Explicit values still bypass the
picker after local validation, so knowledgeable users and tests retain a fast
path.

The picker state, reducer, and row projection are shared by first-run setup,
runtime selection, and pre-host resume selection. Setup and pre-host resume
may use the larger bordered surface because no transcript/composer exists yet;
runtime selection remains the compact pane. Both presentations are
keyboard-only, scrollable, usable in narrow/no-color terminals, and bounded
for large inventories. Opening, filtering, cancelling, or confirming a picker
sends no provider request.

Alternative considered: print an inline list and require the user to retype an
identifier. Rejected because it preserves the original copy/paste problem and
loses the selection, filtering, active-marker, and empty-state behavior already
appropriate for an interactive safety surface.

### Available models are valid provider/model pairs

`smith-config` exposes a provenance-aware local selection inventory containing
effective profiles, declared providers, and provider/model pairs whose adapter
and enforceable model metadata are available from configuration or the trusted
catalog. Enumeration does not resolve credentials, open a keychain, or access
the network; full runtime preflight still occurs after a choice.

`/model` shows every locally selectable pair across providers, labelled
`provider/model`, with the active pair and associated profile names marked.
The selection result contains both provider and model, and Smith applies them
atomically before resuming the current session. An unqualified explicit model
name is accepted only when it resolves uniquely within the active provider or
globally; ambiguity returns to a filtered picker rather than guessing.

`/provider` lists configured providers. Choosing one with a single selectable
model applies the pair atomically; choosing one with several immediately opens
the model picker filtered to that provider. Smith never carries an incompatible
model name from the prior provider. `/profile` lists each profile with its
resolved provider/model summary and applies the coherent profile selection.

An empty model/provider/profile picker explains that no local choice is
available and points to `smith setup add-model` or
`smith setup add-provider`. Remote provider catalogs are intentionally not
queried: setup owns durable additions and trusted limits.

### Resume choices carry local meaning

`/resume` lists only sessions belonging to the current canonical project,
newest first. Each entry shows a shortened ID, update time, turn count,
provider/model when known, and a bounded single-line preview derived locally
from the most recent user message. It never includes reasoning, tool arguments,
tool results, assistant text, or secret-bearing configuration. Older snapshots
without summary metadata remain selectable with the unknown fields labelled.

The session store's lightweight listing metadata is versioned and
backward-compatible. It is prepared during ordinary persistence so opening the
picker does not materialize full histories or call a provider. The current
session is marked, and selecting it is a no-op. An empty list explains that the
project has no saved sessions and offers `/new`.

At process startup, interactive `smith --resume` without an ID opens the same
project-scoped picker before creating a host or session. Explicit
`--resume <SESSION_ID>` remains unchanged. A no-ID resume in headless,
machine-output, or non-TTY use fails locally with a stable diagnostic pointing
to `smith sessions list`; it never chooses the newest session implicitly.

### Secrets use a write-only enrollment boundary

The existing runtime-facing credential resolver remains read-only. Setup adds
a separate injectable enrollment interface with `store` and cleanup/restore
operations for the OS credential service. API-key input is masked and held only
for the active setup transaction; it is never inserted into TOML or a normal
TUI transcript.

The default API-key choice writes a keychain/Secret Service entry and records a
reference such as `keychain:smith/<provider>`. The alternative asks for an
environment variable name and records `env:<NAME>` without reading or copying
the value. If platform credential storage is unavailable, setup keeps the user
in the authentication step and offers the environment-reference path; it
never falls back to plaintext.

### User configuration is a reviewed transaction

Automatic first-run setup writes only `~/.smith/config.toml`. It creates the
user directory with restrictive permissions, builds the proposed edit in
memory, validates it, and publishes the file with a same-directory atomic
replace. Explicit re-entry preserves unrelated tables/comments and refuses an
unconfirmed collision with an existing profile/provider/model.

After persistence, Smith performs complete configuration resolution,
credential resolution, model-profile resolution, workspace construction, and
runtime factory preflight without creating a session or sending a provider
request. Only a successful preflight may transition to the ordinary TUI.
Failure restores the prior config file when possible, reports an actionable
error, and leaves no completion marker. An enrolled key may be retried or
cleaned up through the enrollment boundary without ever being displayed.

### Setup has its own terminal lifecycle

The setup surface is pure state and rendering in `smith-tui`; filesystem,
credential, and runtime operations are effects orchestrated by `smith-cli`.
It may enter the alternate screen before a runtime exists, which is a narrow
exception to the normal startup rule. Every completion, cancellation, error,
panic boundary, and signal path restores the terminal before returning or
starting the normal host.

No session identity, journal, approval channel, tool registry, or provider
transport is created while setup is active.

## Risks / Trade-offs

- Provider/model setup descriptors can become stale. Each descriptor and model
  record therefore needs a versioned source and deterministic tests; an
  unsupported adapter is hidden rather than routed through a different one.
- A provider could misuse `reasoning_content` for both private reasoning and a
  reasoning-only final answer. Promotion is therefore opt-in per provider,
  limited to non-redacted output, and occurs only when the attempt produced no
  ordinary text or tool call.
- Cross-store atomicity is imperfect because an OS credential service and a
  TOML file cannot share one transaction. Staged validation, config rollback,
  and credential cleanup minimize partial state; readiness remains derived so
  no stale completion bit can conceal failure.
- A custom OpenAI-compatible setup still asks advanced users for model limits.
  This is intentional until a trusted catalog supplies them.
- Session previews trade extra lightweight persistence metadata for meaningful
  discovery. Fields are bounded and locally derived, and older records degrade
  to labelled unknown values rather than requiring migration.
- A global model list can contain duplicate model names. Provider-qualified
  identities and atomic pair selection prevent ambiguous or invalid switches.
- A pre-runtime alternate screen adds another restoration path. The terminal
  guard and pseudo-terminal tests are release requirements, not optional
  polish.
- Explicit setup can collide with user-authored configuration. The non-secret
  diff and confirmation protect ownership; automatic setup never edits a
  non-empty or invalid user file.

## Migration Plan

- Add readiness inspection alongside the existing resolver and prove `Ready`
  is behaviorally identical to `resolve`.
- Add the typed reasoning-only response policy, its provider-stream decorator,
  setup descriptors/model records, and an injectable credential enrollment
  boundary.
- Add safe user-config editing and transaction tests before any UI writes it.
- Implement the pure setup reducer/renderer and terminal host loop.
- Add local configuration/session inventories and the shared resource picker,
  then route no-argument selector commands and interactive no-ID resume through
  it.
- Route unconfigured interactive launch and `smith setup` into the flow while
  preserving headless and invalid-config diagnostics.
- Re-run full preflight after commit, then transition into the existing
  `run_interactive_command` path.
- Document first-run and reconfiguration behavior and run macOS/Linux terminal
  tests plus the workspace quality gates.

## Open Questions

None for proposal approval. Named provider presets may be added or updated as
versioned data, but the first implementation must support the GLM quick start,
the generic OpenAI-compatible path, additive provider/model setup, and local
selection/resume discovery while exposing only adapters present in the current
runtime.
