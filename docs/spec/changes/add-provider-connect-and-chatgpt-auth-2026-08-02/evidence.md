# Coordination Evidence

Verified at `2026-08-02T21:49:23Z`.

## Agent Runtime

The coordinated renewable-credential implementation and conformance suite pass
against Agent Runtime base revision
`39ba8319207a8b51a6be84e2ad60a18edf2a5fc8`. That implementation remains an
uncommitted sibling working tree. Smith's committed manifest therefore still
pins released revision `b24cc1bec22ffca106591feee9eb4f5bb2a9a9d3`, while the
git-ignored local Cargo patch supplies the reviewed sibling checkout for this
development build. Task 0.5 stays open until publication is separately
authorized.

## Direct ChatGPT boundary

The public OpenAI Platform contract documents API-key and workload-identity
authentication, not direct ChatGPT subscription access. The approved feature is
therefore explicitly experimental. The implementation pins the current public
Codex native-client issuer, client identifier, scopes, PKCE/device ceremonies,
and the observed `https://chatgpt.com/backend-api/codex/responses` wire contract.
It neither installs nor launches Codex and does not read Codex/OpenCode caches.

## Smith implementation

- `/connect [PROVIDER]` and `/disconnect [PROVIDER]` are typed idle-only
  commands backed by searchable resource pickers and safe host boundaries.
- OpenRouter uses a fixed descriptor endpoint and an offline reviewed catalog
  handoff; reconnect and disconnect preserve unrelated configuration.
- Browser PKCE and device-code login run inside Smith with bounded waits, strict
  callback path/state validation, and redaction-safe progress and errors.
- Smith stores one versioned renewable token bundle in the `chatgpt` entry of
  fixed owner-only plaintext `~/.smith/auth.json`, performs single-flight
  proactive refresh, persists refresh-token rotation, and invalidates exact
  revisions on authentication rejection. The ChatGPT lifecycle performs no
  Keychain/Secret Service operation and leaves the legacy entry untouched.
- The dedicated runtime adapter sends canonical messages, tools, and reasoning
  directly to the fixed ChatGPT Responses endpoint, normalizes SSE output,
  tool calls, usage, and terminal reasons, and permits only one pre-output auth
  recovery replay.
- The backend currently rejects the standard `max_output_tokens` field. Smith
  keeps that value as local context/reserve policy and omits it only from this
  experimental wire request.
- Configuration cannot redirect the endpoint or credential location from a
  project layer. Reconnect, disconnect, and active-provider rebuild remain
  transactional and preserve unrelated configuration and the Smith session.

## Verification

- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- Strict change validation: `Valid` (39 of 41 tasks complete).
- Focused OAuth callback-forgery, token-bundle, concurrent refresh/rotation,
  exact 401 invalidation, direct adapter, malformed stream, config
  anti-redirection, CLI, and PTY tests pass.
- The two opt-in live provider tests remain ignored by default because they
  require credentials and spend quota.
- A redaction-safe live diagnostic isolated HTTP 400
  `Unsupported parameter: max_output_tokens`; the same request without that
  field returned HTTP 200, `OK`, and `response.completed`. The live adapter
  smoke then returned `SMITH_OK` while its canonical request still carried a
  local output budget.
- The exact Smith request through the new auth-file source isolated a second
  HTTP 400: canonical tool name `tool:registry.search` violated the backend's
  `^[a-zA-Z0-9_-]+$` wire-name rule. The adapter now assigns collision-free
  aliases only to incompatible names and reverses them on streamed tool calls;
  its exact previously rejected payload then returned HTTP 200.
- At the user's request, the committed ChatGPT live smoke never queries
  Keychain. It requires explicitly injected test-only access-token/account
  variables and otherwise remains ignored, so tests cannot trigger a macOS
  Keychain password prompt.
- Owner-only auth-file fixtures cover `0700`/`0600` permissions, version and
  size bounds, symlink/non-regular refusal, concurrent writers, preservation of
  unrelated entries, exact enrollment rollback, idempotent removal,
  resolution, rotated refresh persistence, redaction, and panic-on-any-
  Keychain lifecycle behavior.
- `cargo install --path crates/smith-cli --force --locked` replaced the local
  `/Users/mai1015/.cargo/bin/smith`; SHA-256 is
  `347717f7a7d09244492d38d9f08d934ed11078dd5cb7804d1889231bede6b3cd`.
- A local PTY completed Smith-owned browser PKCE, wrote a regular mode `0600`
  `~/.smith/auth.json` beneath a mode `0700` directory, and published only
  `authfile:chatgpt` in configuration. The installed binary then completed
  `smith -p hello --provider chatgpt --model gpt-5.6-terra` with one committed
  attempt and the response `Hello! How can I help with the repository?`.

The remaining release gates are intentionally open: publish and pin the exact
compatible Agent Runtime revision, and run the credential/login lifecycle on
the Linux CI target.
