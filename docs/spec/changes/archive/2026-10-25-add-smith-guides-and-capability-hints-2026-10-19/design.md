## Context

Built-in skills already flow through `SmithSkillSources::with_built_in` from
`built_in_skills.rs`. Reference documents register as an outline plus one
skill per `##` section, because whole documents are too large for the shared
capability budget. Modules report `Mounted::Inactive { reason }`, which
carries no contributions, so an inactive module cannot add a context fragment
today. Config resolution maps both an unset `vision` and `vision = false` to
`None`.

## Goals / Non-Goals

- Goals:
  - The agent can find and activate a short, correct how-to for driving
    Smith, in any workspace and offline.
  - The agent learns when a fixable capability is off, what fixes it, and
    which of the user's own profiles would work.
  - The user stays in control: nothing gets turned on automatically.
- Non-Goals:
  - Picking a vision model automatically.
  - A `smith config set` command or a live config reload.
  - Letting guides grant any tool, permission or approval.

## Decisions

- **Guides are whole-body skills, not outline-plus-sections.** Each guide is
  small enough to activate whole: a test enforces at most 1,200 estimated
  tokens per guide and 600 for `overview`. `overview` maps tasks to guides
  and lists the CLI surface (subcommands and slash commands) in one table.
  Alternative considered: more `##` sections in `configuration.md`. Rejected
  because sections describe keys, not tasks, and putting a task in a
  reference section breaks the reference structure.
- **Guides pre-activate only on authored keywords.** Pre-activation binds
  any skill whose keywords meet a prompt term, and derived keywords include
  every description word. A guide description saying "file" or "turn" would
  load on ordinary coding prompts. Each guide carries a `smith.keywords`
  metadata list of distinctive terms (`vision`, `view_image`, `advisor`, …),
  which replaces the derived terms. `registry.search` still matches the full
  description.
- **Guides state who acts.** Slash commands and interactive pickers are
  typed by the user. The agent may run read-only `smith` subcommands
  (`config explain`, `config modules`, `sessions list`, `plugin list`)
  through its shell tool, and may edit `config.toml` through the ordinary
  edit tool and its approval. Guides never ask the agent to handle a
  credential: `smith setup credential` and `/connect` stay with the user.
- **Hints live on `Mounted::Inactive`.** It gains an optional `setup_hint`
  with `missing` (a short safe phrase), `guide` (a built-in skill name) and
  `candidates` (profile names or `provider/model` references, never values or
  secrets). The composition gathers the hints of all inactive modules into
  one developer fragment, `smith.capability-hints`, revisioned by its
  content. Alternative considered: mounting the module with a
  guidance-only contribution. Rejected because `/modules` would then call the
  module active while it offers no tool, and each module would need its own
  fragment.
- **One hint, two renderings.** The fragment line is written for the agent:
  the gap, the candidates, and "offer to set `<key>` to one of these; suggest
  no other value". The `/modules` suffix is written for the user:
  `set up with <guide> · <candidates>`. In live testing without the key, the
  agent suggested a model from a documentation example that the user had
  not configured.
- **Image generation is usually `off`, not inactive.** Its switch defaults
  off for providers without an image binding, so the hint appears only when
  the user has turned image generation on for such a provider.
- **The hint tells the agent to raise the gap only when relevant**: when the
  user's request needs the missing capability (an image appears and cannot
  be seen, or the user asks for an image to be generated). It is not raised
  at session start.
- **`ModuleContext` gains `image_capable_profiles`**: the main-enabled
  profile names whose resolved model accepts image input, in `profile_order`
  order, excluding the active profile. These are computed from the frozen
  catalog that config resolution already uses, with no credential lookup.
- **Vision becomes tri-state after resolution**: `Unset`, `Disabled`
  (explicit `false`, with its source), or `Selected(target)`. Only `Unset`
  produces a hint. `config explain vision` prints `disabled by <source>` or
  `unset`.
- **Config edits apply on rebuild.** Every host rebuild (`/profile`,
  `/model`, `/resume`, a module or plugin switch) resolves the config files
  again, so the guides say a change applies on the next rebuild rather than
  in the next session.
- **The placeholder does not change.** The same placeholder is used for
  images outside the window on vision-capable models, where a setup hint
  would be wrong.

## Risks / Trade-offs

- The guides can drift from behaviour. Mitigation: guides are compiled from
  repository docs, a test checks that every command and key a guide names
  exists (CLI help, the slash-command registry, the config schema), and the
  release checklist reviews `docs/guides/`.
- The fragment costs a few tokens on every turn while a hint is active. It
  stays small: one line per hint, with at most 5 candidates.

## Migration Plan

Additive. Existing sessions see the hint fragment on their next composition.
No checkpoint or persisted state changes, so the checkpoint revision
constants stay as they are.

## Open Questions

- Resolved: `/modules` shows the hint, in its short user-facing form.
- `smith.guide.overview` pre-activates on the bare word `smith`. That is
  cheap (about 430 tokens, once per session) but binds in almost every
  session inside the Smith repository. The owner may prefer `smith-guides`
  only.
