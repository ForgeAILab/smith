## Context

The named files are large for two different reasons. `app.rs`, `main.rs`,
`resolve.rs`, and `render.rs` combine several stable responsibilities in one
module; `factory.rs` has a large orchestration function whose stages are not
represented in its shape. A safe refactor must preserve the current public
paths and test behavior while avoiding a broad visibility expansion between
new sibling modules.

The baseline is commit `c900c24` (`feat(tui): add turn steering and input
queue`). The only unrelated working-tree entry at proposal time is the
untracked `add-file-backed-project-memory-2026-08-02` proposal, which is out of
scope and must remain untouched.

## Goals / Non-Goals

- Goals:
  - Give each extracted module one recognizable reason to change.
  - Keep existing public paths, types, signatures, and behavior stable.
  - Make the largest production functions reviewable as short orchestration
    over named stages.
  - Split tests by behavior while retaining access to private implementation
    only where the existing tests already require it.
  - Keep every extraction independently format-able, compilable, and
    reversible.
- Non-Goals:
  - Redesign application state, keyboard behavior, CLI commands, config
    precedence, rendering, provider construction, or runtime policy.
  - Introduce new crates, dependencies, traits, serialization formats, or
    abstraction layers merely to move code.
  - Make private implementation public to work around Rust module privacy.
  - Combine this change with project memory or another pending capability.

## Decisions

### Preserve the existing files as facades

Existing imports such as `smith_tui::app::App`,
`smith_config::resolve::ResolvedConfig`, and
`smith_runtime::factory::RuntimeRequest` remain valid. Facades declare private
child modules and selectively re-export the same public items; leaf modules
own implementation. Cross-module helpers use the narrowest viable visibility,
normally `pub(super)`, and no public symbol is added solely for extraction.

### Decompose `smith-tui::app` by state transition ownership

Target shape:

```text
app.rs                    public facade and re-exports
app/
  state.rs                App state, public action/resource/overlay types,
                          construction, and simple state queries
  pending_input.rs        prepared submissions, paste/image material,
                          steer/queue bookkeeping, and restoration
  reducer.rs              canonical EventEnvelope reduction, speculative
                          attempts, work/plan/child projection
  prompts.rs              approval/questionnaire FIFO, expiry, answer,
                          cancellation, and exit restoration
  input.rs                mouse/key routing, composer/history, escape/exit,
                          and transcript scrolling
  resources.rs            palette commands, runtime resource selection,
                          model/reasoning/profile choices, local results
  tests/
    mod.rs                shared fixtures only
    reducer.rs
    input.rs
    prompts.rs
    pending_input.rs
    resources.rs
    child_lifecycle.rs
```

The `App` remains an I/O-free reducer. The extraction does not replace it with
multiple state owners or duplicate pending-input state. Methods that must
coordinate across child modules remain `impl App` methods with
`pub(super)` visibility only where a sibling call requires it.

### Reduce `smith-cli::main` to routing

Existing `cli`, `headless`, `interaction`, `setup`, and `terminal` modules
remain. New private modules take the responsibilities currently embedded in
`main.rs`:

```text
main.rs                   process entry, top-level command routing, constants
runtime_host.rs           resolution-to-HostSession construction and restart
tui_driver.rs             interactive event loop and terminal event routing
local_command.rs          typed local commands and status/context rendering
submission.rs             file materialization, steer/turn dispatch, and
                          agent/review actions
resources.rs              runtime inventory, workspace/session entries, and
                          session picker
config_command.rs         readiness, resolve request, explain, and list flows
main_tests/               responsibility-based binary tests and fixtures
```

The host is still constructed only through `smith_runtime::host`; this split
must not introduce a second composition path. Shared structs move with the
stage that owns them rather than becoming a generic `util` module.

### Keep `smith-config::resolve` as the compatibility boundary

Target shape:

```text
resolve.rs                public facade, re-exports, resolve/inspect entry
resolve/
  types.rs                public request/result/error/resolved value types
  provenance.rs           Layer, Source, Sourced, SettingValue, overrides,
                          contributions, explanation
  load.rs                 discovery, TOML loading/errors, flattening, env and
                          textual value parsing
  agent.rs                profile selection, profile inheritance, agent and
                          child resolution
  provider.rs             provider/model/reasoning/context/limits,
                          persistence/approval/background validation
  tests/
    mod.rs
    provenance.rs
    load.rs
    agent.rs
    provider.rs
```

`resolve()` continues to apply layers in exactly the current order. Moving a
type does not change its visibility, serde representation, error text, or
source provenance. The facade re-exports every currently public item under the
same `smith_config::resolve::*` path.

### Split rendering by visual region, not widget type

Target shape:

```text
render.rs                 draw/draw_synced facade
render/
  layout.rs               surface geometry and anchored row budgets
  transcript.rs           transcript, Markdown, status/local-result lines
  composer.rs             composer, pending input, todos, hints, footer
  modal.rs                approval, questionnaire, palette, search, and
                          confirmation overlays
  helpers.rs              bounded wrapping/truncation helpers shared by two
                          or more regions
  tests/
    mod.rs
    transcript.rs
    composer.rs
    modal.rs
    layout.rs
```

Visual ownership follows screen regions so a feature normally changes one
renderer module. Existing snapshot strings and narrow-terminal behavior are
the compatibility oracle. A helper stays in its owner unless at least two
regions use it; there is no catch-all widget utility module.

### Stage the factory before splitting it physically

`factory::build` becomes short orchestration over private stages with typed
intermediate state:

1. validate host policy and prepare model/provider inputs;
2. prepare prompt, skills, memory, summary, and child routes;
3. prepare tools, abilities, delegation slot, and goal/todo components;
4. initialize checkpoint durability;
5. assemble `RuntimePolicy`;
6. configure and build `RuntimeBuilder`;
7. assemble delegation and the final `SmithRuntime`.

Stage structs own values currently threaded through the 431-line body and
must not retain raw credentials longer than the current construction path.
After helper extraction, a physical `factory/` split is allowed only if it
reduces cross-stage coupling without widening visibility. Public request,
policy, runtime, and error types stay at `smith_runtime::factory::*`.

## Risks / Trade-offs

- Rust privacy can tempt a broad `pub(crate)` expansion. The refactor uses
  `pub(super)` for sibling coordination and keeps public API diffs empty.
- Moving tests can accidentally change which `cfg(test)` imports exist. Each
  production slice compiles with and without tests before the next move.
- Large mechanical moves are merge-conflict prone. The implementation lands
  one file family at a time and does not mix semantic cleanup with movement.
- Module names such as `composer`, `transcript`, and `host` already exist at
  crate scope. New child modules use explicit `super`/`crate` imports, and CLI
  host construction uses `runtime_host` to avoid ambiguous paths.
- File size is a diagnostic, not the architecture. The review criterion is
  cohesive ownership and short orchestration; arbitrary micro-modules are not
  a goal.

## Migration Plan

1. Record the clean baseline and run focused tests before moving code.
2. Extract one responsibility at a time, preserving item bodies first; format
   and run the owning crate's tests after each responsibility.
3. Move and group tests only after their production module compiles.
4. Run public-path compile checks and workspace validation after each file
   family.
5. Rebase or pause if an approved capability starts editing the same source
   family; never resolve that overlap by silently dropping either behavior.

No data, configuration, or runtime migration is required.

## Open Questions

- None for approval. The optional physical split of `factory.rs` is decided
  after staged extraction using the narrow-visibility criterion above.
