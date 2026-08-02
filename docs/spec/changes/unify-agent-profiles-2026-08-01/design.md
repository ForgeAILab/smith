---
created_at: 2026-08-01T21:41:00Z
updated_at: 2026-08-01T22:49:15Z
---

## Context

The current file model contains `ProfileSection`, `AgentModeSection`, and
`ChildAgentSection`. A profile already controls provider, model, output budget,
context, loop limits, approval, and background policy, but delegates its
behavioral posture to a root-only mode name. Child presets carry only posture
and description. At runtime, the main agent receives a host-authored mode
fragment while a composer-launched child gets an ad hoc task prefix and
inherits the main agent's provider, model, prompt contributor, and limits.

This means the object called a profile is not yet an agent profile, while the
objects that resemble agent presets cannot be reused across placements.

## Goals / Non-Goals

### Goals

- Define one named profile that can configure either the main agent or a
  direct child.
- Let profiles carry bounded instructions, posture, model/runtime preferences,
  limits, and presentation metadata.
- Keep shared settings reusable without copying a full provider/model block
  into every behavioral preset.
- Preserve deterministic layering, provenance, setup, status, cache identity,
  persistence, and safe-boundary rebuild behavior.
- Ensure profile configuration can only select or narrow authority already
  granted by the host.

### Non-Goals

- Letting profile text replace Smith's complete system prompt.
- Granting tools, permissions, trust, credentials, approval, or a larger
  workspace through a profile.
- Enabling write-capable children, nested delegation, remote profiles,
  executable hooks, or arbitrary prompt include expansion.
- Changing provider/model catalog declarations or moving secrets into a
  profile.

## Decisions

### Profiles become the only agent-preset declaration

`[profiles.<name>]` keeps its existing runtime fields and gains:

```toml
default_profile = "work"
profile_order = ["work", "plan", "review"]

[profiles.work]
provider = "zai"
model = "glm-5.2"
description = "implementation with bounded mutation"
posture = "build"
use = ["main"]
instructions = """
Implement the requested change, verify it, and report concrete evidence.
"""

[profiles.plan]
extends = "work"
description = "read-only planning"
posture = "plan"
use = ["main", "child"]
instructions = """
Inspect the repository and produce an implementation-ready plan.
"""

[profiles.review]
extends = "work"
description = "independent review"
posture = "review"
use = ["main", "child"]
instructions = """
Report prioritized, evidence-backed findings without modifying the workspace.
"""
```

`use` controls discovery and eligibility, not authority. `main` makes the
profile selectable at startup or through `/profile`; `child` makes it visible
to explicit `@` child invocation. Omitting `use` on an existing profile
defaults to `main` for compatibility. Names remain bounded ASCII identifiers,
descriptions remain bounded display text, and instructions are bounded UTF-8
text with source provenance.

`posture` moves onto the profile. The built-in spellings remain `build`,
`plan`, and `review`; plan/review continue to impose read-only capability
views. This change removes a second user-defined mode registry rather than
adding arbitrary authority-bearing behavior types.

Alternative considered: add a new `[agents]` registry and retain runtime
profiles unchanged. Rejected because users would still have to combine two
named presets to describe one agent, and `/profile` would continue to mean
only provider/runtime selection.

### Single-parent inheritance reuses a runtime baseline

A profile may `extends` exactly one declared profile. Resolution performs a
bounded acyclic walk, overlays child fields on the parent, records provenance
for every effective field, and rejects missing parents, self-reference,
cycles, or excessive depth before provider or terminal construction. Profile
selection applies the fully expanded result as one existing profile-precedence
layer.

Single inheritance is sufficient for a provider/model baseline plus several
behavioral presets and avoids ambiguous multi-parent merge order. Inherited
`use` and instructions may be replaced explicitly; instruction bodies are not
implicitly concatenated. Smith can add explicit composition later if concrete
use cases justify its prompt and provenance semantics.

Alternative considered: require every profile to duplicate provider/model and
limits. Rejected because it makes plan/review variants drift from the primary
configuration. Multiple inheritance was rejected because overlapping policy
tables need a new conflict language and are unnecessary for the initial goal.

### Profile instructions are additive, isolated prompt fragments

