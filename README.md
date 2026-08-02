# Smith

Smith is a terminal-first coding agent: a TUI and a non-interactive `-p` mode
over a shared, provider-neutral agent runtime.

## Where the code lives

Smith consumes [`../agent-runtime`](../agent-runtime), which owns reusable
**mechanism** — provider adapters, the direct provider/tool loop, versioned
events, disjoint usage accounting, cancellation, and deterministic testing.
The dependency is one-way: the runtime never depends on Smith.

This repository owns Smith's **policy** — the parts a product must decide for
itself, which the runtime deliberately ships as traits only:

| Crate | Owns |
| --- | --- |
| `smith-config` | Layered configuration, provenance, credential references, and project trust |
| `smith-host` | Prepared approval/question brokers and the project workspace boundary |
| `smith-runtime` | The single runtime factory, harness policy, checkpoints, artifacts, snapshots, and event journal |
| `smith-tools` | The coding tools: `read`, `list`, `search`, `edit`, `shell` |
| `smith-tui` | Transcript, status, composer, theme, key map, and rendering |
| `smith-cli` | The `smith` binary, terminal host loop, and headless output contracts |

### Tools

Every path resolves through the session's workspace, so containment is enforced
in one place rather than re-implemented per tool. Each invocation is prepared
before approval: argument validation and canonical resource resolution produce
the exact permissions, target, deadline, display, and fingerprint that are
authorized and then executed. `edit` requests authority for its canonical file;
`shell` conservatively declares broad workspace, process, and network authority.
The read-only three run without asking. Reads, listings, searches, and command
output are bounded. Oversized output is stored in the session-private artifact
store and represented to the model by a bounded preview plus a retrievable
reference.

`shell` puts each command in its own process group and signals the group, so a
build script's background watcher does not outlive the invocation.

Smith's root surface also composes `ask_user`, `write_todos`, and depth-one
delegation through Agent Runtime's ability registry. Initial activation is
intent-scoped: a read-only request does not advertise mutation merely because
`edit` and `shell` are registered. Trusted skills, bounded memory, todo state,
artifact offloading, and semantic summarization contribute through the same
session-scoped harness pipeline.

When ordinary session persistence and protected checkpoints are both
available, delegated children are durable addressable sessions. `@child-1
continue …` starts a confirmed follow-up on that same child's canonical
history; `/agent resume child-1` separately confirms continuation of one safe
interrupted checkpoint. Startup, `/agent`, `/timeline`, and headless status
only inspect retained metadata and never spend provider tokens. Smith never
turns a missing or incompatible child ID into an implicit new spawn.

Persisted root sessions may also own one explicit multi-turn goal. `/goal
<objective>` creates it locally; bare `/goal` shows canonical status, reported
token evidence, budget, and active elapsed time. Smith continues an active goal
only while this process is running and the session is idle. Real user input
wins admission, child and ephemeral sessions never receive goal abilities, and
no daemon or restart-time scheduler is created.

During eligible provider-backed work, ordinary composer input has two explicit
intents: `Enter` steers the serving turn at its next safe boundary, while
`Tab` stores a process-local future whole turn. Pending text is not added to
canonical transcript history until Agent Runtime emits its committed steering
disposition. `Alt+Up` restores the newest Smith-owned queued turn for editing;
`Esc` interrupts and resends only steers the runtime actually discarded. File
identities remain exact and their bytes are read when the queued turn starts.
Slash commands, shell shortcuts, child operations, approvals, and
questionnaires are never queued implicitly.

## Running it

Smith requires Rust 1.88 or newer.

```sh
cargo run -p smith-cli --                         # interactive TUI
cargo run -p smith-cli -- setup                   # guided provider/model setup
cargo run -p smith-cli -- -p "explain this repo" # one headless turn
cargo run -p smith-cli -- sessions list
cargo run -p smith-cli -- config explain model
cargo test --workspace
cargo clippy --workspace --all-targets
```

