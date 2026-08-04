---
created_at: 2026-08-01T17:26:57Z
updated_at: 2026-08-01T17:26:57Z
---

## Context

Smith already composes stable product instructions and dynamic agent, skill,
memory, and project-context fragments through Agent Runtime. The standard
factory currently creates `DynamicPromptContext` with only `agent_mode`, so an
`AGENTS.md` file is never discovered or supplied. The TUI independently times
an active turn with a monotonic `Instant` and turns successful terminal events
into local transcript notices, even though the same runtime envelopes are
persisted by the host journal.

This change keeps both concerns at their existing ownership boundaries:
project-file discovery is host policy, prompt placement/revisioning remains in
the shared context plan, and successful-terminal suppression is only a TUI
projection decision.

## Goals / Non-Goals

### Goals

- Make root `AGENTS.md` guidance available by default in standard interactive
  and headless Smith runs.
- Keep one immutable instruction view during a runtime so background file
  edits never rewrite an active model context.
- Preserve exact prompt and cache identity when a later runtime observes
  changed instructions.
- Give children the same instruction snapshot as their parent.
- Remove routine successful terminal noise without hiding failures or deleting
  canonical lifecycle evidence.

### Non-Goals

- Recursive parent-to-child instruction discovery or nested override files.
- Watching the filesystem or automatically refreshing an active runtime.
- Treating repository text as permission, approval, or executable project
  trust.
- Expanding `@/...` references inside `AGENTS.md` automatically.
- Changing Agent Runtime events, provider protocols, machine output, or the
  duration-rendering helper used for active and non-success states.

## Decisions

### The standard host captures one root instruction snapshot

Before provider construction, session start, or terminal entry, the standard
host resolves the canonical project root and examines only:

```text
<canonical-project-root>/AGENTS.md
```

An absent file yields no fragment and no warning. A present file must be a
regular non-symlink, remain inside the canonical root, be valid UTF-8, and be
at most 32 KiB. Unreadable, non-regular, non-UTF-8, or oversized content fails
preflight with an actionable path-specific diagnostic; Smith never silently
uses a partial instruction file.

The result is an immutable snapshot containing the bounded body, canonical
project-relative source label, and content digest. `HostSessionRequest` (or an
equivalent host-owned input prepared before `factory::build`) carries that
snapshot into the one runtime composition path. TUI and headless startup use
the same loader. Direct embedders remain deterministic and opt in by supplying
an already validated snapshot; the runtime factory does not perform ambient
filesystem I/O.

An explicit complete `system_prompt` override retains its existing meaning and
does not receive an implicit `AGENTS.md` fragment. Standard Smith composition,
where no complete override is supplied, includes the snapshot.

Alternative considered: let `smith-runtime::factory` read the workspace
directly. Rejected because it would make an otherwise explicit composition
depend on ambient mutable I/O and could drift between tests, hosts, and
children.

### Activation guides behavior but never grants authority

Default discovery is a deliberate Smith host activation of declarative project
guidance. The body enters the instruction lane with a source label making clear
that it came from `AGENTS.md`. It may narrow or guide how the agent works in the
repository, but it cannot add tools, expand registered abilities, widen
workspace containment, change approval mode, approve an action, expose a
credential, or activate an executable extension/hook/setting. Higher-priority
Smith safety and host policy remain authoritative.

No automatic include syntax is interpreted. For example, a line naming
`@/docs/spec/AGENTS.md` remains guidance the agent can follow through an
authorized read; the host does not recursively splice that file into privileged
context.

Alternative considered: require an executable-project trust prompt for every
`AGENTS.md`. Rejected because the file is bounded declarative guidance, not an
execution mechanism or authority grant. Exact prepared authorization still
mediates every side effect.

### Project instructions are independently revisioned

The prompt layer represents the snapshot with a dedicated fragment such as
`smith.prompt.project-instructions`, `DeveloperInstruction` kind, host-activated
project provenance, a revision derived from its source identity and content,
and a stable cache class for the lifetime of the runtime. It is required when
present rather than an optional retrieval/memory item. The existing generic
`project_context` slot remains a separate optional retrieval contribution.

Smith's built-in identity, workflow, trust, inspection, tool, verification,
approval, and response fragments retain their own fixed revisions. When the
same snapshot is used, the prompt plan and child policy fingerprint are stable.
When a newly constructed runtime observes changed `AGENTS.md` content, the
project-instruction revision and exact overall prompt/cache identity change,
as correctness requires; Smith must not report reuse under the old identity.
Providers with multiple supported prefix breakpoints may reuse an unchanged
earlier prefix, but Smith does not promise or infer that reuse.

