---
created_at: 2026-10-23T10:00:00Z
updated_at: 2026-10-23T12:02:16Z
---

# Proposal: Configure compaction instructions, model, and model-specific timing

## Why

Compaction currently has a fixed instruction, always uses the conversation
model, and shares one global idle cutoff. Users need to choose how summaries
are written, use an independent model, and set different inactivity cutoffs
for model routes with different cache behavior.

## What Changes

- Keep `/compact` as the only built-in command spelling.
- Add `[context.compaction]` with optional `model` and `instructions` values.
  An absent model uses the conversation model; a target accepts a configured
  profile or `provider/model`. An absent instruction uses the built-in prompt.
- Allow `/compact [instructions]` to supply instructions for this attempt.
  Keep the safety requirements for factual summaries and complete tool exchanges.
- Add `/compact model <PROFILE|PROVIDER/MODEL|default>` and
  `/compact instructions <TEXT|default>` to persist those defaults in the user
  config. Explicit setting commands authorize the edit; candidates must pass
  validation and preflight before atomic publication, preserving comments and
  unrelated settings. Rebind the idle session so subsequent compaction uses
  the new default without losing canonical history or summary provenance.
- Add `models."provider/model".cache.inactivity_limit_ms`, with the same
  bounds as `context.cache.inactivity_limit_ms`. Normal layer precedence
  applies; a model-specific value wins within its layer. This controls the
  host's inactivity cutoff, not the provider's server-side cache TTL.
- Add `/compact idle <DURATION|default>` to save or remove the current model's
  cutoff, with `ms`, `s`, `m`, and `h` duration suffixes.
- Preserve protected originals, separate summary usage attribution, failure
  rollback, and the existing once-per-idle-interval admission.

The separately authorized 0/1/5/15-minute live cache sweep uses synthetic
inputs and independent idle lanes. It reports counters and observed bounds,
not an inferred guaranteed TTL; no hit, missing counters, and unavailable
models remain distinct. Measurements will not silently change configuration.

## Example

```toml
[context.compaction]
model = "zai/glm-4.5-air"
instructions = "Preserve decisions, constraints, test results, and outstanding work."

[models."zai/glm-5.3".cache]
inactivity_limit_ms = 900000
```

The duration above is an example, not a measured cache-retention claim.

## Impact

- Specs: `configuration`, `client-surfaces`, `runtime-integration`.
- Code: config schema/resolution/provenance and atomic user-config edits;
  command grammar and execution; secondary provider/model routing; summary
  adapter revision and instructions; idle-safe session reconstruction.
- No new dependencies or provider cache TTL controls.
