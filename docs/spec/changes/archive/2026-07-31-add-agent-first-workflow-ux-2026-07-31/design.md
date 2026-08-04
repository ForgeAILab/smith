## Context

The completed stable-session integration gives Smith typed abilities,
attempt-scoped streaming, todos, direct children, exact approvals, artifacts,
and encrypted checkpoints. The current cold-start TUI nevertheless presents
almost only `Ask Smith to do anything` plus model/path/context status. A local
OpenCode 1.18.9 comparison made the active agent, model, commands, plan,
tools, gates, and delegated reviewer visible at the point of action. It also
demonstrated trade-offs Smith must avoid: a failed optional CodeGraph tool,
project-local `.omo/` state, broad auto-approval, and a reviewer silently
routed to another provider/model.

The same coding fixture exposed three product defects independently of visual
design: Smith's 8,192-token GLM-5.2 request ended before editing, its initial
activation omitted `read` and `edit` while enabling broad `shell`, and its
successful JSON result contained one `in_progress` todo. The passing retry
used 32,768 tokens and completed planning, editing, validation, and a same-
model child review.

## Goals / Non-Goals

### Goals

- Make the active agent mode and the next useful actions obvious from the idle
  composer without introducing permanent dashboard chrome.
- Let users attach files, invoke bounded child presets, and run a local shell
  command with compact keyboard-first syntax.
- Keep active plans, tools, gates, and children understandable during a turn
  and terminal at the result boundary.
- Preserve Smith's exact resource authorization, approval, redaction, replay,
  project-trust, and one-level delegation guarantees.
- Provide no-prompt encrypted checkpoint recovery with a deliberately chosen
  owner-only local or environment key.
- Turn the live coding benchmark into a repeatable product evaluation.

### Non-Goals

- Copy OpenCode source code, configuration formats, plugin architecture,
  branding, large logo, or unrestricted permission rules.
- Add split panes, mouse-required flows, an embedded editor, nested agents, or
  cross-provider child routing.
- Let `@file`, `@agent`, or `!shell` bypass preparation, policy, approval,
  workspace, deadline, cancellation, output bounding, or artifact offloading.
- Store transcript, child, timeline, or continuation metadata in the user's
  project checkout.
- Store checkpoint plaintext, silently weaken encryption, or select a
  plaintext key source without an explicit user choice and warning.
- Make arbitrary shell changes attributable or redoable when Smith cannot
  prove their exact patch.

## Decisions

### Agent identity and help live at the point of action

The existing no-permanent-header rule remains. When the composer is empty and
idle, the footer renders one identity row in priority order:

```text
build · zai/glm-5.2 · project:branch · ? ctx
```

Smith does not reserve a shortcut strip that appears and disappears around
typing. `?` from an empty composer and `/help` both append the same bounded
local command/composer guide without provider spend or canonical history.
Narrow terminals truncate low-priority path detail before agent, model,
activity, approval, or context honesty. During a turn the same area shows
activity rather than a decorative identity banner. Color remains supplemental.

Alternative considered: add an OpenCode-style large startup logo. Rejected
because it consumes transcript space and conflicts with Smith's established
single-composer, terminal-native design.

### Exit keeps interrupted drafts recoverable

The first `Ctrl+C` clears the composer and stores its exact non-blank draft in
bounded process-local recall history. `Up` restores the newest interrupted
draft and repeated `Up` walks older entries; `Down` returns toward an empty
composer. A second `Ctrl+C` within one second exits from idle or active work.
Any intervening key disarms the exit sequence, so recalling or editing a draft
cannot accidentally turn a later interrupt into a quit. `/quit` retains its
explicit idle-exit/live-work confirmation path.

The recall buffer is UI state only: it is not canonical conversation history,
is not persisted in checkpoints, and never reaches a provider unless the user
later submits the recovered draft.

### Agent modes are policy presets, not authority grants

Smith ships three root modes:

- `build`: normal coding workflow; mutation still requires resolved policy.
- `plan`: read-only inspection and planning; mutation abilities are absent.
- `review`: read-only change inspection and findings-oriented prompting.

`Tab` cycles modes only when the composer is empty, the runtime is idle, and
no overlay is open. A mode switch occurs at a safe session boundary and never
changes provider, model, credentials, project trust, or approval policy. User
configuration may add or reorder modes, but the effective tool view is always
the intersection of the mode and authoritative run policy.

Named child presets initially include `explore` and `review`, both read-only.
`@explore <task>` or `@review <task>` is an explicit user-requested depth-one
delegation. It shows provider/model, workspace posture, limits, and spend
confirmation before dispatch. Children inherit the parent provider/model in
this change.

Alternative considered: make every agent name a free-form prompt template
that can declare permissions or models. Rejected because repository text
could then masquerade as product authority.

### One typed `@` reference surface

Typing `@` at a token boundary opens one compact completion pane with labelled
`file` and `agent` entries. File entries come from a bounded workspace index,
respect ignore policy, and remain relative to the canonical project root. On
submission, each file attachment is prepared and authorized as an exact read;
bounded text enters a distinct provenance-bearing context fragment. Binary or
oversized content becomes a preview/artifact reference or a local error.

Agent entries resolve only registered host presets. The literal escape `@@`
sends one leading `@` without resolution. Unresolved references remain in the
draft and fail locally before provider spend.

Alternative considered: insert only path text and hope the model chooses to
read it. Rejected because the affordance would look like an attachment without
guaranteeing content or provenance.

### `!` is a direct prepared shell action

An input whose first non-whitespace character is one `!` becomes a local shell
action. Smith validates and canonicalizes it with `ShellTool::prepare`, applies
the same broad descriptor bound, authorization, approval, deadline,
cancellation, scheduler, output bound, and artifact store used for model-
requested shell calls, then renders the result as a local transcript block.
It does not create a provider request. `!!` escapes a literal leading `!` into
an ordinary user prompt.