The runtime policy/diagnostic projection records the source label and digest or
revision, never a fabricated cache hit. The canonical transcript does not copy
the raw body as a user message.

Alternative considered: concatenate `AGENTS.md` into Smith's built-in system
text. Rejected because any repository edit would disguise itself as a product
prompt revision and make cache/provenance diagnostics less precise.

### An active runtime never watches or reloads the file

The snapshot is read once for each constructed hosted runtime and then cloned
into direct child factories. Child creation and follow-up do not reread the
filesystem, even when a child uses an explicit workspace, so one parent tree
cannot observe mixed instruction revisions.

Editing `AGENTS.md` while that runtime is active has no automatic effect. If a
user explicitly asks the agent to reread it, the agent may use its ordinary
authorized read path and reason from that user-requested tool result; this does
not silently replace the dedicated prompt fragment. A later runtime
construction, including a new process or explicit runtime rebuild, captures
the then-current file and revision. No watcher or `/reload-instructions`
command is added in this change.

On resume in a new process, the new runtime records the newly captured revision
alongside the resumed composition evidence. A changed revision is visible and
creates a new exact cache identity rather than pretending byte-for-byte prompt
continuity.

Alternative considered: check the file before every turn. Rejected because it
would mutate developer context without a conversational boundary, cause
surprising cache churn, and let a concurrent checkout edit alter an in-flight
session.

### Successful turn terminals are transcript-silent

`RuntimeEvent::TurnCompleted` remains the sole canonical terminal evidence.
The TUI reducer always cancels obsolete prompts, discards orphan speculative
output, closes open transcript streaming, clears the monotonic active-turn
timer, returns activity to idle, and reconciles work/todos exactly as before.

For `TurnFinish::Completed`, it appends no `turn` notice, regardless of
`visible_output`. A visible answer is already present; tool/reasoning-only work
has its committed tool/result projection; and a truly empty success may simply
return to the idle composer. This removes both `completed in ...` and
`completed in ... without a visible answer (reasoning only)` decorations.

Cancelled, limit-reached, needs-input, and failed finishes retain concise
visible notices, including elapsed time when locally available. Separate error,
approval, tool, usage, and downgrade rows are unchanged. The journal continues
to store the canonical start/completion envelopes and timestamps, from which
timeline or diagnostic tooling can inspect lifecycle and derive duration
without duplicating it into the conversational transcript.

Alternative considered: render sub-second success as `<1s`. Rejected because
the user's concern is routine per-turn noise, not merely the rounding format.

## Risks / Trade-offs

- Root-only discovery is smaller than Codex-style hierarchical instruction
  lookup. The explicit non-goal prevents accidental precedence semantics and
  leaves room for a separate nested-instructions change.
- Failing on an invalid present file can block startup. This is preferable to
  silently claiming project guidance is active when it was skipped or
  truncated.
- A changed file is not automatically honored in an already running session.
  That immutability is intentional; the user can request an ordinary reread or
  start/rebuild a runtime.
- A changed project fragment necessarily changes the exact full cache identity
  on the next runtime. Separate revisions preserve attribution and potential
  prefix reuse, not an unsafe promise that semantically different prompts share
  one cache key.
- A successful turn with no visible answer becomes visually quiet. Tool rows,
  status/timeline commands, and canonical event evidence remain available; if
  later usability testing shows a true empty success is confusing, it should
  receive a distinct actionable invariant rather than restoring a notice for
  every successful turn.

## Migration Plan

1. Capture current prompt fragments, policy fingerprints, standard host
   preflight, child composition, and successful/non-success TUI terminal
   fixtures.
2. Add the bounded snapshot loader and explicit runtime request field, then
   wire both standard surfaces through it before factory construction.
3. Add the dedicated prompt fragment and child inheritance, preserving complete
   prompt override behavior and exact cache fingerprints.
4. Suppress only successful terminal notices and update reducer/replay/render
   fixtures at narrow, normal, and wide widths.
5. Update `DESIGN.md`, security/context documentation, run formatting, Clippy,
   workspace/all-feature tests, strict spec validation, and diff hygiene.

## Open Questions

None for proposal approval. Hierarchical lookup, override filenames, automatic
reload, and a local reload command remain explicit follow-up decisions.
