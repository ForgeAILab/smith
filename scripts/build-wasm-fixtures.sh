#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export TMPDIR="${TMPDIR:-/private/tmp/smith-fixture-tmp}"
mkdir -p "$TMPDIR"
export CARGO_HOME="${CARGO_HOME:-$PWD/target/wasm-gate/cargo-home}"
export CARGO_TARGET_DIR="$PWD/target"
manifest=crates/smith-wasm-host/tests/guests/Cargo.toml
fixtures=crates/smith-wasm-host/tests/fixtures
# Rebuild legacy fixtures with frozen 0.1 sources, and current fixtures with 0.2.
mkdir -p "$fixtures/v0.2"
cargo +stable build --manifest-path "$manifest" -p smith-fixture-componentize --locked
for mode in tools context context-tools processor inactive mount-error malformed collision mount-http quiet; do
    if [[ "$mode" == tools ]]; then
        cargo +stable build --manifest-path "$manifest" -p smith-conformance-guest-v01 --target wasm32-unknown-unknown --release --locked
    else
        cargo +stable build --manifest-path "$manifest" -p smith-conformance-guest-v01 --target wasm32-unknown-unknown --release --locked --features "$mode"
    fi
    "$CARGO_TARGET_DIR/debug/smith-fixture-componentize" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/smith_conformance_guest_v01.wasm" "$fixtures/$mode.wasm"
done
for mode in tools context context-tools processor inactive mount-error malformed collision mount-http quiet complete watcher poll-oversize poll-many poll-unknown poll-loop watcher-duplicate watcher-invalid; do
    if [[ "$mode" == tools ]]; then
        cargo +stable build --manifest-path "$manifest" -p smith-conformance-guest --target wasm32-unknown-unknown --release --locked
    else
        cargo +stable build --manifest-path "$manifest" -p smith-conformance-guest --target wasm32-unknown-unknown --release --locked --features "$mode"
    fi
    "$CARGO_TARGET_DIR/debug/smith-fixture-componentize" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/smith_conformance_guest.wasm" "$fixtures/v0.2/$mode.wasm"
done
for fixture in diagnostic wasi wasi-network wasi-environment wasi-clock unsupported; do
    "$CARGO_TARGET_DIR/debug/smith-fixture-componentize" "$fixtures/$fixture.wat" "$fixtures/$fixture.wasm"
done
