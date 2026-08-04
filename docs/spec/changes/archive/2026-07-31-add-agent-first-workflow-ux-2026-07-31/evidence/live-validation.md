# Local live validation

Completed at `2026-08-01T00:44:17Z` on `Darwin arm64`.

## Provenance

- Agent Runtime commit: `50f230e4ca8ba318fd4c5c84df0876051c54d93e`.
- Smith/TUI benchmark commit: `06206de2962df9511d5cc6bd9dd9f5e0078cf404`.
- Smith: `smith 0.0.1`.
- Profile: `zai-glm-5-2`.
- Provider/model: `zai` / `glm-5.2`.
- Model request ceiling: `32768` output tokens.
- Local toolchains: Rust `1.93.0`; MSRV gate Rust `1.88.0`.

Configuration explanation reported the provider API key only as `[redacted]`
from the owner-only user configuration. The durability run selected the
`SMITH_CHECKPOINT_KEY` environment source. Neither run selected a Keychain or
Secret Service reference.

## Repository gates

Both workspaces passed:

- `cargo fmt --all -- --check`;
- warning-denied Clippy for all workspace targets and features;
- `cargo test --workspace`;
- Rust 1.88 workspace checks for all targets and features;
- `git diff --check`;
- strict change-spec validation;
- `cargo deny check` (advisories, bans, licenses, and sources all OK); and
- `cargo audit` against 1,177 loaded RustSec advisories with no vulnerability.

## Disposable coding benchmark

Retained local run:
`/var/folders/17/5r3gw10108g2h5wgw9hl1wqc0000gn/T/smith-agent-workflow.chRgbPgR`.

- The broken baseline reproduced three failing tests.
- Smith returned schema-v2 status `ok` from `zai` / `glm-5.2`.
- Provider-reported usage was 78,959 uncached input, 163,904 cached input, and
  15,171 output tokens.
- Activation included exact read/edit/shell tools, todo planning, registry
  search, and delegation.
- The terminal plan contained six completed items, with zero pending,
  in-progress, or cancelled items.
- The agent changed only `src/scheduler.rs` (53 insertions, 17 deletions).
- Independent `cargo fmt --check`, all six public/adversarial tests, and
  warning-denied Clippy passed after the run.
- The same-model read-only review completed and its documentation clarification
  was applied before the gates were rerun.
- No `.smith`, `.omo`, session, timeline, or child-control metadata appeared in
  the project checkout.

## Durable no-Keychain recovery

Retained local run:
`/var/folders/17/5r3gw10108g2h5wgw9hl1wqc0000gn/T/smith-durable-pty.6jsHjC6M`.

Smith was killed after the exact shell call had reached an encrypted
`AwaitingApproval` checkpoint and before the tool executed. The same terminal
surface resumed that session with the same environment-provided key and
approved the restored action once.

- Original provider request records: `1`.
- Successful shell completion records: `1`.
- Marker lines written by the shell: `1`.
- Recovered turn completion records: `1`.

The deterministic resolver-count tests also prove inline/environment
checkpoint sources make zero credential-service calls.

## Installed artifact

- Path: `/Users/mai1015/.cargo/bin/smith`.
- SHA-256:
  `c7452e614464cc6cae4990f84943625eda34c005d7495a67994495104417f690`.
- The installed file is byte-identical to `target/release/smith`.
- Installed resolution still reports `glm-5.2` and a `32768` output-token
  request ceiling.

Hosted macOS CI and hosted Linux release jobs were not available in this local
workspace and remain explicit external release gates.
