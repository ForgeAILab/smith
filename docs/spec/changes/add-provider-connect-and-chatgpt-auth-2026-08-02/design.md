## Context

Smith already supports static provider setup and in-session provider/model
selection. The approved `/connect` work added an Agent Runtime credential lease
contract for expiry, refresh, exact invalidation, and one bounded pre-output
authentication recovery attempt.

The first ChatGPT implementation used Codex app-server for login and execution.
That does not meet the product requirement: Smith must own login and model
requests, and a user must not need a Codex process installed.

The public OpenAI API accepts Platform API keys and workload-identity tokens;
it does not document ChatGPT subscription OAuth as third-party API
authentication. The public Codex source nevertheless defines browser PKCE and
device-code ceremonies, renewable token behavior, and the ChatGPT Codex
Responses backend. OpenCode implements the same class of direct integration.
The user explicitly approved this unsupported boundary for an experimental
Smith-native implementation.

## Goals / Non-Goals

### Goals

- Connect, reconnect, inspect, and disconnect providers from the running TUI.
- Make OpenRouter a first-class API-key connection with secure storage.
- Give direct adapters renewable credential mechanics without teaching Agent
  Runtime core about browser UI or provider-specific OAuth.
- Support ChatGPT browser and device-code login directly in Smith.
- Default ChatGPT token persistence to a fixed owner-only Smith `auth.json`
  file without initializing Keychain or Secret Service.
- Keep ChatGPT work inside Smith's normal runtime, tool, approval, persistence,
  cancellation, event, recovery, and usage contracts.
- Keep tokens, callback material, account identifiers, and raw backend
  diagnostics out of render state, events, transcripts, snapshots, and logs.
- Preserve cancellation, bounded waits, rollback, and safe-boundary runtime
  replacement.

### Non-Goals

- Implement a generic password manager or arbitrary OAuth client editor.
- Allow project configuration to define OAuth issuers, client identities,
  scopes, redirect URIs, token endpoints, callback listeners, storage, or the
  ChatGPT backend endpoint.
- Treat ChatGPT subscription tokens as OpenAI Platform API keys.
- Read, import, or mutate Codex/OpenCode credential caches.
- Automatically read, migrate, or delete Smith's legacy ChatGPT Keychain
  entry; reconnect writes a fresh file credential without prompting for it.
- Present the experimental flow as a supported OpenAI Platform contract.
- Refresh or replay after a provider stream has accepted semantic output.

## Decisions

### `/connect` is a local provider-to-auth funnel

`/connect [PROVIDER]` and `/disconnect [PROVIDER]` remain local, idle-only
commands. A missing provider argument opens the shared searchable picker.
Selecting a provider shows only its trusted authentication methods.

OpenRouter fixes its standard endpoint and then reuses the existing reviewed
API-key transaction and model-selection handoff. Reconnecting an existing
provider changes only authentication. Arbitrary endpoints remain setup work.

### Agent Runtime owns renewable authorization mechanics

Agent Runtime consumes a host-injected `ProviderCredentialSource` which returns
a secret lease with optional expiry and opaque revision. It owns minimum-valid
acquisition, exact-revision invalidation, and the canonical one-replay fence.
It does not own browser control, callbacks, OAuth parameters, account
selection, logout, or refresh-token persistence.

The dedicated Responses adapter and ChatGPT credential source live in
`smith-runtime`, because the endpoint and auth behavior are Smith product
policy and an undocumented provider integration. They still implement Agent
Runtime's neutral `Provider`, transport, credential, event, and usage contracts.

### Smith owns OAuth presentation and owner-only plaintext persistence

The trusted ChatGPT integration pins the public Codex native-client behavior in
product code:

- issuer `https://auth.openai.com`;
- the public Codex client identifier;
- PKCE S256 with strict state checking and an allow-listed localhost callback;
- current Codex scopes including offline and connector access;
- the published device authorization endpoints; and
- refresh at the issuer token endpoint.

State, verifier, authorization code, and callback query remain memory-only and
are zeroized on completion, cancellation, or failure. The access token,
rotating refresh token, expiry, and account ID are serialized as one versioned
secret bundle in the `chatgpt` entry of `~/.smith/auth.json`. Configuration
records only the fixed non-secret `authfile:chatgpt` reference.

