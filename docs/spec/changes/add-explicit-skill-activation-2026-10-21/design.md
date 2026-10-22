## Context

- `smith-runtime/src/skills.rs` resolves layered skills into a bounded
  `SkillIndexEntry` catalog and lazy, digest-pinned abilities. `activatable`
  means admitted by discovery/trust; actual activation also depends on the
  session's capability scope and budget.
- `smith-cli/src/local_command/skills.rs` implements listing and trust only.
- `smith-client/src/commands.rs` owns the table shared by parsing, slash
  completion, `Ctrl+P`, and help. File commands already extend that table.
- The current pinned Agent Runtime exposes activation epochs and a read-only
  capability catalog. Its local-tool path does not route the protected
  `registry.activate` marker through the live-ability activation machinery, so
  invoking that marker as a local tool would not implement this feature.
- The shared builder already supports explicit ability pins and performs
  materialization, authorization, and budget admission. Smith already has a
  safe session rebind path for runtime selections.

## Goals / Non-Goals

- Make a detected skill directly discoverable and explicitly selectable by
  the user, with deterministic activation before its next provider request.
- Keep all instruction loading inside the existing skill authority boundary.
- Reuse Ratatui's current completion, picker, and notice components. There are
  no new visual tokens or component primitives; browser and React tooling do
  not apply to this terminal surface.
- Skill creation, installation, automatic trust, new discovery directories,
  skill-specific argument templates, and unloading instructions already in
  context are outside this change.
- Installed CLI context injection is outside the initial delivery; its
  existing `registry.search`/`registry.activate` MCP flow remains available.

## Decisions

### Direct slash entries

The user's selected interaction is `/<skill-name>`. Add a skill variant to the
existing shared menu/parser model, backed by the effective descriptor rather
than by a generated Markdown command file. Include the description and source
in existing row/detail fields and expose withheld entries with their reason.
Opening a menu or completing a name reads metadata only.

Preserve precedence: built-in command, existing file command, then skill alias.
Collisions are reported with `/skills use <name>` guidance. That qualified
invocation takes a skill name as data and therefore works even for a skill
whose name is an existing command. Skill descriptions and bodies cannot rename
or override commands.

Tab completes a name without executing it. Enter on a completed skill name
activates it. Enter with trailing task text activates first, then submits that
task. Empty arguments must never create a blank model turn. The transcript
keeps the typed invocation; the provider receives the task and canonical skill
instructions through their respective existing context lanes.

### Explicit activation through the factory

Resolve the name to the winning descriptor in the current session and check
its actual authorized capability standing, rather than treating an index row
as proof of permission. Carry that exact identity into a factory request for
the existing safe rebind, adding its registry id to the explicit pinned set.

The runtime's normal materialization must verify the discovered bytes and
apply activation authorization and context budgets. Finish that admission
before replacing the current session. An unknown, untrusted, edited, denied,
or oversized skill produces a local named error and leaves the previous
session usable. Do not silently skip a requested pin when the shared builder
filters unknown or scope-denied ids; Smith must validate explicit user intent
before declaring success.

Use the existing live activation epoch and checkpoint behavior after
admission. A repeat invocation is idempotent. Instructions remain active in
the session just as an agent-activated skill does; there is no new persistent
user configuration or second copy of skill state. Model/profile changes and
resume continue to revalidate the normal scope and content identities.

### Busy sessions and backend availability

Completion remains available while a turn runs, but applying a skill requires
an idle boundary. A busy invocation gets a local explanation without changing
the current provider stream, activation epoch, task queue, or skill trust.

An installed CLI owns its context, and a native activation epoch does not
deliver instructions into it. Mark this operation unavailable for that backend
with bridge guidance; keep its skills inspectable. Extend direct invocation
there only when an acceptance test can prove the instructions reached the CLI
through its authorized skill bridge.

### Alternatives considered

- A `/skill` picker was offered; the user chose individual slash entries.
- Expanding a skill into ordinary user prompt text would lose its canonical
  activation provenance and budget admission.
- Asking the model to find the named skill would retain the current
  nondeterministic activation behavior.
- A new upstream session activation API could avoid rebind, but the installed
  immutable runtime already supplies a usable pin/admission path.

## Risks / Trade-offs

- Large catalogs need the existing bounded completion viewport and filtering;
  the composer must remain usable at narrow terminal sizes.
- A name can be shadowed or collide with a command. Display the effective
  source and resolve the same identity again at invocation.
- Skills can exceed the instruction share even when their files are within
  discovery's byte limit. Explicit activation must report that refusal.
- Rebind can fail after selection. Delay the success notice and task
  submission until admission succeeds, retaining the previous session on error.

## Migration Plan

Add the new skill menu rows and `/skills use` grammar without changing the
existing listing, trust, built-in, or file-command routes. Update documentation
and fixtures alongside implementation. The active guides proposal can add
descriptors without changing this protocol.

## Open Questions

None for the initial provider-backed implementation. Installed CLI delivery
will require a separate, proven context-delivery design.
