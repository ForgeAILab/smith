---
created_at: 2026-08-02T17:45:46Z
updated_at: 2026-08-02T21:49:23Z
---

## Why

Smith can enroll an API key through pre-session setup and can switch among
configured providers, but a running TUI has no `/connect` surface and the
shared runtime accepts only a static secret resolved during construction.
Users cannot connect or reauthenticate a provider where they are already
working, and there is no supported path for a renewable OAuth credential.

ChatGPT subscription authentication needs an especially explicit boundary.
The public OpenAI API accepts Platform API keys and workload-identity tokens;
it does not document ChatGPT subscription OAuth as third-party API
authentication. OpenAI's public Codex source and OpenCode nevertheless expose
the browser/device ceremony, renewable bearer-token behavior, and direct
Responses-compatible ChatGPT Codex endpoint used by native clients. The user
has approved that unsupported integration boundary for an experimental,
Smith-native implementation with no Codex process dependency.

The first Smith-native build stored that bundle in macOS Keychain. Rebuilt
local binaries can repeatedly trigger Keychain authorization prompts, which is
not the requested default. The user has requested the same explicit
owner-only plaintext-file posture Codex supports for `auth.json`: Smith should
use `~/.smith/auth.json` by default for ChatGPT and avoid Keychain entirely on
that path.

## What Changes

- Add `/connect [PROVIDER]` as an idle-only local command. With no argument it
  opens a searchable provider picker; with an argument it opens that
  provider's supported authentication methods.
- Add `/disconnect [PROVIDER]` and connection status so renewable credentials
  can be cleared without editing files or leaking account/token data.
- Reuse Smith's masked input, reviewed credential transaction, and provider
  inventory for API-key providers. Add a first-class OpenRouter descriptor so
  connecting it does not require typing its standard endpoint.
- Add a host-neutral renewable-credential contract to Agent Runtime through a
  separately approved upstream change. The runtime owns lease acquisition,
  expiry, invalidation, and one bounded authentication-refresh replay;
  providers own wire authorization; hosts own login presentation and storage.
- Add browser-URL and device-code progress states to Smith's connection UI,
  including cancellation, timeout, terminal restoration, and secret-free
  diagnostics.
- Add an experimental Smith-native ChatGPT provider using browser PKCE or
  device-code login, Smith-owned owner-only `auth.json` storage and refresh, and
  direct Responses-protocol requests to the ChatGPT Codex backend. Do not
  launch Codex or delegate execution to a second agent loop.
- Add a dedicated Responses adapter in `smith-runtime` over Agent Runtime's
  neutral `Provider`, renewable credential, transport, event, and usage
  contracts. Keep undocumented OAuth and endpoint policy out of Agent Runtime
  core.
- Rebuild the selected direct-provider runtime only at an idle safe boundary
  after a successful connection change. Preserve the Smith session and mark
  provider cache state unknown, as for an ordinary provider switch.
- Keep `smith setup` as the durable endpoint/model/limit editor. `/connect`
  manages authentication and may instantiate only reviewed built-in provider
  descriptors; it does not become a general configuration editor.

## Impact

- Affected specs: `client-surfaces`, `configuration`, `provider-runtime`,
  `runtime-integration`.
- Affected code: `crates/smith-tui` (commands and connection modal),
  `crates/smith-cli` (login effects and browser/device orchestration),
  `crates/smith-config` (auth descriptors, user-scoped connection state, and
  credential transactions), and `crates/smith-runtime` (renewable ChatGPT
  credential source and Responses provider composition).
- Upstream dependency: a coordinated Agent Runtime proposal and release are
  required for the renewable direct-provider credential contract. This Smith
  proposal does not by itself authorize edits in the sibling repository.
- Active-change coordination: extends the completed setup and slash-command
  surfaces and uses the approved renewable credential contract. It removes
  the Codex external-backend spike and keeps ChatGPT inside Smith's canonical
  direct-provider loop.
- Security impact: OAuth access and refresh tokens become plaintext user-scope
  secret material in an owner-only file, with strict permissions, atomic
  replacement, symlink refusal, cancellation, redaction, and rollback
  requirements. Same-user processes and backups can read or retain the file.
  Project configuration cannot select its path, OAuth endpoints, client
  identities, scopes, or callback listeners.

## Approval Boundary

Approval authorizes the experimental Smith-native integration using the public
Codex OAuth client identifier and scopes, Smith-owned browser/device login and
token refresh, plaintext storage in the fixed owner-only
`~/.smith/auth.json`, and direct calls to the undocumented ChatGPT Codex
Responses endpoint. It does not claim that flow is a supported OpenAI Platform
API, permit project-controlled OAuth/endpoint/storage overrides, read another
client's credential cache, access Smith's legacy ChatGPT Keychain entry, or
conceal the experimental compatibility and plaintext-at-rest risks.
