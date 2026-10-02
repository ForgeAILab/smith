# Verification: Let a flat model limit outrank a built-in default window

- Regression tests in `crates/smith-runtime/tests/context_windows.rs`:
  flat limits over the trusted default resolve to 872000 / 828400 / 128000;
  an explicit `872k` over the same flat limit still fails naming the flat
  key; without flat limits the trusted `272k` default is still active. The
  first test failed before the fix with the original startup error.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, and `cargo test --workspace --locked` pass at
  `0f5fc48`.
- The release build starts against the owner's real configuration: the
  default `code` profile opens, and `--profile sol` reports an 872k window
  with an 828.4k input budget in `/context`.
- Release workflow run 37075984412 succeeded: six platform archives,
  SHA256SUMS, both portable smoke tests, and npm publication of
  `@forgeailab/smith@0.2.16`.
- Installed locally as `~/.local/bin/smith` (0.2.15 kept as a backup).
