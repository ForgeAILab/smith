#!/usr/bin/env bash
set -euo pipefail

runtime_dir="${SMITH_AGENT_RUNTIME_DIR:-../agent-runtime}"
runtime_manifest="${runtime_dir}/Cargo.toml"

if [[ ! -f "${runtime_manifest}" ]]; then
    echo "smith-ci: Agent Runtime is missing at ${runtime_manifest}" >&2
    echo "smith-ci: set SMITH_AGENT_RUNTIME_DIR to the tested checkout" >&2
    exit 2
fi

if [[ -n "${SMITH_AGENT_RUNTIME_REVISION:-}" ]]; then
    if [[ ! "${SMITH_AGENT_RUNTIME_REVISION}" =~ ^[0-9a-f]{40}$ ]]; then
        echo "smith-ci: SMITH_AGENT_RUNTIME_REVISION must be a full lowercase commit SHA" >&2
        exit 2
    fi
    actual_revision="$(git -C "${runtime_dir}" rev-parse HEAD)"
    if [[ "${actual_revision}" != "${SMITH_AGENT_RUNTIME_REVISION}" ]]; then
        echo "smith-ci: runtime is ${actual_revision}, expected ${SMITH_AGENT_RUNTIME_REVISION}" >&2
        exit 2
    fi
    if [[ -n "$(git -C "${runtime_dir}" status --porcelain)" ]]; then
        echo "smith-ci: the pinned runtime checkout has uncommitted changes" >&2
        exit 2
    fi
fi

# A pinned toolchain gates on warnings. A floating one reports them instead:
# clippy grows lints on its own six-week schedule, and a leg tracking `stable`
# that denies warnings turns main red for a Rust release rather than for
# anything anyone changed. The pinned legs still fail the build, so nothing
# stops being enforced -- the floating leg is where upcoming lints get read
# before they are binding.
cargo fmt --all -- --check
if [[ "${SMITH_CI_CLIPPY_WARNINGS_ONLY:-}" == "true" ]]; then
    cargo clippy --workspace --all-targets --locked
else
    cargo clippy --workspace --all-targets --locked -- -D warnings
fi
cargo test --workspace --locked
cargo test \
    --manifest-path "${runtime_manifest}" \
    --package agent-runtime-testkit \
    --locked
