# Proposal: Connect any OpenAI-compatible endpoint from `/connect`

## Why

`/connect` today only reaches providers with a built-in ceremony: configured
providers (credential swap), OpenRouter (fixed endpoint), Google Gemini (native
endpoint), ChatGPT (OAuth), and xAI (login). A user running vLLM, Ollama,
LM Studio, Groq, Together, or any other OpenAI-compatible gateway must leave
the session and run `smith setup add-provider` separately — even though the
full flow for exactly that case already exists (the `AddProvider` setup mode,
advertised as "custom OpenAI-compatible endpoint"). The capability is present;
`/connect` simply does not expose it.

## What Changes

- The `/connect` picker gains a stable `openai-compatible` entry (deduplicated
  against a configured provider of the same name, matching how the OpenRouter,
  ChatGPT, xAI, and Google entries behave) describing the generic flow:
  chosen name, endpoint, authentication, and a first model with enforceable
  limits.
- Selecting it (or typing `/connect openai-compatible`) routes to the existing
  `SetupMode::AddProvider` ceremony at the same safe session-rebuild boundary
  every other connection uses. No new setup steps, config keys, or credential
  machinery are added.
- The unconfigured-provider error in the connect path now points at
  `/connect openai-compatible` as the in-session route, alongside the existing
  `smith setup add-provider` guidance.
- The CLI help line for `/connect`/`/disconnect` mentions custom endpoints.

Out of scope: new providers, catalog entries, adapter kinds, config schema,
credential methods, non-interactive connect, and any change to the AddProvider
flow itself.

## Impact

- Affected specs: `client-surfaces` (one modified requirement)
- Affected code: `crates/smith-cli/src/resources.rs` (picker entry),
  `crates/smith-cli/src/connection.rs` (routing + error text),
  `crates/smith-cli/src/cli.rs` (one help line), and a focused unit test in
  `crates/smith-cli/src/main_tests/resources.rs`
- No configuration, persistence, wire-protocol, credential, approval, or
  authority changes; no new dependencies

## Approval Boundary

Approval authorizes exactly the picker entry, the routing of the
`openai-compatible` connect id to the existing `SetupMode::AddProvider`
ceremony, the two guidance-string updates, and the unit test. It does not
authorize changes to the AddProvider steps or review model, new special-case
provider ids, non-interactive connect behavior, or any setup/config schema
change.
