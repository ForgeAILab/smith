# Release blocker verification — 2026-09-05

Smith uses Agent Runtime commit `93f476bc6e6cbb3e73deef3dde065d132e425d34`
from branch `fix/smith-command-provider-compat`. All eight resolved runtime
packages have that exact Git source; no package comes from a sibling path.
The branch backports bounded command providers and hardened fetch onto Smith's
existing runtime baseline, preserving semantic-summary APIs and manifest v1.
It also backports syntax-only fixes for the runtime's declared Rust 1.86 MSRV.
The corresponding runtime-main framework/fetch commit is `da9fbf99a41eb44411869ff22b0d9cba43a76713`
on `fix/smith-runtime-release-blockers`; Smith intentionally uses the compatible
backport pending a separately scoped LCM migration. Neither branch is merged.

## Checks

| Check | Result |
| --- | --- |
| Smith isolated checkout: `cargo test --workspace --all-features --locked` | 1,500 passed; 6 ignored |
| Runtime backport: same all-feature workspace suite | 1,177 passed; 1 ignored; includes Smith consumer conformance |
| Both: `cargo fmt --all -- --check` | Pass |
| Both: `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Pass |
| Both: `cargo deny --locked check all` | Pass (duplicate-version warnings remain non-fatal) |
| Smith: Rust 1.88 `check -p smith-runtime --all-targets --locked` | Pass |
| Runtime: Rust 1.86 facade/provider check with `command-provider` | Pass |
| `bash scripts/test-installer.sh` | Both Linux architecture selections install a local fixture successfully |
| `npm test` in `npx-cli` | 4 passed |
| Strict validation of affected spec changes | Pass |

## Regression evidence

Before the fetch fix, the added tests accepted `http://127.1/` and panicked
when slicing a Chinese HTTP error body at byte 1000. They pass after canonical
URL parsing and UTF-8 boundary truncation. Coverage includes alternate numeric
IPv4, percent-encoded hosts, fragments, mapped IPv6, malformed authorities,
invalid UTF-8, character-count output limits, and pagination overflow.

The original snapshot-v1 fixture is unchanged. The existing write golden and
new load/re-save regression pass with one homogeneous runtime revision;
history, counters, usage, manifest versions, and audit fingerprints survive.

The obsolete musl-rejection assertion is replaced with offline x86_64 and
aarch64 installer tests using exact release URLs and temporary destinations.

## Remaining release checks

Real Linux process execution and live-provider integration remain unverified.
The installer tests simulate Linux platform selection on macOS; they do not
claim Linux binary execution. This record covers the implementation prepared for Smith's command-provider
commit. The runtime's separate CLI/MCP work remains outside Smith's pinned
compatibility backport.
The original ignored Cargo patch is backed up at
`.git/smith-cargo-config.toml.sibling-backup` and is not active.

## Release-prep addendum (2026-09-05)

The pinned revision now carries the annotated upstream tag
`smith-baseline-v0.1.0`. It was previously reachable only from
`fix/smith-command-provider-compat`, so retiring that branch would have made
`cargo build --locked` unresolvable; the tag removes that dependency on a
working branch.

Neither unmerged branch has anything left to contribute. Upstream `main`
carries the same command-provider and hardened-fetch work as `b2fbd9f`, which
shares parent `f369b9e` with `fix/smith-runtime-release-blockers`'s `da9fbf9`
and is its superset. `fix/smith-command-provider-compat` is a backport of that
work onto Smith's pre-LCM baseline. Repinning Smith to `main` was attempted
and does not compile: the semantic-summary surface is renamed and restructured
to LCM (`SemanticSummaryCoordinator` -> `LcmCoordinator`, `SummaryModel` ->
`LcmSummaryModel`, `SEMANTIC_SUMMARY_*` -> `LCM_*`), and
`protected_semantic_summary_from_state` is `pub(crate)` upstream, leaving no
crate-public legacy import path. The surface is 83 references across 11 files,
including the 509-line `summary.rs` and the resume-capsule persistence path.
That migration stays separately scoped.

A detached worktree at the release commit builds
`cargo build -p smith-cli --release --locked` with no sibling checkout and no
Cargo patch, and the binary reports `smith 0.1.0`.
