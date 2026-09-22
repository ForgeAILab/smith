# Tasks: Auto-resolve custom model limits in setup

## 1. Resolution sources

- [x] 1.1 `CatalogSnapshot::resolve_model_by_id` in `smith-config`: exact,
      case-folded, last-segment, and separator-folded alias tiers over
      providers with complete limits; deterministic order; unit tests.
- [x] 1.2 New `smith-runtime/src/probe.rs`: bounded `GET {base}/models` with
      optional bearer, permissive field parser (incl. `top_provider`), pure
      entry parser unit-tested against OpenRouter/Groq/LiteLLM/vLLM fixtures.
- [x] 1.3 Refresh the embedded reviewed Models.dev snapshot so current aliases
      such as Claude Fable 5.1 are available on a fresh install.

## 2. Setup flow

- [x] 2.1 `SetupEffect::ResolveModelLimits` emitted at the model step for
      add-provider/add-model only; busy state with a resolving note while the
      driver works.
- [x] 2.2 `apply_resolved_limits`: skip numeric entry when discovery succeeds;
      nothing-found falls back to one context-window field; Back pre-fills
      that field.
- [x] 2.3 Derive maximum input and output from a manually entered context;
      remove their separate input steps; review shows limit provenance.
- [x] 2.4 CLI driver: endpoint/bearer plumbing (typed key, env var, configured
      provider credential), source priority endpoint > catalog, derivation.

## 3. Tests and validation

- [x] 3.1 Unit tests: catalog tiers, probe parser, single-fallback app
      transitions/defaults/provenance, driver derivation math.
- [x] 3.2 Move the two network-named PTY endpoints to a refused localhost URL
      so the suite stays offline; keep their assertions meaningful.
- [x] 3.3 `cargo fmt`, clippy on touched crates, focused test runs; rebuild
      and reinstall the local binary.
