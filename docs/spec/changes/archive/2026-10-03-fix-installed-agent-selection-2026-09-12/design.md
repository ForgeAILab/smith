# Design: Fix installed coding-agent selection and honest candidate previews

## Root causes

### Installed agents never enumerate

`crates/smith-config/src/inventory.rs` builds `candidate_pairs` from
`[models]` table keys, profile pairs, and catalog models, then resolves each
pair's limits through `inventory_limit`, which knows only three origins:
explicit config, trusted descriptors, catalog snapshot. A `cli/...` model has
none of those, so all three limits are `None` and the guard

```rust
if catalog_model.is_none() && (context_tokens.is_none() || ...) { continue; }
```

drops the entry entirely. `model_ids` therefore never contains
`google/cli/claude-code/sonnet`, and `profile.selectable` — defined as the
pair being present in `model_ids` — is false.

Meanwhile `resolve_model_limits` (`crates/smith-config/src/resolve/provider.rs`)
already fills the built-in CLI bookkeeping constants for `cli/...` ids when a
profile is actually resolved. The runtime works; only the inventory preview
is blind to the same constants.

### Active profile values poison candidate previews

In the same loop, the preview request is derived once from the active
resolution:

```rust
let configured_request_tokens = match resolution.config.max_output_tokens.as_ref() {
    Some(request) if request.source.layer != Layer::BuiltIn => Some(request.value),
    ...
};
```

Any non-built-in layer qualifies, including `profiles.code.max_output_tokens`
— a value scoped to the active profile that would *not* survive switching to
another profile or model. `resolve_output_budget` then fails the candidate
with `RequestExceedsModel` when the carried value exceeds the candidate's
ceiling, and the row is disabled. The same carry-over applies to
`context.output_reserve` (a profile-scoped reserve can fail other candidates
with `NoInputBudget`).

## Approach

1. **Built-in CLI limits in inventory.** When `parse_cli_model_id(model)`
   matches and no explicit/trusted/catalog limit exists, fall back to
   `CLI_AGENT_CONTEXT_TOKENS` / `CLI_AGENT_MAX_INPUT_TOKENS` /
   `CLI_AGENT_MAX_OUTPUT_TOKENS`, mirroring `resolve_model_limits`. Record
   provenance as a new `ModelLimitOrigin::BuiltIn` variant (additive; display
   code gains one match arm) so surfaces can label them as bookkeeping, not
   advertised capability — matching how `resources.rs` already renders the
   curated rows (`[built-in]`).

2. **Per-candidate preview budgets.** A carried value survives only when its
   provenance is *not* profile-scoped. Concretely: the preview uses the
   active request/reserve only when the `Source` key is outside the
   `profiles.<name>.` scope (user-global `[context]`, environment, flags,
   session overrides persist across a switch; a different profile's values do
   not). Otherwise the candidate derives its automatic budget from its own
   limits. Explicit-and-persistent conflicts still disable the row with the
   existing bounded reason — the "explicit values are authoritative and
   never clamped" contract is untouched for values that actually apply.

3. **Render installed agents once.** The curated `CLI_AGENTS` loop in
   `resources.rs` (which has the PATH/installed check and `[built-in]`
   labeling) remains the display row. Provider-qualified inventory rows whose
   model parses as a CLI id are skipped in the display mapping; they exist
   for pair matching and direct selection, not as a second row.

## Alternatives considered

- **Clamp explicit request values to candidate ceilings during preview.**
  Rejected: it would silently misreport what a switch does and erodes the
  authoritative-explicit contract. The defect is carrying a value that would
  not apply, not refusing to clamp one that would.
- **Enumerate every `cli/...` id under every configured provider.** Rejected:
  wrong pairing noise (an agent has no relationship to, say, the zai
  provider) and rows the user never asked for. Enumeration follows profile
  references, which is also how the resolver pairs them.
- **Fix only the user's configuration.** Rejected: the workaround tables are
  pure bookkeeping the product already owns; asking users to write them is
  the documented anti-goal.

## Risks

- `ModelLimitOrigin::BuiltIn` is a public enum variant; all in-tree matches
  are updated and it is additive for downstream consumers.
- The dedup rule keys on `parse_cli_model_id`, the same parser the resolver
  uses, so display and selection cannot disagree about what is a CLI id.
- Behavior change is observable: profiles previously shown `unavailable`
  become selectable. That is the intended contract restoration, verified by
  the new regression tests.

## Test plan

- `smith-config` inventory tests: zero-declaration CLI profile enumerates and
  is selectable; explicit `[models]` overrides built-ins with configured
  provenance; profile-scoped request/reserve no longer disable other
  candidates; persistent user-global request still disables a conflicting
  candidate with the bounded reason; the active candidate's own budget is
  unchanged.
- `smith-cli` resource tests: curated CLI rows appear exactly once; profile
  rows for installed agents are not marked unavailable.
- Full workspace `cargo test` plus fmt/clippy before review.
