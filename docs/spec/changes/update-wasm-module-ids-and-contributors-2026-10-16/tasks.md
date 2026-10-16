---
created_at: 2026-10-16T23:20:00Z
updated_at: 2026-10-16T22:36:45Z
completed_at:
---

## 1. Implementation
- [x] 1.1 Readable module ids `<plugin>/<path>` when representable, falling
      back to the hashed form. Port from `663a5ed`.
- [x] 1.2 The hashed id resolves as an alias wherever a module id is looked
      up: install records, capability grants, enable/disable and module
      configuration, `smith config explain`. Listings and diagnostics show
      the readable id.
- [x] 1.3 A context contributor without the session-history grant runs with
      an empty history.
- [x] 1.4 An unfittable view skips the contribution and is not counted as a
      module failure.
- [x] 1.5 `docs/wasm-modules.md` describes ids, the alias, and both
      contributor rules.

## 2. Tests
- [x] 2.1 Id derivation, readable and fallback.
- [x] 2.2 An install recorded under the hashed id still loads and keeps its
      grants after upgrade, and a config entry naming the hashed id applies.
- [x] 2.3 A contributor without history sees an empty history and its tools
      stay available.
- [x] 2.4 An oversized history skips the contribution and never disables the
      module.

## 3. Verification
- [x] 3.1 Full gate.
