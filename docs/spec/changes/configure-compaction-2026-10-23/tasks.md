---
created_at: 2026-10-23T11:00:31Z
updated_at: 2026-10-23T12:02:16Z
completed_at: 2026-10-23T11:00:31Z
---

# Tasks

- [x] Resolve compaction model/instructions and model-specific inactivity with provenance and validation.
- [x] Compose an independently attributed summary route and configurable instructions; retain one-attempt overrides.
- [x] Extend `/compact` grammar and atomic persistent configuration edits with idle-safe application.
- [x] Verify session reconstruction, protected summary compatibility, failure rollback, and model-specific timing.
- [x] Document the settings and commands; update fixtures and run relevant checks.

- [x] Restore decimal rate-limit journal recovery exposed by live ChatGPT cache testing, with a regression test.
- [x] Keep cold schema validation off the nested async startup stack and verify ordinary model switching.
- [x] Keep model and host source files within the architecture line budget by extracting focused child modules.
- [x] Apply automatic summary defaults at startup using the final persistence setting, including a no-artifact regression check.

## Verification

- Release gate passes: `TMPDIR=/private/tmp RUST_TEST_THREADS=4 bash scripts/ci.sh`, with the clean Agent Runtime checkout pinned to `bba8996c213f2f65287d84befc1c7dc6ced3241a`. This covers workspace formatting, clippy with `-D warnings`, all workspace tests, and the runtime consumer-conformance suite.
- The final CLI checks pass all 415 unit tests and 139 integration tests. All 50 hosted-session tests pass, including independent summary routes, instructions, output caps, changed-default resume, and automatic idle compaction.
- Both tool-output context tests pass, including the final persistence override without an artifact store. All 17 architecture checks pass after extracting focused child modules.
- Dependency policy passes after updating locked `ctutils` from 0.4.2 to 0.4.4 for RUSTSEC-2026-0385. The patched package requires Rust 1.87, within Smith's Rust 1.88 minimum.
- The seven NPX bootstrapper tests and installer smoke checks for both Linux architectures pass.
- The optimized executable reports `smith 0.7.13`; installed configuration-explain checks confirm the summary model, instructions, enabled idle compaction, persistence, and model-specific cutoff.
- Live cache sweep: `docs/qa/cache-retention-2026-10-23/summary.json` records completed 0/1/5/15-minute observations and limitations for every configured route. These are observed lower bounds, not guaranteed provider TTLs.
- External-volume temporary storage caused wall-clock fixture timeouts. The final gate uses internal temporary storage and four test threads; no assertions or fixture deadlines were relaxed.
