## Context

Smith currently accepts `keychain:`, `env:`, and future-encrypted `file:`
credential references. Keychain may prompt on every rebuilt binary launch;
environment storage is externally managed. Smith's user config is already
atomically written with mode `0600`.

An inline value is not an ordinary configuration string. Resolved
configuration, provenance, explanation, inventory, setup previews,
diagnostics, and debug output currently assume every configured credential is
a safe locator. The implementation must introduce a secret-bearing type
instead of passing an inline key through those string surfaces.

## Goals / Non-Goals

- Goals:
  - Give users the smallest self-contained, non-prompting credential option.
  - Make plaintext-at-rest risk explicit and opt-in.
  - Keep every surface except the owner-only config bytes secret-free.
  - Support migrating an existing provider without rebuilding its model
    configuration.
- Non-Goals:
  - Accept inline credentials from project or project-local configuration.
  - Make plaintext storage the default or automatic fallback.
  - Replace Keychain, environment references, or the future encrypted-file
    backend.
  - Add the separate credential-file backend before the bootstrap design is
    ready.

## Decisions

### Use a distinct secret-bearing user-config field

Providers may declare either `credential = "keychain:…|env:…|file:…"` or
`api_key = "<value>"`, never both. `api_key` is accepted only from
`~/.smith/config.toml`; project and project-local files fail before terminal,
provider, or credential-service I/O.

The file model wraps `api_key` in a custom redaction-safe type. Layer
provenance retains a secret variant whose `Debug` and `Display` are redacted.
The resolved provider carries a `Secret`, and runtime construction registers
it with the existing redactor before it reaches the transport.

### Make setup choice explicit and transactional

“Store in config (no prompts)” warns that processes acting as the same OS user
and backups can read the key. Secret input remains masked. Setup serializes the
key only into a mode-`0600` same-directory temporary config and atomically
publishes it. Collision previews and review lines render `[redacted]`, and a
failed preflight restores the exact prior config bytes.

`smith setup credential --provider <name>` reuses this authentication step for
an existing provider and changes no endpoint, model, limits, profile, or
default selection.

### Move the redaction boundary into configuration

Only config deserialization/serialization and the final transport boundary may
expose the value. Public/debug types, config explanation, setup review,
inventory, events, journals, session snapshots, machine output, failures, and
temporary-file names never contain it.

### Alternatives considered

- Overload `credential = "sk-…"`: rejected because callers already treat that
  field as a display-safe locator. A distinct `api_key` field makes accidental
  rendering reviewable and lets both forms be mutually exclusive.
- Export from a shell profile: already supported through `env:` but does not
  give Smith-managed, self-contained setup.
- Relax the Keychain ACL: platform-specific, sensitive to rebuilt unsigned
  binaries, and does not solve Linux or unavailable-service cases.
- Finish encrypted-file support first: stronger at rest, but its external
  decryption key can recreate the same repeated-prompt problem.

## Risks / Trade-offs

- The API key is plaintext at rest. Mode `0600` does not protect against
  malware or another same-user process.
- Backups may capture `~/.smith/config.toml`.
- A permissive or non-regular user config containing `api_key` is refused
  rather than silently repaired during ordinary startup.

## Migration Plan

- Existing `keychain:`, `env:`, and encrypted `file:` references are unchanged.
- Users who want no-prompt local storage run
  `smith setup credential --provider <name>` and select config storage.
- The transaction replaces the provider's reference with `api_key`, then rolls
  the exact config bytes back if full runtime preflight fails.
- No automatic migration reads a Keychain secret and copies it to disk.

## Open Questions

- A dedicated credential file is deferred to the later bootstrap design.
