---
created_at: 2026-08-03T20:21:57Z
updated_at: 2026-08-03T20:21:57Z
---

# Design

## What the comparable harnesses actually do

| Concern | `claude-code` | `codex-rs` | Deep Agents v0.7 | Smith today |
| --- | --- | --- | --- | --- |
| Whole-file write | `Write` tool, overwrites | `apply_patch` `*** Add File` | `write_file`, overwrites (v0.7 changed it from erroring) | none |
| Partial edit | `Edit`, exact string, unique | `apply_patch` `*** Update File` | `edit_file` | `edit`, exact string, unique |
| Delete | **none** — `Bash rm` | `apply_patch` `*** Delete File` | `delete` tool (new in v0.7) | **none** — `shell rm` |
| Overwrite safety | must have read the file, full view, mtime ≤ read time | patch context must match | none | n/a |
| Planning tool | `TodoWrite`, always on | `update_plan`, always on | `write_todos`, **opt-in since v0.7** | `write_todos`, always on |
| Base tool count | large | 4 (`shell`, `apply_patch`, `update_plan`, `view_image`) | small | 5 + harness tools |

Two lessons survive the disagreement between them:

1. **The verb set matters more than the tool count.** Codex proves one tool can
   carry add/update/delete safely; Claude Code proves separate tools work too.
   Neither ships a whole-file write without an accompanying safety
   precondition, and Claude Code's is the strictest and the cheapest to
   implement.
2. **A destructive operation deserves a narrow permission.** Claude Code omits
   `delete` because `Bash` is already unrestricted in its model. Smith's
   `TOOL_USE` section explicitly forbids routing around a narrow denial with a
   broader tool, and `shell` carries spawn and network, so for Smith the codex
   answer is the correct one.

## Tool shape: extend `edit`, do not add tools

`edit` gains `operation: "replace" | "create" | "overwrite" | "delete"`,
defaulting to `replace` so existing callers and recorded journals stay valid.

```
edit { path, operation?, old_string?, new_string?, replace_all? }
```

| operation | requires | permissions | effects |
| --- | --- | --- | --- |
| `replace` | `old_string`, `new_string` | `FsRead`, `FsWrite` | write |
| `create` | `new_string` | `FsCreate`, `FsWrite` | write |
| `overwrite` | `new_string`, prior full read | `FsRead`, `FsWrite` | write |
| `delete` | prior full read | `FsRead`, `FsDelete` | write |

This keeps the base tool surface at five, which is the point of the exercise —
adding `write` and `delete` as peers would have grown base tokens while we were
trying to shrink them. It also extends a pattern already present: `edit`'s
`prepare` already computes a different `PermissionSet` for the create case.

Backwards compatibility: an empty `old_string` keeps meaning `create`, so
existing transcripts replay unchanged. `create` still uses the atomic
`create_new` open and still fails when the target exists — `overwrite` is the
operation that accepts an existing file, and it is separately gated.

`ChangeRecorder` already models deletion: `EditMutation::after` is documented as
`None` when the operation removed the file, and `recovery_path` exists for an
untracked removal. Delete therefore lands inside the existing undo and
change-review machinery rather than beside it.

## Read-before-overwrite

`overwrite` and `delete` destroy content that no `old_string` proves the model
has seen. The precondition is Claude Code's, with the same three parts:

1. the session read this exact canonical path,
2. the read was a **full** view — an `offset`/`limit` read does not qualify,
3. the file's `mtime` is not newer than that read.

Part 3 is what catches a concurrent external change; without it an overwrite
silently clobbers work the user did in their editor between the read and the
write. Smith's exact-match `replace` gets this property for free (a stale
`old_string` simply fails to match), which is precisely why the new operations
need it stated.

Storage is a `ReadRecorder` — an `Arc<Mutex<HashMap<PathBuf, ReadObservation>>>`
sibling to `ChangeRecorder`, recorded in the existing `ObservedTool` wrapper so
no tool becomes stateful and the plumbing already exists.

## Conditionality must not perforate the stable prefix

`agent-runtime-context/src/cache.rs` defines the stable prefix as **the longest
leading run of `CacheClass::Stable` segments** in canonical plan order. Smith's
ten instruction sections are positions 0–9 in `ContextLane::Instructions` and
all carry `CacheClass::Stable`; the dynamic `agent-profile` fragment sits at
position 11 and carries `CacheClass::Ephemeral`. Position order and cache class
currently agree, which is why the head is a hit on every turn.

Gating `QUESTIONNAIRE` (position 7) and `DELEGATION` (position 8) in place would
break that in one of two ways, both worse than today:

- left as `Stable`, their content varies with posture, so a mid-session switch
  makes the prefix comparison fail at position 7 — the cached run collapses
  from ten segments to seven;