The live TUI may show the bounded command the user just typed. Redacted events
retain only approved projections/fingerprints; exact resumable state stays in
the protected checkpoint.

Alternative considered: spawn the user's shell directly from the TUI.
Rejected because it would create a second execution and security path.

### Live todos stay anchored above the composer

The reducer maintains the latest replaceable public todo projection from
versioned runtime events. The renderer keeps at most five authored items in a
non-focusable pane immediately above the composer, where updates replace the
same rows instead of entering transcript history. A compact picker temporarily
replaces the todo presentation in that anchored pane without mutating the todo
projection; closing it restores the todo and removes the temporary picker
control row. Sensitive projections retain counts
internally but render no item pane. `/details` may add bounded tool lifecycle detail beneath the quiet
`Working… · time` transcript row; it never reveals protected arguments. Turn
termination commits no aggregate `work` evidence row.

The runtime/harness closes advisory plan state at the same terminal boundary.
Items the model did not explicitly finish become `cancelled` with a stable
`turn_ended_unfinished` reason; Smith does not silently mark work completed.
No successful or unsuccessful result may report pending or in-progress items.
Live reduction and journal replay produce the same todo projection and
committed result. The terminal todo remains visible until the next turn starts.

Alternative considered: a focusable todo/child side pane. Rejected because it
adds navigation and narrow-terminal complexity; the anchored composer pane is
bounded, read-only, and gives its rows back when no public plan exists.

### Activation and limits are product-profiled and evidence-tested

Initial retrieval gains a Smith coding-intent profile. Repository inspection
activates `list`, `search`, and `read`; explicit modification intent adds the
exact `edit` ability; broad `shell` activates only for command/build/test or
explicit shell intent; multi-step work adds todos; delegation language adds
the root `agent` tool. Authority is unchanged and protected capability search
remains available for intent misses.

Model catalogs/configuration distinguish the provider's absolute output limit
from Smith's per-request output budget. The cataloged Z.AI Coding Plan
`glm-5.2` product default is 32,768 tokens, bounded by the declared 131,072
model limit and overridable through normal provenance rules. A limit terminal
event discards uncommitted reasoning/text, emits a concise structured reason,
and terminalizes plan state.

### No-prompt checkpoint protection is explicit

Smith accepts a 32-byte checkpoint key from either a higher-precedence
`SMITH_CHECKPOINT_KEY` environment value or an explicit inline value under
owner-only user configuration. It is mutually exclusive with a protected
credential reference, forbidden in project configuration, redacted from all
display/provenance/debug/event/journal/snapshot output, zeroized in memory,
and validated before any checkpoint access.

`smith setup checkpoint-key` offers protected OS storage, environment
reference, or `Store in config (no prompts)`. The local option generates the
key with operating-system randomness, warns that same-user processes and
backups can read it, and publishes through the existing mode-`0600` atomic
user-config transaction. When local or environment storage is selected, Smith
MUST NOT initialize or query Keychain/Secret Service.

The checkpoint envelope remains authenticated-encrypted. Key rotation either
atomically re-encrypts every compatible checkpoint under an exclusive lease
or refuses without modification; it never abandons unreadable state silently.

Alternative considered: disable persistence whenever Keychain is unwanted.
Rejected because it avoids the prompt by removing the durable feature the user
asked to exercise.

### Timeline and redo reuse proven change evidence

`/timeline` appends a bounded local view of root turns, child runs, plan/gate
outcomes, and undo/revert transactions. Selecting a child opens a temporary
read-only inspector with previous/next/parent navigation; the root composer
remains the only persistent focus.

`/redo` applies only the newest successfully undone or reverted exact change
whose forward patch and expected pre-image remain valid. It previews the patch,
requires explicit non-default confirmation, applies atomically, and journals
the outcome. Ambiguous shell deltas are never made redoable by observation.

## Risks / Trade-offs

- `Tab` mode cycling is easy to discover but changes an existing idle-key
  behavior; limiting it to an empty idle composer prevents draft surprises.
- Direct file attachments consume context before the model asks for them;
  strict size/count budgets and explicit provenance keep that cost visible.
- A plaintext local checkpoint key protects checkpoint contents only from
  parties that obtain state without the user config. The warning and explicit
  choice make that weaker at-rest boundary honest.
- Direct shell convenience increases accidental-execution risk. The leading
  position rule, visible command, exact preparation, and non-default approval
  preserve a deliberate boundary.
- Agent modes and child presets can become configuration sprawl. The initial
  built-in set is intentionally small and cannot widen authority.
- Redo can become stale quickly. Exact pre-image validation deliberately
  favors refusal over convenience.

## Migration Plan

1. Capture current reducer, PTY, headless result, activation, and checkpoint
   fixtures before changing keys or presentation.
2. Add typed agent-mode and checkpoint-key configuration with redaction and
   fail-closed precedence tests.
3. Tune activation and model request profiles; fix plan/limit terminal state.
4. Add direct prepared file, agent, and shell host actions without changing
   canonical provider/tool security paths.
5. Extend the reducer/rendering for stable identity/footer rows, completion,
   anchored todos, explicit details, and child navigation at
   narrow/normal/wide widths.
6. Add timeline and exact redo over existing attribution/recovery records.
7. Run deterministic, PTY, replay, security, MSRV, dependency, and live Z.AI
   evaluations, including no-Keychain mid-turn resume.
8. Update design/configuration/security/headless documentation and reinstall
   the verified binary.

## Open Questions

None for proposal approval. User-defined write-capable child presets,
cross-provider/model routing, arbitrary key remapping, nested agents, and a
bidirectional headless interaction protocol remain follow-up changes.
