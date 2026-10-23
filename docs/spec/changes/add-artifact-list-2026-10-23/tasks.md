---
created_at: 2026-10-23T21:37:12Z
updated_at: 2026-10-23T22:00:00Z
completed_at:
---

## 1. Implementation

- [x] 1.1 Owner-scoped, newest-first metadata listing on `SmithArtifactStore`
  behind an `ArtifactCatalog` trait; skip malformed records. New records carry
  `created_at_ms`; older ones order by file mtime.
- [x] 1.2 `artifact.list` tool: purpose filter, offset/limit paging, metadata
  rows, `artifact.read` permission over a `session-artifact` resource. Pages
  stop before exceeding the inline allowance and are never offloaded.
- [x] 1.3 Carry the catalog from host startup through `RuntimeRequest` to the
  root and child tool sets; both register through one
  `ToolOutputContextPolicy::artifact_tools` constructor, only with a catalog.
- [x] 1.4 Rewrite `artifact not found` in `ContextArtifactReader`, naming
  `artifact.list` only when the same constructor registered it.
- [x] 1.5 Ability keywords and affordance disjoint from the reader's; CLI
  bridge treats `artifact.list` as artifact support, not a supporting tool.
- [x] 1.6 Delegation guidance points a lost ID to `agent result`.
- [x] 1.7 Document the lister in `docs/context-working-set.md`.
- [x] 1.8 `child_id` column and filter: provenance child first, else a copy's
  lineage session resolved through the root's coordinator; child runtimes
  have no directory. `agent result` carries `reply_artifact`.
- [x] 1.9 Pin the runtime attribution change (`62bdc69` on the 0.7 line).

## 2. Tests

- [x] 2.1 Store: owner filtering, newest-first order, malformed record skipped.
- [x] 2.2 Tool: purpose filter, paging, metadata only, invalid limits and
  purposes refused, listing not offloaded.
- [x] 2.3 Reader: invented ID error names `artifact.list`; without a catalog it
  does not; access denial keeps Runtime's error.
- [x] 2.4 Registration: a standard host registers both tools and recovers an
  invented ID end to end through `artifact.list`; a store without a catalog
  registers only the reader; no store registers neither; each discovery query
  selects only its own tool. Child registration shares the root constructor
  and has no separate test.
- [x] 2.5 CLI bridge: artifact tools alone are not bridged.
- [x] 2.6 Lister attributes stored replies and copies to their child and
  filters by `child_id`; end to end, a long child reply is listed by its child
  and `agent result` names the same reply artifact.

## 3. Verification

- [x] 3.1 `smith-runtime` full suite, workspace Clippy `-D warnings`, fmt,
  `smith-cli` contract suite.
- [x] 3.2 Headless golden fixtures regenerated; the only change is the longer
  delegation prompt (+127 developer-instruction tokens).
- [x] 3.3 Full gate: 3,245 passed with only the known
  `metadata_adoption_waits_for_open_picker...` failure; Rust 1.88 Clippy,
  `--no-default-features`, smoke, and `cargo deny` (after the tokio and
  ctutils lockfile bumps) pass.
