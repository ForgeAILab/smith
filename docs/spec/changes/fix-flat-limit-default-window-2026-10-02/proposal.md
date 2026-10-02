---
created_at: 2026-10-02T22:33:07Z
updated_at: 2026-10-02T23:10:00Z
---

# Proposal: Let a flat model limit outrank a built-in default window

## Why

Smith 0.2.15 refuses to start when a configuration gives a model flat
`context_tokens` or `max_input_tokens` and the trusted catalog also names a
default context window for that model. 0.2.15 added such a record for
`chatgpt/gpt-6.1-sol`, so a configuration that worked in 0.2.14 now fails
with `window 272k is pinned by flat model limit ...`, even though nobody
selected `272k`. A child-enabled profile is enough to trigger it, because one
child profile that fails to resolve ends the root session.

The configuration resolver already drops a built-in default that a flat limit
outranks (`smith config explain context_window` prints "not set"). The
session factory derives the default a second time without that check and
reports it as a window the user asked for.

## What Changes

- The session factory drops a default window name whose source the flat limit
  outranks, and applies the flat limits.
- A window name whose source outranks the flat limit (profile
  `context_window`, `--context-window`, a session override) still fails with
  the error that names the flat key.
- Release the fix as Smith 0.2.16.

## Impact

- Affected specs: `configuration`.
- Affected code: `crates/smith-runtime/src/factory/context_policy.rs`,
  `crates/smith-runtime/tests/context_windows.rs`, release metadata.
- Not changed: whether a child profile that fails to resolve ends the root
  session, and the duplicated default-window derivation between
  `smith-config` and the factory.

## Authorization

The owner approved fixing the startup failure first as 0.2.16 and committing
it. The in-flight client work in the original checkout is a separate later
release and stays there. The owner then approved tagging and releasing
0.2.16 and installing it locally.