Smith discovers the nearest `.smith/config.toml`, then layers user,
project-local, profile, `SMITH_*`, and CLI settings.

### First run and ongoing setup

A plain interactive `smith` inspects configuration before entering raw terminal
mode. A genuinely empty install opens guided setup; a partial or malformed
configuration is reported as invalid and is never overwritten. Headless,
piped, machine-output, session-list, and config-explain commands remain
non-interactive and non-mutating.

The first-run GLM path configures Z.AI's Coding Plan endpoint and `glm-5.2`,
including the verified 1M context/input limits, 131,072 model-output ceiling,
Smith's 32,768 per-request output budget, and compatibility policy for
providers that deliver a successful final answer only as reasoning content.
Ongoing changes use the same reusable surface:

```sh
smith setup
smith setup add-provider
smith setup add-model
smith setup add-model --provider openrouter
smith setup credential --provider openrouter
smith setup checkpoint-key
```

The generic provider path asks for a stable provider name, OpenAI-compatible
base URL, credential reference, exact model ID, explicit model limits, response
behavior, and whether the new pair becomes the default. `add-model` adds a
model beneath an existing provider without redeclaring that provider.

API-key input is masked. Authentication offers three explicit storage models:

- Keychain / Secret Service is the default and gives the strongest at-rest
  protection, but the operating system may ask for access again—especially for
  a locally rebuilt binary.
- An environment reference such as `env:OPENROUTER_API_KEY` leaves storage to
  the shell or an external secret manager. Smith stores only the variable name.
- “Store in config (no prompts)” writes `api_key = "…"` only to the owner-only
  user file `~/.smith/config.toml`. This is self-contained and avoids
  credential-service prompts, but it is plaintext: other processes running as
  the same OS user and backups can read it.

The local-config choice is never an automatic fallback. Setup warns before
accepting it, masks the input, renders `api_key = [redacted]` during review,
and publishes the config through a mode-`0600` atomic replace. Project and
project-local files cannot contain `api_key`; ordinary startup also refuses an
inline-key user config that is a symlink, non-regular, owned by another user,
or group/world accessible.

Before committing, setup shows the secret-free change and exact destination,
then performs a local factory preflight. A failed preflight restores the exact
prior config bytes and any Keychain value it replaced. Runtime credential
reads have a 30-second startup boundary, so a hidden or unanswered platform
unlock prompt fails with an `env:<VAR>` recovery hint instead of hanging.
`smith setup credential --provider <name>` changes only that existing
provider's credential source; it does not rewrite its endpoint, models,
limits, profiles, or default.

The same credential funnel is available at an idle TUI boundary with
`/connect [PROVIDER]`; omitting the name opens a searchable provider picker.
`/disconnect [PROVIDER]` removes only that authentication source, and
`/status` reports the connection method/readiness without showing a key,
account ID, or credential locator. OpenRouter is built in: `/connect
openrouter` fixes `https://openrouter.ai/api/v1`, offers the normal protected
credential choices, and hands off to a reviewed catalog-model picker with
enforceable limits. Existing OpenRouter connections change only authentication.

ChatGPT subscription login is deliberately different and explicitly
experimental. `/connect chatgpt` offers Smith-owned browser PKCE or device-code
login in a bounded popup, stores one renewable token bundle in the `chatgpt`
entry of the fixed owner-only plaintext file `~/.smith/auth.json`, and adds a
trusted `chatgpt/gpt-5.6-terra` model. Smith creates `~/.smith` as mode `0700`
and `auth.json` as mode `0600`; same-user processes and backups can still read
or retain these tokens, so treat the file like a password. ChatGPT connect,
startup, refresh, reconnect, and disconnect never query Keychain or Secret
Service.
Smith then calls the fixed ChatGPT Codex Responses endpoint directly through
its normal runtime, so Smith tools, approvals, attachments, steering, goals,
checkpoints, persistence, cancellation, recovery, events, and usage remain in
force. No Codex executable or another client's auth cache is used.

