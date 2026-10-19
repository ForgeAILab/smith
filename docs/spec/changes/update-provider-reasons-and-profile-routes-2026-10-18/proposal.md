---
created_at: 2026-10-19T00:05:00Z
updated_at: 2026-10-19T04:40:05Z
---

## Why

Session `18a11954` showed three gaps after its Gemini fix shipped in 0.7.2:

- A rejected request read `Config: Gemini provider rejected the request`.
  Gemini had said why ("Request contains an invalid argument."), but its
  answer to a streamed request is an SSE `error` frame, which the transport's
  reason parser did not read, and Agent Runtime then dropped the reason and
  labelled the error `Config`.
- A profile revision hashed each value's full source, file path included.
  The same configuration read from another path got a new revision, and so
  did any edit to a child profile. Resuming a parent then failed outright
  with `no preflighted agent-profile route`, because the recorded child route
  key no longer existed.
- Each of these was found by hand. There was no repeatable live check of
  continuation across turns, interruptions and providers.

## What Changes

- Smith's transport reads a provider's reason from an SSE-framed error body,
  uses a string `code` where there is no `status`, and drops a reason that
  echoes only the bare token of a `Bearer` header. With Agent Runtime v0.7.0
  the reason reaches the user, headless results and a child's report to its
  parent, labelled as a provider error.
- A profile revision hashes each value's layer and key, not its file. Every
  revision changes once on upgrade (prefix `smith-agent-profile-3`).
- A recorded child route key whose exact revision is gone resolves to the
  profile's current route by name, and a child that cannot be routed gets a
  fingerprint that matches nothing. The parent resumes; the child is reported
  as changed.
- An ignored live conformance test, `live_conformance`, drives the real
  binary against the owner's configured routes and writes a report.

## Impact

- Affected specs: `provider-runtime`, `prompt-cache`, `child-agents`,
  `evaluation-harness`.
- Affected code: `smith-runtime/src/transport.rs`,
  `smith-config/src/resolve/agent.rs`, `smith-runtime/src/delegation.rs`,
  `smith-cli/tests/live_conformance.rs`.
- After upgrading, a child interrupted under an older Smith is reported as
  changed and cannot resume; its parent resumes and finished children keep
  their results.
