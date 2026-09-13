# Proposal: Fix installed coding-agent selection and honest candidate previews

## Why

Two selection-inventory defects make Smith harder to use than its own
contracts promise:

1. A profile that selects an installed coding agent by model id
   (`cli/claude-code/sonnet`, `cli/codex/gpt-6-astra`) resolves correctly at
   runtime — `resolve_model_limits` supplies built-in bookkeeping limits — but
   the selection inventory never enumerates the pair, because
   `inventory_limit` consults only explicit `[models]` tables, trusted
   descriptors, and the Models.dev catalog. The picker therefore reports the
   profile as `unavailable: profile does not resolve to a usable
   provider/model pair`, contradicting the documented contract in
   `crates/smith-config/src/cli_agents.rs`: "A CLI agent is selected the same
   way any other model is — by id — so it needs no declaration to use."
   Users today must hand-write `[models."google/cli/..."]` bookkeeping tables
   purely to make profiles selectable, which is exactly the friction the
   namespace exists to remove.

2. While previewing any non-active candidate, the inventory carries the
   *active* profile's explicit request-output and reserve values across as
   hard constraints. A profile-scoped `max_output_tokens = 32768` therefore
   disables every candidate whose ceiling is smaller (for example the CLI
   bookkeeping ceiling of 32,000) with `request output budget 32768 exceeds
   model output ceiling 32000`. The value belongs to the active profile and
   would not apply after a switch, so the preview is not honest: switching
   models appears to break when it does not.

Both were reproduced with committed tests against Smith 0.2.5 on a real
configuration.

## What Changes

- The selection inventory enumerates installed-agent model ids referenced by
  any profile using Smith's built-in bookkeeping limits (context 200,000 /
  input 180,000 / output 32,000), with a new `ModelLimitOrigin::BuiltIn`
  provenance. Explicit `[models]` limits continue to win.
- Profile-scoped request-output and reserve values from the active profile
  are no longer applied when previewing other candidates; each candidate
  derives its own automatic budget. Explicit values from layers that persist
  across a model switch (user-global configuration, environment, command-line
  flags, session overrides) still apply and still disable conflicting
  candidates with a bounded reason.
- Selection surfaces render installed agents exactly once through the curated
  `cli/<kind>/<model>` namespace; provider-qualified inventory rows for the
  same agent no longer duplicate it.
- Resolve-time behavior is unchanged: explicit values remain authoritative
  and are never clamped for the model actually being run.

## Impact

- Affected specs: `configuration`, `client-surfaces`
- Affected code: `crates/smith-config/src/inventory.rs` (built-in CLI limits,
  per-candidate preview budgets), `crates/smith-cli/src/resources.rs`
  (curated row dedup), plus focused tests in both crates
- No wire protocol, credential, persistence, approval, or authority changes
- Users can delete hand-written `[models."provider/cli/..."]` workaround
  tables after this ships

## Approval Boundary

Approval authorizes exactly the inventory enumeration and preview-budget
changes described above. It does not authorize clamping explicit values for
the active model, changing resolve-time validation, adding configuration
authority, altering harness `[harness.<kind>]` owner-only rules, or any
provider/wire behavior change.

Approved by the user for implementation on 2026-09-12.