This subscription-token endpoint is not a supported public OpenAI Platform API
contract and may change without compatibility notice. Use an OpenAI Platform
API-key provider when a supported integration is required. `/disconnect
chatgpt` removes Smith's protected bundle while preserving the endpoint and
model declaration.

If an earlier Smith build stored ChatGPT at `keychain:smith/chatgpt`, that
legacy entry is intentionally neither read nor deleted. Start Smith with
another configured provider, run `/connect chatgpt`, and complete a fresh login
to publish `authfile:chatgpt`. Remove the old Keychain item manually only if you
want to clean it up without involving Smith.

Checkpoint protection is configured separately. `smith setup checkpoint-key`
offers owner-only inline storage, an environment-variable reference, or OS
protected storage. Inline/environment choices never initialize Keychain or
Secret Service, remain redacted in config explanation and journals, and keep
the exact checkpoint authenticated-encrypted. Source changes refuse without
modification while encrypted checkpoints exist, so old resumable state is
never silently abandoned.

Encrypted `file:` references are reserved for a future externally unlocked
encrypted-store backend; Smith does not silently treat a plaintext file as
encrypted storage. When rotating a key, revoke the old key at the provider,
rerun credential setup (or carefully replace the user-only `api_key`), verify
`chmod 600 ~/.smith/config.toml`, and account for retained backup copies.

For local development, a minimal offline smoke configuration is:

```toml
default_profile = "dev"
profile_order = ["dev", "review"]

[profiles.dev]
provider = "local"
model = "example-model"
description = "local implementation"
posture = "build"
use = ["main", "child"]
instructions = "Implement the request and verify the result."

[profiles.review]
extends = "dev"
description = "read-only independent review"
posture = "review"
use = ["main", "child"]
instructions = "Report prioritized evidence-backed findings."

[providers.local]
kind = "fake"

[models."local/example-model"]
context_tokens = 128000
max_input_tokens = 124000
max_output_tokens = 4096
```

Inside the empty idle TUI, the footer identifies the active agent profile
beside provider/model and context confidence. `Tab` cycles the main-enabled
profiles in `profile_order` only at that safe empty/idle boundary. If the order
is omitted, Smith derives it deterministically from real main-enabled profiles;
guided setup creates build, plan, and review variants explicitly. `@`
completes canonical workspace files and child-enabled profiles, plus retained
child IDs available for follow-up; leading `!` runs the canonical
prepared local-shell path without a provider request. `@@` and `!!` are the
literal escapes. `/details`, `/timeline`, and `/redo` expose bounded live work,
canonical session evidence, and exact recovery locally.

`Ctrl+P` opens the command palette. `/model`, `/provider`, `/connect`,
`/disconnect`, `/profile`, and `/resume` need no argument: each opens a
searchable local list with current and unavailable states. Explicit values
remain validated shortcuts. Model rows are always provider-qualified, so
selecting a model applies its provider/model pair atomically; selecting a
provider with several models cascades into that provider's model list.

`/think [on|off|default]` and `/effort [LEVEL|default]` use the same local
picker grammar and require an idle turn. They become real controls only when
the exact provider/model metadata can represent the request; a generic catalog
reasoning boolean remains fixed. The effective state, effort, and source are
visible in `/status` and `/context`, and a non-default override is shown in the
wide footer. These choices can affect latency, token use, and cost.

`smith --resume` likewise opens a project-session picker before any host is
constructed. `smith --resume <session-id>` remains the explicit form, while
headless use without an ID is refused with a `smith sessions list` hint.
Session rows include the full ID, updated time, turn count, provider/model, and
a bounded preview when known.

`/agent` lists retained child sessions with durability, child-session ID,
state, cumulative turns/tokens, and bounded incompatibility. `/agent resume
<child-id>` is intentionally explicit and has no Enter-default confirmation;
it resumes the exact interrupted turn rather than consuming a new child task
slot.

