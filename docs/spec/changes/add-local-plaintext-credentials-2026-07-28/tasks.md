---
created_at: 2026-07-29T02:05:43Z
updated_at: 2026-07-29T03:13:32Z
completed_at: 2026-07-29T03:13:32Z
---

## 1. Credential model and resolution

- [x] 1.1 Add a secret-bearing `api_key` provider field that is accepted only
  from owner-only user configuration and is mutually exclusive with
  `credential`.
- [x] 1.2 Carry inline values through redaction-safe file, provenance, resolved
  configuration, explanation, and runtime types without consulting the
  platform credential service.
- [x] 1.3 Add adversarial tests proving `Debug`, errors, inventory, runtime
  events, journals, snapshots, setup previews, and machine output never reveal
  inline values.

## 2. Setup and migration

- [x] 2.1 Add an explicit “Store in config (no prompts)” authentication choice
  with plaintext-at-rest and backup warnings plus masked input.
- [x] 2.2 Publish the secret-bearing user config through a restrictive
  same-directory atomic replace, redact collision review, and restore exact
  prior config bytes on failure.
- [x] 2.3 Add `smith setup credential --provider <name>` to migrate only an
  existing provider credential and preflight the unchanged provider/model
  selection before committing.

## 3. Verification and documentation

- [x] 3.1 Add unit, transaction, headless, and pseudo-terminal coverage for
  fresh setup, manual inline config, and migration without Keychain prompts.
- [x] 3.2 Document Keychain, environment, local plaintext, and future encrypted
  storage trade-offs plus rotation/backup guidance.
- [x] 3.3 Run strict spec validation, formatting, warning-denied Clippy,
  workspace tests, credential-redaction conformance, CodeGraph sync, and
  reinstall Smith.
