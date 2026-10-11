#!/usr/bin/env bash
set -euo pipefail
export PATH=/usr/bin:/bin

smoke_repo="$(CDPATH= cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
smoke_root="$(mktemp -d /tmp/smith-minimal.XXXXXX)"
trap 'rm -rf "${smoke_root}"' EXIT
smoke_home="${smoke_root}/home"
smoke_project="${smoke_root}/project"
mkdir -p "${smoke_home}/.smith" "${smoke_project}/.smith"
cp "${smoke_repo}/crates/smith-cli/tests/fixtures/minimal-build/config.toml" "${smoke_project}/.smith/config.toml"
cd "${smoke_project}"

# Drop inherited Smith overrides and credential variables along with the real
# user configuration. Use the exact binary built by the caller's build gate.
smoke_smith() {
    env -i HOME="${smoke_home}" PATH=/usr/bin:/bin "${smoke_repo}/target/debug/smith" "$@"
}

smoke_smith --version
listing="$(smoke_smith config modules --project "${smoke_project}")"
printf '%s\n' "${listing}"
grep -F 'image-generation · not built' <<<"${listing}"
grep -F 'budget-notice · not built' <<<"${listing}"
