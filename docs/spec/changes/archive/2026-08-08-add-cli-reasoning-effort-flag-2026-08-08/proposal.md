---
created_at: 2026-08-08T04:55:00Z
updated_at: 2026-08-08T23:02:35Z
---

## Why

Reasoning effort is a per-task decision, but Smith only exposes it as a
per-deployment one. The layered resolver already understands `reasoning.effort`
at every layer, `/effort` already selects it inside a running session, and
`SMITH_REASONING_EFFORT` already addresses it from the environment. The one
place it cannot be said is the command line, so the only way to fix an effort
for one invocation is to declare a `[profiles.<name>.reasoning] effort` and then
select that profile — which turns a per-run knob into a config-deployment
artifact and multiplies otherwise-identical profiles by the size of the ladder.

The concrete driver is Forge, the separate workflow engine that dispatches
Smith headlessly and carries a per-agent `reasoning_effort` field. It has
nothing to pass that field to. An orchestrator that wants `low` for a triage
task and `high` for a hard refactor has to rewrite `~/.smith/config.toml` on
the host between dispatches, which is not something a workflow engine should be
doing. This change is motivated by that use, not shaped by it: nothing here is
Forge-specific, and Forge gains no privileged path.

`SMITH_REASONING_EFFORT` is technically available today and would work for a
subprocess launcher. It is not the answer: it is undiscoverable from `--help`,
it ranks below command-line flags so it cannot be used to override one, it
cannot be spelled per-invocation in a shell pipeline as naturally as a flag,
and it is not how any other run-scoped selection (`--model`, `--approval`,
`--background-exit`) is expressed.

## What Changes

- Add `--effort <NAME>`, a run-scoped invocation flag selecting one
  provider-advertised reasoning effort. It is accepted anywhere the existing
  selection flags are — `smith`, `smith -p`, `smith config explain`, and
  `smith sessions list` — because they share one selection parser.
- Contribute the flag at the **command-line layer**, so it ranks above the
  selected profile's `[profiles.<name>.reasoning] effort` and above
  `SMITH_REASONING_EFFORT`, and below an explicit in-session `/effort`.
- **On resume, an explicitly supplied flag wins over the session's persisted
  override for that run, and leaves the persisted override intact.** See the
  design decision below.
- Validate through the existing rule with no new vocabulary: an effort the
  resolved binding does not advertise fails before runtime construction, names
  the requested value, lists the supported alternatives, and performs no
  credential lookup or provider request.
- Refuse rather than degrade when the binding advertises no adjustable
  reasoning at all, and when it is controllable but advertises no effort ladder
  (a toggle-only binding). Both diagnostics already exist; the flag reuses them.
- Do not let the interactive startup recovery path silently swallow an explicit
  flag. That path exists to clear a *saved* override the current binding cannot
  represent; an effort the user typed on this invocation must fail loudly.
- Suppress the flag when composing child-profile runtimes, exactly as the
  session override is suppressed today.
- Name the flag the user actually typed in `smith config explain
  reasoning.effort` output, and list it in `--help` and the configuration
  reference.

## Design Decisions

### The flag is a command-line-layer value, not a session override

`Selection` already carries `reasoning_effort`, but that field is the transport
for `/effort` and is deliberately converted into the *session* layer — the
comment on `Selection::session_overrides` says so explicitly: interactive
controls become "the highest-precedence session layer rather than pretending
they were command-line flags". Reusing that field for a flag would be the same
dishonesty in the other direction, and would make `smith config explain
reasoning.effort --effort high` report a session override for something typed on
a command line.

So the flag gets its own field on `Selection`, contributed through
`Selection::overrides()` at `Layer::CommandLine`. The layer order already places
that above `Profile` and `Environment`, so the required precedence falls out of
the existing resolver with no new ranking rule.

### Resume precedence: the flag wins for the run, the persisted override survives

This is the real decision in the change. A resumed session may already carry a
persisted `reasoning.effort` written at the session layer, which
`PersistedReasoningOverride::apply` reinstates during resume — above the
command-line layer. Three options were considered.

- **(A) Route the flag into the session layer**, so it is indistinguishable
  from `/effort`. Costs nothing to build: `apply` already declines to reinstate
  a persisted value when an explicit session-layer value is present, and the
  save path would then persist the flag's value. Rejected because it lies about
  provenance and because a *per-invocation* flag would permanently rewrite
  durable session state — one `--effort high` resume would silently replace a
  `/effort medium` the user chose inside that session.
- **(B) Leave the flag at the command-line layer and change nothing else.** The
  persisted session override then outranks the flag on every resume, which is
  precisely the case an orchestrator needs to win. Rejected: a flag that is
  silently ignored on resume is worse than no flag.
- **(C, recommended) Command-line layer, and an explicitly supplied flag
  suppresses the persisted *effort* for that run without rewriting it.** The
  run uses the flag; the saved override is neither applied nor overwritten, so
  dropping the flag on a later resume brings the session's own choice back. The
  persisted thinking state (`reasoning.enabled`) is untouched either way.

