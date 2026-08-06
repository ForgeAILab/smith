---
created_at: 2026-08-06T00:00:00Z
updated_at: 2026-08-06T00:00:00Z
---

## Why

A bare `@token` that does not resolve to a workspace file or child agent
blocked the send with "unresolved reference; choose it from `@` completion".
That is the wrong default for ordinary prose: npm scoped package names
(`npx @forgeailab/smith`), social handles, and email-like text all contain a
leading `@` that the user never intended as an attachment. The only documented
escape was `@@`, which is discoverable but friction for the common case of
pasting a command that contains a scope name.

The user's expectation is that an `@` sign they typed should send as text unless
they actively selected a file or agent through the `@` completion surface.

## What Changes

- A bare `@token` that resolves to a known file or child agent keeps its
  current reference behavior (attached file or agent delegation).
- A bare `@token` that resolves to **neither** now passes through as literal
  text and sends normally. No error, no attachment, draft is consumed.
- Explicit typed prefixes still fail locally when unresolved:
  `@file:missing.rs` and `@agent:ghost` report the existing bounded error,
  because the user explicitly asked for a typed reference.
- Ambiguous collisions (a name that is both a file and an agent) still report
  the existing bounded ambiguity error.
- The `@@` escape continues to produce one literal `@` with no resolution.

## Impact

- Affected specs: `client-interaction`
- Affected code: `crates/smith-tui/src/references.rs` (one parser branch);
  `crates/smith-tui/src/app/tests/pending_input.rs` and
  `crates/smith-tui/src/app/tests/child_lifecycle.rs` (integration tests)
- Compatibility: no runtime, journal, or event-schema change. References that
  still resolve behave identically.
- Trade-off acknowledged: a typo'd file reference like `@src/li b.rs` now sends
  as text instead of surfacing an attachment failure. This is accepted because
  blocking ordinary prose was a worse default. Explicit typed prefixes
  (`@file:`, `@agent:`) preserve the hard-fail path for deliberate attachments.

## Approval Boundary

Approval authorizes treating unresolvable bare `@token` as literal text instead
of a local error. It does not change typed-prefix resolution, ambiguous-collision
handling, the `@@` escape, agent delegation, or any provider-runtime behavior.
