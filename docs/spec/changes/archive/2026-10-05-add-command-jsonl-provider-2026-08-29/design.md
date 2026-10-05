## Context

Smith already resolves an open provider-kind string and constructs one
`Arc<dyn Provider>` in `smith-runtime::factory` for the TUI, `smith -p`, child
sessions, tests, and future embedding. Agent Runtime's new command-provider
feature supplies the neutral process mechanism: direct argv, cleared
environment, bounded stdin/stdout/stderr, one process group per provider
attempt, cancellation/deadline/drop cleanup, an adapter-owned decoder, and an
explicit version probe.

That mechanism intentionally does not define a vendor CLI protocol. Smith must
own one exact adapter and its host policy. Open Forge's Codex integration is a
useful process-supervision and JSON-RPC reference, but it drives Codex
app-server as an autonomous executor with its own thread and turn. The official
Codex contract likewise describes app-server as the owner of authentication,
conversation history, approvals, sandboxed command/file work, MCP, and agent
events. It is therefore not the direct model transport this change needs.

## Goals / Non-Goals

### Goals

- Make a trusted local model bridge selectable through the same provider/model
  configuration and runtime factory as native adapters.
- Keep Smith's canonical tool loop fully functional, including connected MCP
  tools, without giving the bridge authority to execute them.
- Define one small, versioned, deterministic wire protocol that can be
  implemented by local shims in any language.
- Fail before terminal entry or inference when the executable, protocol,
  selected model, capability contract, or process policy is incompatible.
- Reuse Agent Runtime's process lifecycle and normalized provider contracts
  instead of creating Smith-local supervision or events.

### Non-Goals

- Make arbitrary human-oriented stdout into a provider stream.
- Treat an autonomous coding agent as a stateless model.
- Preserve a hidden child conversation between attempts.
- Expose every Agent Runtime provider feature in protocol revision 1.
- Discover executables or infer trust from an installed command name.

## Decisions

### Add one exact adapter kind

The stable provider kind is `command-jsonl`. It means the executable implements
`smith-command-provider` revision 1; it does not mean "run these arbitrary
arguments and hope stdout looks useful." Smith implements a trusted
`CommandAdapter` and `CommandOutputDecoder` for that revision, while Agent
Runtime owns `CommandProvider` and process supervision.

The provider advertises only the selected configured model. Revision 1 has a
fixed provider-local capability record:

| Capability | Revision 1 |
| --- | --- |
| Streaming text | required |
| Smith/MCP tool calling | required |
| Usage reporting | required |
| Reasoning | unsupported |
| Structured output | unsupported |
| Input/output modality | text only |
| Prompt cache evidence/control | unsupported |
| Server-side continuation | unsupported |
| Authentication | custom local command |

This provider-local record participates in the existing model-profile
resolution and narrows catalog claims. Model context and output limits still
come from Smith's ordinary explicit or trusted catalog sources. A request for a
feature absent above fails through the standard capability preflight before
the process is spawned.

### Keep process settings inside the provider declaration

A representative user configuration is:

```toml
[profiles.local]
provider = "local-bridge"
model = "local-model"

[providers.local-bridge]
kind = "command-jsonl"

[providers.local-bridge.command]
executable = "/Users/me/bin/model-bridge"
args = ["serve-smith"]
cwd = "workspace"

[providers.local-bridge.command.env]
BRIDGE_HOME = "/Users/me/.model-bridge"
BRIDGE_TOKEN = "env:MODEL_BRIDGE_TOKEN"

[models."local-bridge/local-model"]
context_tokens = 32768
max_input_tokens = 28672
max_output_tokens = 4096
```

`executable` is required and must be an absolute, canonicalizable executable
file. Smith does not search PATH. `args` are fixed, bounded, non-secret argv.
`cwd` is either the exact token `workspace` (the default) or an absolute
directory. Environment values use the same typed literal-or-credential-
reference representation and secret-resolution/redaction path as MCP process
environment values. The process framework clears the ambient environment and
receives only these resolved names and values.

For revision 1, every winning `kind = "command-jsonl"` and `command.*` process
field must come from owner-controlled user configuration. Future explicit CLI
or per-session authority may be added without changing this rule. A project
profile may select the already declared provider/model pair, but project or
project-local configuration cannot add or override any process field. Smith
rejects that composition before resolving credentials or starting a process.

HTTP-only provider keys (`base_url`, top-level provider credentials,
credential pools, rotation, headers, and response normalization) are invalid
for `command-jsonl`; secrets intended for the bridge are named in
`command.env`. Conversely, a `command` table on a native provider is invalid.

### Use a fixed preflight handshake

Before constructing the runtime, Smith explicitly calls the framework's
bounded preflight operation. The adapter appends
`--smith-provider-probe <selected-model>` to the configured fixed arguments.
The executable must write exactly one bounded JSON object to stdout and exit
successfully:

```json
{
  "protocol": "smith-command-provider",
  "schema_version": 1,
  "model": "local-model",
  "implementation": "example-bridge",
  "implementation_version": "1.2.3"
}
```

The protocol name, schema revision, and exact selected model are compatibility
gates. Implementation name/version are bounded redaction-safe diagnostics, not
capability authority. Unknown fields, extra output, malformed JSON, timeout,
non-success exit, or a mismatch fails startup before terminal entry and before
an inference attempt. Configuration loading and static inspection never run
the probe.

### Send one attempt envelope on stdin

For each `Provider::stream` call, the adapter appends
`--smith-provider-attempt` to the fixed arguments and writes exactly one JSON
object followed by a newline to stdin. The envelope carries
`protocol = "smith-command-provider"`, `schema_version = 1`, the typed attempt
purpose, and a Smith-owned projection of the canonical `ProviderRequest`:

