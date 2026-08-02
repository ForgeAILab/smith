---
created_at: 2026-08-02T17:45:46Z
updated_at: 2026-08-02T21:49:23Z
completed_at:
---

## 0. Approval and upstream contract gate

- [x] 0.1 Approve the `/connect` UX, provider/auth ownership boundary,
  OpenRouter descriptor, and supported ChatGPT integration classification.
- [x] 0.2 Create the coordinated Agent Runtime
  `add-renewable-provider-credentials-2026-08-02` proposal for a cancellable
  renewable credential lease with expiry and invalidation.
- [x] 0.3 Approve the coordinated Agent Runtime proposal before changing its
  public contracts or implementation.
- [x] 0.4 Add upstream conformance fixtures for static credentials, proactive
  refresh, one bounded auth-rejection replay, cancellation, timeout, and
  redaction, and record the verified upstream base revision.
- [ ] 0.5 Pin the exact released compatible Agent Runtime revision in Smith
  after publication is separately authorized.
- [x] 0.6 Verify the current public Codex/OpenCode OAuth, refresh, device-code,
  direct endpoint, request-header, Responses wire, and model-catalog behavior;
  record that the direct third-party contract remains undocumented.
- [x] 0.7 Approve replacing the Codex-owned backend with the experimental
  Smith-native OAuth and direct-request architecture.
- [x] 0.8 Approve replacing ChatGPT's Keychain default with plaintext
  `~/.smith/auth.json`, including the same-user/backup exposure and the decision
  not to read or remove the legacy Keychain entry automatically.

## 1. Connection descriptors and persistence

- [x] 1.1 Add typed connection/auth-method descriptors distinct from model
  capabilities, filtered by compiled direct providers and discovered external
  backends.
- [x] 1.2 Add the built-in OpenRouter descriptor with its fixed endpoint,
  API-key methods, catalog identity, and reviewed model-selection handoff.
- [x] 1.3 Extend the user-scope credential transaction for connection,
  reconnection, and disconnection without modifying unrelated provider,
  model, profile, or default fields.
- [x] 1.4 Add secret-safe connection inventory/status carrying only provider,
  method, account label, backend kind, and classified readiness.

## 2. `/connect` and `/disconnect` surfaces

- [x] 2.1 Register `/connect [PROVIDER]` and `/disconnect [PROVIDER]` in the
  single command registry as idle-only, local, zero-provider-spend actions.
- [x] 2.2 Implement searchable provider and auth-method pickers reusing the
  existing resource picker, masked input, review, and error surfaces.
- [x] 2.3 Add browser URL, device-code, waiting, cancellation, timeout,
  completion, and retry states without retaining token or callback secrets.
- [x] 2.4 Route successful active-provider changes through save/rebuild/resume,
  preserve the session, invalidate cache identity, and restore the old runtime
  and credential transaction if rebuild fails.
- [x] 2.5 Render connection method/readiness in `/status` and help without
  exposing account IDs, tokens, credential locators beyond reviewed references,
  or storage backend payloads.

## 3. Direct-provider renewable authentication

- [x] 3.1 Adapt existing inline, keychain, environment, and injected test
  credentials to the released Agent Runtime credential-source contract.
- [x] 3.2 Inject credential sources through the one Smith runtime factory and
  keep provider-specific header construction inside shared adapters.
- [x] 3.3 Bound acquisition/refresh by provider deadline and cancellation, and
  permit one replay only for a classified pre-stream authentication rejection.
- [x] 3.4 Register every acquired access token with the non-printing redactor
  before provider construction or transport use; prove snapshots/events/errors
  contain no token or refresh material.

## 4. OpenRouter connection

- [x] 4.1 Connect a fresh OpenRouter provider from `/connect` using Keychain,
  existing Keychain, environment reference, or explicit owner-only config
  storage according to existing credential policy.
- [x] 4.2 Reconnect an existing OpenRouter provider without changing endpoint,
  models, limits, profiles, or default selection.