(C) keeps two concepts cleanly separated — a flag is this run, `/effort` is this
session — and is the only option where both "the orchestrator's per-task effort
wins" and "the user's in-session choice is not destroyed" hold at once.

Its cost is one behavior that must be built rather than inherited: today the
value written back to session state is derived from the resolved config's
session-layer values alone, so suppressing the persisted effort would also erase
it on the next save. The save path must carry the restored value forward when a
higher layer merely shadows it. That is a small, testable change in the host,
and it is the reason this decision is called out rather than assumed.

A consequence worth stating: with (C), a mid-session `/effort default` clears
the *session* override and therefore falls back to the invocation flag, not to
the provider default. That is the correct layered reading of "default" and is
explainable through `smith config explain`, but it is a behavior a reviewer
should agree with before implementation.

### The flag does not disable reasoning

`--effort` selects one value from the binding's advertised ladder. It gets no
`off`, no `none` shorthand, and no companion `--no-think` flag in this change.

Disabling is a different concept with a different representation per dialect: on
the OpenAI-effort dialect off *is* the effort `none` and exists only where the
ladder advertises it; on the OpenRouter and Z.AI dialects it is a separate
boolean on a separate wire field; on native Gemini thinking it is not
representable at all. A flag that sometimes means "pick this effort" and
sometimes means "set `reasoning.enabled = false`" would smuggle a second
concept into one argument and reintroduce exactly the guessing
`resolve_reasoning_policy` refuses to do.

Where a binding genuinely advertises `none`, `--effort none` already expresses
"off" through the ordinary ladder with ordinary validation, so nothing is lost.
If an invocation-level thinking switch is wanted later it belongs in a separate
`reasoning.enabled` flag and a separate change.

## Alternatives Considered

- **Document `SMITH_REASONING_EFFORT` instead of adding a flag.** Cheapest
  option, and it does unblock a subprocess launcher. Rejected as the answer:
  environment variables rank below flags, so they cannot override one; they are
  invisible in `--help`; and every other run-scoped selection in Smith is a
  flag. The variable stays supported and is worth documenting regardless.
- **`--reasoning-effort` as the spelling.** It matches the dotted setting key
  and matches what `Source`'s `Display` currently prints for a command-line
  `reasoning.effort` value. Rejected: Smith's flag vocabulary is the *control's*
  name, not the key path — `--approval` for `approval.mode`,
  `--background-exit` for `background.exit_policy` — and the in-product name of
  this control is already `effort` (`/effort`, the effort picker, the status
  row). `--effort` is also unclaimed: nothing in the repository parses either
  spelling today. The mismatch this exposes in diagnostics is real and is
  handled as its own task rather than by renaming the flag.
- **`--effort` as an alias for a profile lookup** (resolve to a
  convention-named profile such as `<profile>-high`). Rejected outright: it
  invents a naming convention, fails opaquely, and keeps effort a
  config-deployment concern.
- **Accepting a numeric effort or a `max`/`min` alias.** Rejected: the ladder is
  provider-advertised and endpoint-specific; mapping a synthetic name onto it is
  the "guessed nearest effort" the existing specs already forbid.

## Impact

- Affected specs: `configuration`, `client-surfaces`
- Affected code: `crates/smith-cli` (`cli.rs` parser, `Selection`, `HELP`,
  `runtime_host.rs` startup recovery and child-profile composition),
  `crates/smith-config` (`Overrides` field, command-line contribution, flag
  spelling in diagnostics), `crates/smith-runtime` (`host.rs` save path so a
  shadowed persisted override survives)
- Affected docs: `docs/configuration.md` command-line selection block and
  reasoning section
- Compatibility: additive. An invocation without `--effort` resolves exactly as
  before, including on resume.
- Network behavior: none. Every failure mode is local and pre-credential.

## Out of Scope

- Any flag that turns reasoning on or off, including `--think`, `--no-think`,
  `--effort off`, and `--effort default`.
- A reasoning token-budget flag. `ReasoningConfig::max_tokens` is still unused
  repo-wide and gains no surface here.
- New effort ladders, new dialects, or any change to which endpoints grant
  adjustable controls.
- Forge-side work of any kind. Smith gains no knowledge of Forge, and the
  one-way dependency boundary in `forge-integration` is untouched.
- Changing `/effort`, its picker, or the persisted-override format.
- A machine-readable listing of the advertised ladder (for example
  `smith config explain --list`). Callers still discover supported values from
  the refusal diagnostic.

## Approval Boundary

Approval authorizes exactly: the `--effort <NAME>` invocation flag at the
command-line layer; the resume rule in decision (C), including the host change
that preserves a shadowed persisted override; refusal-not-degradation on
non-controllable and ladder-less bindings; suppression of the flag for child
profiles; and the help, diagnostic-spelling, and documentation updates listed in
`tasks.md`.

It does not authorize an invocation-level thinking switch, a second spelling or
alias for the flag, any change to the advertised ladders or dialects, any change
to `/effort` semantics beyond the layered fallback that decision (C) implies, or
any Forge-side integration.