For runtime changes Smith restores the normal terminal, saves the current
session, re-runs complete preflight through `smith-runtime`, and resumes the
same identity. It never swaps an immutable runtime during a turn and the local
pickers never contact a provider.

### Provider model catalogs

`/model` is not limited to model IDs copied into TOML. Smith ships a reviewed,
generated Models.dev snapshot and augments configured providers only when an
available OpenAI-compatible adapter has one of these exact normalized
endpoints:

| Configured endpoint | Catalog |
| --- | --- |
| `https://api.openai.com/v1` | Models.dev `openai` |
| `https://openrouter.ai/api/v1` | Models.dev `openrouter` |
| `https://api.z.ai/api/coding/paas/v4` | Models.dev `zai-coding-plan` |

The configured provider name remains authoritative. A provider named `router`
at the OpenRouter endpoint therefore exposes choices such as
`router/openai/gpt-5.2`; a provider merely named `openrouter` at another
endpoint inherits nothing. Catalog data never creates a provider or imports
Models.dev endpoint, package, environment, header, or credential settings.

Startup uses a schema-validated last-good cache at
`~/.smith/cache/models-dev-v1.json`, falling back to the embedded seed when the
cache is absent, corrupt, truncated, oversized, or from the wrong origin. A
stale snapshot schedules a bounded credential-free refresh from exactly
`https://models.dev/api.json`; redirects, timeouts, bad status, responses over
8 MiB, and invalid metadata leave the current snapshot untouched. Publication
is atomic, and a successful refresh is visible only to a later host rebuild.
Deleting the cache safely restores the embedded seed.

Catalog rows show display name, provider-qualified ID, enforceable limits,
capabilities, revision/age, and `advertised` provenance. Deprecated rows are
omitted; entries without text output, tool calling, valid limits, or a usable
input budget remain searchable but disabled with a reason. Advertisement is
not entitlement—the provider can still reject a model for account, plan,
region, or rollout reasons.

Explicit `[models."<provider>/<model>"]` fields override catalog fields
independently. Smith's embedded trusted GLM metadata remains above the cached
catalog, and picker selection plus runtime preflight use the same frozen
snapshot so a background refresh cannot change an active session underneath
it.

For a real endpoint, set the provider kind to `openai-compatible`, add its
`base_url`, and choose exactly one credential source. A reference keeps the
key outside the config:

```toml
[providers.remote]
kind = "openai-compatible"
base_url = "https://provider.example/v1"
credential = "env:PROVIDER_API_KEY"

[providers.remote.response]
reasoning_only = "text" # only when this provider needs the compatibility policy
```

The explicit no-prompt user-config form is:

```toml
# ~/.smith/config.toml only; never a project .smith/config*.toml
[providers.remote]
kind = "openai-compatible"
base_url = "https://provider.example/v1"
api_key = "replace-with-the-provider-key"
```

The selected model must also resolve accurate `context_tokens`,
`max_input_tokens`, and `max_output_tokens` from explicit
`[models."<provider>/<model>"]` fields, trusted embedded metadata, or a
validated bound catalog. Smith refuses to guess missing limits.

### Live provider smoke test

The ignored `smith-cli` live test exercises the installed process, production
HTTP transport, streaming adapter, real `read` tool, continuation request,
provider-reported usage, clean shutdown, and credential redaction. It disables
persistence, allows no retries, explicitly auto-approves the isolated temporary
workspace, permits two tool rounds, caps each provider response at 2,048 tokens,
and has a 150-second outer deadline.

For a Z.AI Coding Plan account, export the API key as
`SMITH_LIVE_API_KEY` through a secret-safe shell or credential tool, then run:

```sh
SMITH_LIVE_BASE_URL=https://api.z.ai/api/coding/paas/v4 \
SMITH_LIVE_MODEL=glm-5.2 \
SMITH_LIVE_CONTEXT_TOKENS=1000000 \
SMITH_LIVE_MAX_INPUT_TOKENS=1000000 \
SMITH_LIVE_MAX_OUTPUT_TOKENS=131072 \
cargo test --all-features -p smith-cli --test live_provider -- --ignored --nocapture
```