The resolved profile contributes one `DeveloperInstruction` fragment after
Smith's stable identity/workflow/security fragments and separately from
project `AGENTS.md`, activated skills, memory, and retrieved project context.
The fragment includes the validated profile name, posture semantics, and the
optional instructions body. Its revision derives from the exact effective
profile behavior and source identity; raw instructions do not enter canonical
user history or ordinary status/debug output.

A profile cannot change instruction priority, fragment kind, source, or cache
class. A direct embedder's existing complete `system_prompt` override keeps its
explicit replacement semantics; ordinary config profiles never reach that
escape hatch.

Alternative considered: expose `system_prompt` as a complete replacement.
Rejected because a normal project profile could erase Smith's stable trust,
approval, verification, and tool-use guidance while appearing to be routine
configuration.

### Main and child placement share resolution but not authority

Main selection uses the existing profile precedence and safe-boundary rebuild:
startup `default_profile`/`--profile`, `/profile`, and eligible idle cycling all
resolve one complete main-enabled profile. Switching profile clears narrower
provider/model overrides as it does today and updates prompt, posture, and
profile revision atomically.

Child selection resolves a child-enabled profile through the same typed named
profile registry. It may choose another already-declared provider/model and
request narrower limits, but credential lookup and provider construction still
occur through the one Smith factory path. Confirmation shows profile,
instructions summary, provider/model, effective limits, workspace posture,
and provider spend before dispatch.

Effective child capabilities are:

```text
parent authority ∩ host direct-child ceiling ∩ profile posture
```

The host direct-child ceiling remains depth-one and read-only in this change.
Profile approval, trust, workspace, persistence, or background settings cannot
widen that ceiling; any inapplicable or widening child value is rejected or
narrowed with source-explainable diagnostics rather than silently treated as
authority. A different provider/model must pass normal declaration, credential,
catalog, context, and spend preflight before a child ID is allocated.

Alternative considered: keep children on the parent's provider/model even
when their selected profile names another. Rejected because that would claim
to apply a reusable agent profile while silently ignoring core configuration.
Allowing write-capable children was deferred because it changes the host's
authority ceiling rather than merely reusing profiles.

### Compatibility adapters are explicit and collision-safe

For one transition release:

- Existing `[profiles.<name>]` with no new fields remains a main-only profile
  with build posture unless its legacy `agent` selection narrows it.
- `[agent_modes.<name>]` is accepted as a deprecated main-only behavioral
  profile overlay for the existing UI path.
- `[child_agents.<name>]` is accepted as a deprecated child-only profile that
  inherits the active parent's provider/model and remains read-only.
- If legacy and new declarations claim the same effective name with different
  bodies or placement, resolution fails with both sources; it never chooses by
  map iteration or hides one preset.

Setup writes only the new profile shape. Config explanation, inventory, and
status identify legacy adaptation and give a concrete replacement snippet.
Removal requires a separately approved breaking change after the transition
release.

## Risks / Trade-offs

- Profile inheritance adds resolution complexity. A single parent, bounded
  depth, cycle detection, and field-level provenance keep the behavior
  deterministic and explainable.
- A child profile may select a different provider/model and therefore requires
  a more general child factory than the currently captured parent provider.
  Reusing the standard preflight/composition path avoids a second partial
  resolver but increases the implementation surface.
- Additive instructions cannot fully replace Smith's personality. That is
  intentional for normal configuration; direct embedders retain the existing
  explicit full override.
- Legacy adapters temporarily increase schema surface. Clear diagnostics and
  an explicit removal boundary prevent the compatibility layer from becoming
  the permanent conceptual model.

## Migration Plan

1. Capture current profile, root-mode, child-preset, prompt, inventory,
   persistence, and safe-boundary rebuild fixtures.
2. Add profile metadata, posture, instructions, placement, inheritance, and
   field-level provenance without changing existing main-profile behavior.
3. Move main mode prompt/capability composition onto the selected profile and
   route built-in/setup configurations through the new shape.
4. Generalize child preflight to select one child-enabled profile through the
   same resolver/factory path, then include profile identity in durable policy
   fingerprints.
5. Add compatibility adapters and diagnostics, update TUI/CLI selection,
   setup, docs, and examples, then run deterministic and live-gated validation.

## Open Questions

None for proposal approval. Multi-parent prompt composition, remote profile
distribution, write-capable children, and complete system-prompt replacement
remain explicit non-goals for later proposals.
