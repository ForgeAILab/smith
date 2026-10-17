#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
export TMPDIR="${TMPDIR:-/tmp/smith-symlink-tmp}"
mkdir -p "$TMPDIR"
export CARGO_HOME="${CARGO_HOME:-$PWD/target/wasm-gate/cargo-home}"
export CARGO_TARGET_DIR="$PWD/target/wasm-fixtures"
cargo +stable build --manifest-path crates/smith-wasm-host/tests/guests/Cargo.toml -p smith-fixture-componentize --locked
cargo +stable build --manifest-path examples/wasm-module-hello/Cargo.toml --target wasm32-unknown-unknown --release --locked
"$CARGO_TARGET_DIR/debug/smith-fixture-componentize" "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/smith_wasm_module_hello.wasm" examples/wasm-module-hello/module.wasm
