#!/usr/bin/env bash
set -euo pipefail

usage() {
  command printf '%s\n' \
    'usage: scripts/eval-agent-workflow.sh [--live] [--keep] [--profile NAME]' \
    '' \
    'Creates a fresh broken stable-ready-queue project under a private temp' \
    'directory. --live runs Smith with explicit allow-all authority inside that' \
    'disposable project. Evidence is stored beside the project, never in it.'
}

live=0
keep=0
profile='zai-glm-5-2'
while (($# > 0)); do
  case "$1" in
    --live)
      live=1
      ;;
    --keep)
      keep=1
      ;;
    --profile)
      shift
      if (($# == 0)); then
        usage >&2
        exit 2
      fi
      profile="$1"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
  shift
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"
fixture_dir="${repo_root}/crates/smith-cli/tests/fixtures/stable-ready-queue"
smith_bin="${SMITH_BIN:-${repo_root}/target/debug/smith}"
run_root="$(mktemp -d "${TMPDIR:-/tmp}/smith-agent-workflow.XXXXXXXX")"
project_dir="${run_root}/project"
evidence_dir="${run_root}/evidence"

cleanup() {
  if ((keep == 0)); then
    command rm -rf -- "${run_root}"
  fi
}
trap cleanup EXIT

command mkdir -p -- "${project_dir}" "${evidence_dir}"
command cp -R -- "${fixture_dir}/." "${project_dir}/"
git -C "${project_dir}" init -q
git -C "${project_dir}" add --all
git -C "${project_dir}" \
  -c user.name='Smith Evaluation' \
  -c user.email='smith-evaluation@invalid' \
  commit -q -m 'test fixture: broken stable scheduler'

if cargo test --manifest-path "${project_dir}/Cargo.toml" \
  >"${evidence_dir}/baseline-test.txt" 2>&1; then
  command printf '%s\n' 'fixture unexpectedly passed before Smith ran' >&2
  exit 1
fi

{
  command printf 'smith_version=%s\n' "$("${smith_bin}" --version)"
  command printf 'smith_revision=%s\n' "$(git -C "${repo_root}" rev-parse HEAD)"
  command printf 'profile=%s\n' "${profile}"
  command printf 'reviewer_model_policy=parent_inherited\n'
  command printf 'fixture_revision=%s\n' "$(git -C "${project_dir}" rev-parse HEAD)"
} >"${evidence_dir}/provenance.txt"

if ((live == 0)); then
  command printf 'fixture baseline reproduced at %s\n' "${run_root}"
  command printf '%s\n' 'pass --live to spend provider quota and run Smith'
  exit 0
fi

prompt='Fix this crate as a production-quality coding task. First use write_todos to make a concise plan. Inspect the README, public API, implementation, and tests; diagnose the failing behavior without weakening or deleting tests. Implement the documented stable batching and validation contract. Run cargo fmt --check, cargo test, and cargo clippy --all-targets -- -D warnings. After the tests pass, use the agent tool to delegate a read-only correctness review focused on ordering, error precedence, and cycles; address any concrete finding. Finish with a concise evidence-backed summary of files changed and exact validation results.'

SMITH_PERSISTENCE_ENABLED="${SMITH_PERSISTENCE_ENABLED:-false}" \
  "${smith_bin}" \
  --prompt "${prompt}" \
  --project "${project_dir}" \
  --profile "${profile}" \
  --approval allow-all \
  --output-format json \
  --no-color \
  --no-motion \
  >"${evidence_dir}/smith-result.json"

cargo fmt --manifest-path "${project_dir}/Cargo.toml" --all -- --check \
  >"${evidence_dir}/fmt.txt" 2>&1
cargo test --manifest-path "${project_dir}/Cargo.toml" \
  >"${evidence_dir}/test.txt" 2>&1
cargo clippy --manifest-path "${project_dir}/Cargo.toml" --all-targets -- -D warnings \
  >"${evidence_dir}/clippy.txt" 2>&1
git -C "${project_dir}" diff --check
git -C "${project_dir}" status --short >"${evidence_dir}/project-status.txt"

for forbidden in .smith .omo .agents sessions timeline children; do
  if find "${project_dir}" -name "${forbidden}" -print -quit | grep -q .; then
    command printf 'Smith created forbidden project-local metadata: %s\n' \
      "${forbidden}" >&2
    exit 1
  fi
done

command printf 'agent workflow evaluation passed at %s\n' "${run_root}"
if ((keep == 0)); then
  command printf '%s\n' 'use --keep to retain the disposable project and evidence'
fi
