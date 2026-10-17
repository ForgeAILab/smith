---
created_at: 2026-10-11T21:09:05Z
updated_at: 2026-10-16T23:30:00Z
completed_at:
---

## 1. Interface

- [x] 1.1 Define `smith:module@0.2.0`: `complete` import and its types, the
  `watcher` contribution variant, the `poll` export, and the notice type.
- [x] 1.2 Load 0.1 components through an adapter; conformance fixtures for
  both versions and for an unsupported version.
- [x] 1.3 SDK support and builders for the new functions.

## 2. Model calls

- [ ] 2.0 Build `complete` on the agent-runtime one-shot call API once the
  runtime release that adds it is pinned.

- [ ] 2.1 A model service on `ModuleContext`, built from the session's
  provider route, with no credential exposed to the module.
- [ ] 2.2 `model` capability: manifest key, install and update review text,
  listings, user narrowing.
- [ ] 2.3 Per-call and per-turn bounds, the separate deadline, and the
  phases in which `complete` is refused.
- [ ] 2.4 Usage attribution to the module id on every usage surface.

## 3. Watchers

- [ ] 3.1 `watcher` contribution in `smith-module`, refused at mount without
  `session_notify`.
- [ ] 3.2 Host-owned poll schedule, bounds, and serialisation with other
  calls; root sessions only; none after a headless turn ends.
- [ ] 3.3 Delivery through the session inbox with monitor flood limits and
  untrusted-text framing; TUI and headless event surfaces.

## 4. Examples and docs

- [ ] 4.1 Relay example delivers with a watcher.
- [ ] 4.2 `second-opinion` example using `complete` and `session_history`.
- [ ] 4.3 `docs/wasm-modules.md`, CI build lines, end-to-end tests with the
  fake provider.

## 5. Gate

- [ ] 5.1 fmt, clippy, full tests with `--no-fail-fast`, feature-free build.
