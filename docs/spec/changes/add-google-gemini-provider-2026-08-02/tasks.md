---
created_at: 2026-08-02T21:10:27Z
updated_at: 2026-08-02T21:42:11Z
completed_at:
---

## 0. Approval and upstream contract gate

- [x] 0.1 Approve native stateless Interactions, automatic Models.dev metadata,
  and the optional `models.toml` configuration split.
- [x] 0.2 Create the coordinated Agent Runtime
  `add-native-gemini-interactions-provider-2026-08-02` proposal covering the
  adapter, exact continuation data, streaming normalization, and conformance.
- [ ] 0.3 Approve and implement the upstream change before exposing the adapter
  in Smith.
- [ ] 0.4 Rebase onto the final provider-catalog and `/connect` contracts
  without parallel inventory, credential, or runtime paths.

## 1. Dedicated model-catalog files

- [ ] 1.1 Add strict `models.toml`/`models.local.toml` file types, discovery,
  layer provenance, and unknown-field rejection at user, project, and
  project-local scopes.
- [ ] 1.2 Keep legacy `[models]` in `config*.toml` readable with a deprecation
  diagnostic and reject same-scope duplicate fields across old/new files.
- [ ] 1.3 Add a comment-preserving, multi-file, rollback-capable migration and
  setup transaction that writes new explicit model records only to dedicated
  files.
- [ ] 1.4 Add precedence, trust, malformed-file, collision, rollback, and
  backward-compatibility tests.

## 2. Google catalog and descriptor

- [ ] 2.1 Add `google` to the bounded Models.dev generator/runtime allow-list
  and regenerate the embedded seed reproducibly.
- [ ] 2.2 Bind the built-in `gemini-interactions` adapter to the Google catalog
  without importing endpoint, npm package, environment, or auth policy.
- [ ] 2.3 Add a trusted Google setup/connection descriptor with fixed endpoint,
  API-key methods, and `gemini-3.6-flash` as only the recommended model ID.
- [ ] 2.4 Add catalog absence/corruption, invalid model, explicit override,
  provenance, and no-generated-model-record tests.

## 3. Native runtime composition

- [ ] 3.1 Consume the approved Agent Runtime release and add
  `gemini-interactions` to Smith's compiled adapter inventory and factory.
- [ ] 3.2 Enforce the fixed trusted endpoint, `store=false`, streaming,
  `x-goog-api-key`, and no project-controlled endpoint/header overrides.
- [ ] 3.3 Preserve exact thought/signature/function continuation through
  session save, resume, retry, compaction boundaries, and provider switching.
- [ ] 3.4 Add deterministic Smith contract fixtures for text, reasoning,
  parallel/sequential tools, multimodal input/results, structured output,
  usage/cache, cancellation, malformed streams, and classified errors.

## 4. Setup and connection surfaces

- [ ] 4.1 Expose Google Gemini in guided setup and `/connect google` using the
  existing masked, reviewed API-key transaction.
- [ ] 4.2 Populate `/model` from the same frozen Google catalog used by runtime
  preflight, with catalog provenance and disabled reasons.
- [ ] 4.3 Ensure catalog-backed setup writes no endpoint or model metadata and
  add cancellation, reconnect, rollback, disconnect, and secret-free UI tests.

## 5. Live validation, docs, and gates

- [ ] 5.1 Add an ignored, spend-bounded native Gemini tool-loop test using only
  `SMITH_LIVE_API_KEY` and an isolated workspace.
- [ ] 5.2 Document the minimal configuration, `models.toml` overrides,
  migration, catalog provenance, and native/Vertex/hosted-tool boundaries.
- [ ] 5.3 Run formatting, Clippy with warnings denied, focused tests, workspace
  tests, catalog reproducibility, migration rollback, replay, and
  secret-scanning gates.
