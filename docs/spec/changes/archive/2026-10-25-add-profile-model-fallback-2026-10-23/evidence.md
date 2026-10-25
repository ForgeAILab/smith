# Verification

Implementation verified on 2026-10-23.

## Dependency

Smith pins all nine shared runtime crates to published commit
[`a5f1b1788e982c39e7c3a394c715a7d92cc7852b`](https://github.com/ForgeAILab/agent-runtime/commit/a5f1b1788e982c39e7c3a394c715a7d92cc7852b).
The implementation branch is `feat/profile-model-fallback`, based on Smith's
previous v0.7.0 pin rather than the sibling runtime's newer main branch.

The ignored development override was removed. `cargo metadata --locked`
confirms that all nine runtime crates resolve from Cargo's Git checkout of
that commit, with no sibling source paths. The lockfile changes only those
nine Git source revisions.

## Smith checks

The full five-package suite passed: 2,875 tests, 14 existing ignored tests or
doctests. Clippy passed for all targets with warnings denied. Formatting and
whitespace checks passed.

```sh
cargo test -p smith-config -p smith-client -p smith-runtime -p smith-tui -p smith-cli --locked --quiet
cargo clippy -p smith-config -p smith-client -p smith-runtime -p smith-tui -p smith-cli --all-targets --locked -- -D warnings
cargo fmt --all --check
git diff --check
```

Focused fallback checks were also run against the published Git dependency
after removing the override: five runtime-composition tests and two CLI tests
passed, as did the locked build check and final Clippy run.

```sh
cargo test -p smith-runtime --test composition fallback --locked --quiet
cargo test -p smith-cli --bin smith fallback --locked --quiet
cargo check -p smith-cli --locked
cargo metadata --locked --format-version 1
```

Coverage includes eight configuration tests for precedence, list replacement,
empty-list opt-out, exact model selection, per-candidate controls, reasoning
table replacement, invalid bindings/timeouts, and source diagnostics. Five
factory-composition tests cover lazy credentials, metadata before primary
credential access, unavailable sources, startup switching, and buffered
reasoning progress. Headless, client, and TUI tests cover actual binding
reporting, typed switches, preserved profile identity, unknown abandoned usage,
child model prices, and stale event handling. Updated built-in skill fixtures
and the embedded reference token cap also pass.

## Shared runtime checks

The runtime/core/provider/obs/testkit suite passed 1,670 tests with
`external-agent-bridge` and `command-provider` enabled. The final fallback
target passed 21 deterministic tests; the final runtime unit target passed
367 tests and the Smith consumer-conformance target passed 10 tests. Clippy,
formatting, whitespace checks, and strict shared change validation passed.
See the runtime commit's
[verification evidence](https://github.com/ForgeAILab/agent-runtime/blob/a5f1b1788e982c39e7c3a394c715a7d92cc7852b/docs/spec/changes/add-model-fallback-2026-10-23/evidence.md).

The tests cover open/empty/usage-only streams, retries/backoff sharing the
startup budget, timeout overrides, text/reasoning/tool progress, retired late
output, hosted-side-effect replay fences, cancellation and turn deadlines,
finite exhaustion, smaller windows and text-only candidates, tool-result
continuation, host interceptors, cache registry retention, and checkpoint
recovery. Frozen transition-table-4 behavior remains intact; new fallback
turns opt into table 5. No live-provider requests were needed.

## Proposal validation

```sh
python3 ~/.codex/skills/spec-toolkit/scripts/spec_toolkit.py validate add-profile-model-fallback-2026-10-23 --type change --strict
```

## Review follow-ups (2026-10-23)

- Child turns now receive the chain. An inheriting child gets the root's
  prepared routes, timeout, and backoff observers; each child profile route
  prepares its own chain. Before this, a child shared the parent's pooled
  provider, which defers quota rotation whenever a chain exists, but ran with
  no chain, so child quota exhaustion failed without a rotation offer.
  `delegation::fallback::an_inherited_child_falls_back_along_the_profile_chain`
  fails without the fix and passes with it. Child profile chains are
  preflighted at startup, so invalid candidate metadata fails the root start,
  as it already does for a child's primary.
- `GeminiContinuationProvider` forwards `pre_output_replay_safe`,
  `prepare_request`, `requires_nonempty_assistant_content`, and
  `cache_resource_provider`. The trait default had made every Gemini primary
  replay-unsafe, so it could neither fall back nor rotate reactively.
- Interactive catalog adoption no longer pins provider/model when a chain is
  active; the pin counted as an explicit selection and disabled the chain for
  the rest of the session.
- The Rust 1.88 `nonminimal_bool` lint in `resolve/fallback.rs` is fixed.