The file is deliberately plaintext, matching the user's requested no-prompt
posture. Smith creates `~/.smith` as mode `0700`, writes `auth.json` as mode
`0600`, refuses symlink/non-regular targets, bounds parsing, serializes writes
under an owner-only lock, and publishes updates with fsync plus atomic rename.
Connect, refresh-token rotation, rollback, and disconnect all use that one
store. The path and entry are product constants, never project-controlled.
Keychain and Secret Service remain available for other provider/setup choices
but are not initialized by the ChatGPT connection lifecycle.

### ChatGPT is an experimental Smith-native direct provider

The provider posts Responses-shaped requests to the fixed
`https://chatgpt.com/backend-api/codex/responses` endpoint with bearer and
ChatGPT account headers. It maps canonical messages, images, reasoning,
function calls, tool outputs, streaming deltas, usage, cache observations, and
finish/error terminals. It never launches Codex or starts another agent loop.

The first trusted model binding is pinned to public Codex model metadata. Smith
uses a conservative explicit output cap below the published context window, so
runtime planning never relies on a guessed unlimited output. That cap remains
local planning policy: the current direct ChatGPT endpoint rejects the
otherwise standard Responses `max_output_tokens` request field, so the adapter
MUST omit it until the reviewed backend contract accepts an equivalent bound.

A current-revision 401 before semantic output invalidates the lease and may
produce one visible Agent Runtime recovery attempt. A 401 after output, a
second rejection, cancellation, or deadline expiry is terminal.

### Connection changes are transactional and safe-boundary only

API-key changes retain the existing reviewed config/credential transaction.
ChatGPT login similarly keeps the prior auth-file entry and exact prior config
bytes until token validation, configuration publication, and direct-provider
preflight succeed. Failure restores both without reading the legacy Keychain
entry.

If the connected provider is active, Smith saves and rebuilds the session only
after work is idle. Cache state becomes unknown. Disconnecting the only active
provider requires a replacement or session exit.

## Risks / Trade-offs

- The OAuth client and ChatGPT endpoint are not a published third-party
  contract. Scopes, redirects, headers, models, and payload validation may
  change without compatibility notice.
- OS credential services can block or prompt; reads and writes remain bounded
  at Smith's async boundary and errors expose only fixed classifications.
- `auth.json` is readable by same-user processes and may survive in backups;
  mode checks do not provide encryption at rest. Smith must disclose this and
  users must treat the file like a password.
- Responses reasoning continuity requires preserving opaque encrypted content
  without rendering it; tests must prove it stays out of user-visible events.
- Device-code availability can be disabled by account/workspace policy.

## Migration Plan

- Existing static key, Keychain, environment, and inline compatibility paths
  remain non-expiring credential sources.
- Existing Codex-managed credentials are not imported. Users complete one
  Smith-owned login to create `~/.smith/auth.json`.
- Existing `keychain:smith/chatgpt` entries are intentionally not read,
  migrated, or deleted, avoiding another OS prompt. A user reconnects ChatGPT
  while running Smith with another configured provider; successful login
  updates the credential reference to `authfile:chatgpt`.
- The Codex app-server module, external session bridge, dynamic external model,
  and Codex-owned status text are removed.
- No existing provider is automatically selected, migrated, or disconnected.

## Rollout Decisions

- ChatGPT is listed only when OAuth, the owner-only auth-file store, the direct
  Responses adapter, and reviewed model metadata are compiled together.
- Every ChatGPT picker/status/help surface labels the integration experimental.
- Browser login attempts to open the URL and always leaves it copyable.
- Live OAuth/provider tests remain opt-in and spend bounded.
- ChatGPT tests inject file-store or static credential backends and never open
  the developer's Keychain or Secret Service.

## References

- OpenAI Authentication: https://learn.chatgpt.com/docs/auth
- OpenAI API authentication:
  https://developers.openai.com/api/reference/overview#authentication
- Public Codex login source:
  https://github.com/openai/codex/tree/main/codex-rs/login
- Public Codex Responses source:
  https://github.com/openai/codex/tree/main/codex-rs/codex-api
- OpenCode provider UX:
  https://github.com/anomalyco/opencode/blob/dev/packages/web/src/content/docs/providers.mdx
- OpenCode direct Codex integration:
  https://github.com/anomalyco/opencode/blob/dev/packages/opencode/src/plugin/openai/codex.ts