- selected model;
- ordered system, user, assistant, and tool messages with text, tool calls, and
  tool results;
- ordered tool schemas and tool-choice policy;
- sampling values, maximum output tokens, and stop sequences.

Revision 1 rejects images, reasoning requests/content, structured output,
cache identity/boundaries, and non-null vendor extensions before spawn. The
projection is defined by Smith protocol structs rather than serializing a
runtime struct wholesale, so an upstream additive field cannot silently alter
the external wire contract. Prompt, history, tool results, and secret-bearing
content stay off argv.

Each process represents exactly one visible runtime attempt. It cannot resume
a hidden session, retry inference, execute a tool, or start another model
turn. A runtime retry or post-tool continuation starts a new process with a new
attempt ID and a complete canonical request.

### Decode only revisioned machine frames

Stdout is newline-delimited JSON. Every line contains the protocol name,
schema revision, and one of these frame types:

- `text_delta { text }`;
- `tool_call_delta { index, id?, name?, arguments_fragment }`;
- `usage { input_tokens, output_tokens, ... }` using the shared disjoint
  counter meanings;
- `finish { reason }`;
- `error { kind, message, retryable }` with a bounded safe classification.

The decoder rejects reasoning, cache, rate-limit, vendor-metadata, and unknown
frames in revision 1. It also rejects invalid UTF-8/JSON, unknown fields,
oversized content, invalid tool fragments, negative/overlapping usage, missing
required usage, and data after a terminal frame. Agent Runtime independently
enforces exactly one terminal event, bounded aggregate output/stderr, successful
exit after finish, and process-tree cleanup. Raw stderr is drained and
discarded; a bridge reports user-visible failures through the machine error
frame.

### Keep MCP and all tools in Smith

Connected MCP tools and Smith built-ins already become canonical tool schemas
before provider I/O. The command request projection carries those schemas to
the bridge. A decoded `tool_call_delta` becomes the ordinary normalized
provider event; Agent Runtime assembles and validates it, Smith's existing
authority and approval paths execute the selected built-in or MCP tool, and a
later command-provider attempt receives the canonical tool result.

No MCP server configuration, credential, process, approval, or result side
effect is delegated to the bridge. This is what makes a command provider and a
native HTTP provider interchangeable above the provider boundary.

### Compose through the existing factory

`smith-config` adds strict file and resolved command types plus provenance and
source-authority validation. `smith-runtime` adds the adapter, decoder, command
provider construction, preflight mapping, provider-local capability record,
and redaction-safe factory errors. The existing `Adapter` mapping gains one
variant and `AVAILABLE_ADAPTER_KINDS` gains one string.

The resulting `Arc<dyn Provider>` continues through the existing response,
reasoning, credential-pool, retry, summary, cache, runtime, journal, TUI, and
headless composition. Inapplicable HTTP wrappers and credential rotation are
not constructed. Cache capability remains unsupported, so adaptive synthetic
cache work is narrowed off through the ordinary capability policy.

Smith pins the exact compatible Agent Runtime revision and enables
`command-provider` on `smith-runtime`'s facade dependency. Released manifests
remain Git/revision based; a sibling path remains only an ignored local Cargo
patch.

## Risks / Trade-offs

- Revision 1 is a bridge protocol, not direct support for popular autonomous
  coding CLIs. The narrower claim preserves one owner for tools and history and
  gives future vendor adapters a conformance target.
- A local executable can read data and use OS authority outside Smith's tool
  approval path. Restricting its declaration to owner-controlled settings,
  clearing its environment, requiring an absolute path, and documenting that
  it is trusted code reduce accidental authority but do not sandbox it.
- One process per attempt has startup cost. It gives cancellation, retry
  attribution, and hidden-state behavior a simple auditable boundary.
- Required usage reporting excludes bridges that expose only text. This keeps
  Smith's accounting honest; a later named text-only revision can explicitly
  advertise `usage = false` if there is demand.
- The active Gemini change also edits provider config/factory files. Additive
  delta requirements do not conflict semantically, but Stage 2 must preserve
  its existing fields and tests.

## Migration Plan

- Existing configurations are unchanged.
- A command provider is inert until a user explicitly declares and selects it.
- Rollback removes the provider kind, command table, feature enablement, and
  exact dependency update; no persisted session schema changes are required.
- Sessions may switch between native and command providers only through the
  existing safe turn-boundary runtime replacement. Cache identity does not
  transfer.

## Open Questions

- None for revision 1. Vendor-specific autonomous CLI backends and richer
  command protocol revisions require separate proposals.

## References

- Agent Runtime command-provider contract:
  `../agent-runtime/docs/command-providers.md`
- Open Forge Codex executor reference:
  `../open-forge/crates/cli-adapters/src/codex.rs`
- Official Codex app-server contract:
  https://developers.openai.com/codex/app-server/

## Dependency compatibility review (2026-09-05)

Runtime main writes manifest vocabulary version 2 and has removed the semantic
summary APIs that Smith still consumes. The previous ignored Cargo patch mixed
that newer core with Smith's older facade, causing the snapshot mismatch while
hiding the API removal. A full runtime-main adoption requires a separate LCM
migration; changing the fixture alone would not establish compatibility.

The compatible dependency branch instead backports command providers and
hardened fetch onto Smith's existing e72390a baseline. Every runtime package
resolves to one immutable Git revision. Run manifests, the outer snapshot, and
listing metadata remain version 1. The original `session-snapshot-v1.json`
fixture stays unchanged. A load/re-save regression verifies complete history,
identity counters, usage, manifest versions, and audit fingerprints survive.
No claim is made about loading LCM records written by the experimental mixed
local-patch build, or equivalent replay across changed policy revisions.