- [x] 4.3 Add offline factory and pseudo-terminal tests for masked input,
  cancellation, collision review, rollback, active-runtime rebuild, catalog
  model handoff, and no provider request during the connection ceremony.

## 5. Smith-native ChatGPT OAuth and direct Responses provider

- [x] 5.1 Remove the Codex app-server process client, external agent loop,
  dynamic model bridge, and every help/status claim that Codex owns execution.
- [x] 5.2 Implement bounded browser PKCE and device-code login using the
  reviewed public Codex client parameters, strict loopback callback/state
  validation, cancellation, timeout, and fixed redaction-safe diagnostics.
- [x] 5.3 Persist only Smith's versioned token bundle in the OS credential
  service; implement single-flight proactive refresh, refresh-token rotation,
  exact-revision invalidation, logout, and access/account extraction without
  exposing tokens or another client's cache.
- [x] 5.4 Add the dedicated ChatGPT Responses adapter over Agent Runtime's
  neutral provider contract, including canonical message/tool/reasoning input,
  SSE output/tool/usage normalization, direct authorization/account headers,
  and one bounded pre-output auth recovery replay.
- [x] 5.5 Add the fixed experimental ChatGPT descriptor, reviewed default model
  metadata, transactional config/credential enrollment, direct provider/model
  selection, safe-boundary rebuild, and normal Smith session/tool/approval/
  persistence behavior.
- [x] 5.6 Add deterministic OAuth/token-store/refresh/provider fixtures for
  callback forgery, cancellation, timeout, rotated refresh tokens, concurrent
  refresh, 401 recovery, malformed streams, tool loops, usage, and canary
  redaction.
- [x] 5.7 Keep Smith's trusted output reserve and request budget local when the
  experimental ChatGPT backend does not accept a per-request output-token
  parameter; add offline and explicitly injected, no-Keychain opt-in live
  regression coverage.

## 6. Verification and documentation

- [x] 6.1 Add reducer, renderer, command, narrow-terminal, no-color, paste,
  redaction, and safe-boundary tests for every connection state.
- [x] 6.2 Add opt-in live smoke tests separately for OpenRouter API-key use and
  experimental direct ChatGPT OAuth/Responses use; keep them spend-bounded and
  exclude them from default CI.
- [x] 6.3 Update README, CLI help, configuration docs, status/help text, and
  recovery guidance, clearly labeling direct ChatGPT subscription access as
  experimental and unsupported by the public Platform API contract.
- [ ] 6.4 Run strict spec validation, formatting, warning-denied Clippy,
  focused tests, workspace tests, and macOS/Linux credential/login lifecycle
  gates.

## 7. Owner-only ChatGPT auth file revision

- [x] 7.1 Add a fixed `authfile:chatgpt` credential reference and a versioned,
  bounded `~/.smith/auth.json` store with mode `0700`/`0600`, symlink refusal,
  cross-process serialization, fsync, atomic replacement, and exact rollback.
- [x] 7.2 Route ChatGPT connect, startup resolution, refresh-token rotation,
  reconnect, and disconnect through the auth file without initializing or
  querying Keychain/Secret Service.
- [x] 7.3 Preserve other auth entries and unrelated configuration, switch the
  ChatGPT descriptor/reference transactionally, and leave the legacy
  `keychain:smith/chatgpt` entry untouched.
- [x] 7.4 Add injected offline coverage for fresh/replacement/rotated/removal
  writes, permissions, malformed/oversized/symlink files, concurrent writers,
  rollback, redaction, and zero Keychain calls across the lifecycle.
- [x] 7.5 Update setup, help, README, configuration, security, and migration
  guidance with the fixed path, plaintext warning, backup risk, and no-Keychain
  behavior.
- [x] 7.6 Re-run the exact Smith live request using the auth-file source,
  isolate any remaining redaction-safe 4xx compatibility detail, add its
  offline regression fixture, and reinstall only after the real turn passes.