- marked `Ephemeral`, the leading `Stable` run truncates at position 7
  **permanently**, for every session, including ones that never switch.

So the rule is placement by mutation frequency, not by topic: every
capability-gated section moves *after* the entire unconditional `Stable` run,
into one contiguous variable block alongside `agent-profile`. The head stays
byte-identical for every run, every posture, and every turn. General policy
first, capability specifics after, which is also the better authored order.

A test asserts the precise property the prefix computation depends on: **no
cache-stable segment may follow a non-stable one** in canonical order. That is
narrower than full monotonicity, which the `Memory` lane already violates
harmlessly by placing `NoCache` memory before `Ephemeral` project context.

### What a mid-session switch still costs, and why that is acceptable

`/profile` rebuilds the runtime and resumes the same session
(`runtime_host.rs` sets `resume = Some(current_session)`), so history survives
while the composition changes. Two costs are unavoidable and must not be
engineered around:

- **The tool array.** Plan → Build adds `edit` and `shell`. Tool definitions
  precede messages in every provider request, so the change invalidates
  downstream cache no matter where the prose sits.
- **The plan identity.** `apply_palette_command` clears `selection.provider`
  and `selection.model` on a profile switch, so the profile re-resolves them.
  `CachePlan::identity` is the resolved model profile fingerprint, and a
  changed identity invalidates the entire prefix by construction — correctly,
  since the bytes were produced for a different provider contract.

A deliberate switch is therefore one cache reset, in the same category as a
compaction boundary: rare, user-initiated, and worth *counting* rather than
avoiding. The session usage record tallies it alongside compaction windows.
What the ordering rule protects is the case that actually recurs — ordinary
turns, and a switch between two profiles that resolve to the same model.

## Context management: what the cache actually costs

Compaction rewrites history. Any deletion or rewrite at message position *k*
invalidates the provider prompt cache from *k* onward, and because compaction
is most effective on the *oldest* history, an effective compaction is also a
maximally destructive one for cache. The current 6-turn trigger therefore buys
a little slack at the price of a full cache miss, repeatedly.

Codex's model is the cache-honest one: compaction is a rare, explicit **window
boundary** (`start_new_context_window`), growth is measured against that
window's baseline rather than against total usage
(`AutoCompactTokenLimitScope::BodyAfterPrefix`), and before the boundary the
model gets a single appended reminder that costs nothing in cache terms because
appending never invalidates a prefix.

Smith adopts the same ladder:

| Tier | When | What it does | Cache cost |
| --- | --- | --- | --- |
| Floor | fewer than `min_turns` complete turns | nothing; there is nothing worth summarizing yet | none |
| Notice | remaining input budget ≤ `notice_threshold_tokens`, once per window | append a bounded in-context notice so the model can persist state before the boundary | **none** — appended after the cached prefix |
| Summary | post-prefix usage ≥ `trigger_fraction` × input budget | semantic summary, reduce to the low watermark, start a new window | one deliberate reset |

`trigger_turns` is not deleted; it is demoted to `min_turns`, an eligibility
floor. That preserves the honest half of the original intent — do not summarize
a two-turn session — while removing the half that fired paid model calls on a
clock instead of on pressure.

Defaults: `min_turns` 4, `notice_threshold_tokens` 12_000,
`trigger_fraction` 0.85. The fraction matches Deep Agents' default and sits
above the existing structural high watermark, so cheap structural compaction
still runs first and semantic summarization stays the last resort.

Measuring **post-prefix** usage matters for more than accuracy: the stable
prefix is `CacheClass::Stable` and is exactly the part we want cached and
untouched. A trigger that counts it would fire earlier on runs with more
skills or project instructions activated — punishing precisely the sessions
whose prefix is most worth keeping cached.

## Usage reporting and analytics

The runtime already carries `UsageRecord`/`UsageDelta` with per-counter
`Confidence`, and `smith-tui/src/status.rs` already renders counts with
provenance. Two additions:

- On exit, the TUI prints one summary line per counter kind plus the session
  total, reusing `compact_tokens` and the existing reported/estimated marker so
  a derived number is never shown as a provider-reported one.
- Each session appends one bounded JSON line to a session-usage log under user
  state: session id, model, turns, per-counter totals with confidence,
  compaction-window count, and notice/summary trigger counts. Path metadata
  only — no prompt text, no tool arguments — so it inherits the journal's
  existing redaction guarantees rather than opening a new leak surface.

The compaction counters are the point: without them there is no way to tell
whether a threshold change helped, which is how the current 6-turn default
survived unexamined.

## Base token budget test

`stable_fragments()` plus `smith_tools::all()` specs plus the harness tool
specs are all deterministic, so their assembled size is a pure function. The
test asserts it stays under an authored ceiling and prints the per-section
breakdown on failure. The ceiling is a reviewed constant, not a measurement —
raising it is the intended way to accept growth.