The key is never written to project configuration. Unset
`SMITH_LIVE_API_KEY` after the run. Other OpenAI-compatible endpoints can use
the same test with their documented model limits.

The live direct ChatGPT Responses smoke test is ignored by default because it
requires explicitly injected test credentials and may spend provider quota. It
never reads Smith's Keychain entry. Have a secret-safe test runner inject
`SMITH_CHATGPT_TEST_ACCESS_TOKEN` and `SMITH_CHATGPT_TEST_ACCOUNT_ID`, then run:

```sh
cargo test -p smith-runtime live_chatgpt_responses -- --ignored --nocapture
```

Headless mode accepts a direct prompt or stdin and keeps machine stdout clean:

```sh
smith -p "review this change"
printf '%s' "run the checks" | smith -p -
smith -p "summarize" --output-format json
smith -p "inspect" --output-format stream-json
smith -p "continue" --resume session-...
```

`json` emits one schema-v3 result. `stream-json` emits versioned runtime events
followed by that result. Results project attempt commits/discards, the frozen
activation epoch, todo counts/items when public, artifact references, recovery
metadata, optional final goal state and continuation count, prepared approval
authority, and interaction-required state. A
forced live questionnaire—or a protected pending question on resume—returns
`interaction_required` with exit status 5 and never reads prompt stdin; resume
the same session interactively to answer the exact request. With the default
`approval.mode = "ask"`, an unattended mutation is denied and exits with status
4; explicitly choose `--approval allow-all` only in an already trusted
automation boundary. See [`docs/headless-protocol.md`](docs/headless-protocol.md)
for the complete stdout, schema, redaction, and exit contract.
`--yolo` is the explicit shorthand for `--approval allow-all`; it never adds
tools removed by a read-only profile, so `smith --profile plan --yolo` remains
read-only.
Repository-controlled config cannot set `allow-all` or `auto_approve` merely
by being opened; those authority-bearing choices must come from user config or
an explicit command-line policy. It likewise cannot redirect or disable
user-scoped snapshots and journals.

Snapshots and redacted canonical event journals live under
`~/.smith/sessions/<project-id>/`. Exact in-flight state uses a separately
encrypted, authenticated checkpoint; artifacts are owner-only and
session-authorized. Completed turns save immediately. On restart, unresolved
children and process-owned monitor markers are reported interrupted and are
never silently restarted. Both the TUI and `smith -p` use the same factory and
session lifecycle. See
[`docs/persistence-recovery.md`](docs/persistence-recovery.md).

## Design

[`DESIGN.md`](DESIGN.md) is the visual and interaction contract for the TUI —
layout, glyphs, color, focus, motion, and the rules that keep an estimated
number from looking like a reported one. Code comments reference its sections.

[`docs/GOAL.md`](docs/GOAL.md) defines the first release outcome, evidence
gates, runtime co-development boundary, and deliberately deferred work.
[`docs/ci.md`](docs/ci.md) documents the fail-closed local and hosted release
gates and the exact Agent Runtime source they require.
[`docs/configuration.md`](docs/configuration.md) is the typed configuration
reference, and [`docs/security.md`](docs/security.md) records Smith's threat
model and trust boundaries.

## Specification

The active harness integration change lives under
[`docs/spec/changes/integrate-stable-session-harness-2026-07-31/`](docs/spec/changes/integrate-stable-session-harness-2026-07-31/).
Its approved proposal and implementation checklist define the session-scoped
turn pipeline, prepared authority, protected recovery, capability activation,
and standard Smith harness components.

The installable package is expected to be `smith-cli`, exposing the `smith`
binary. Registry names must be rechecked immediately before publication.
Licensed `MIT OR Apache-2.0`.
