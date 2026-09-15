#!/usr/bin/env bash
set -euo pipefail

REPO="ForgeAILab/smith"
RELEASE="${SMITH_RELEASE:-latest}"

# Smith installs into the user's own prefix by default so no step needs sudo.
# Override with --prefix/--dir, the PREFIX/BINARY_DIR environment variables, or
# --system for a machine-wide install under /usr/local.
DEFAULT_PREFIX="${HOME}/.local"
PREFIX="${PREFIX:-$DEFAULT_PREFIX}"
BINARY_DIR="${BINARY_DIR:-}"

usage() {
    cat <<'EOF'
Smith installer

Usage:
  install.sh [options]
  curl -fsSL https://raw.githubusercontent.com/ForgeAILab/smith/main/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/ForgeAILab/smith/main/install.sh | bash -s -- --system

Options:
  --prefix <dir>     Install into <dir>/bin (default: ~/.local)
  --dir <dir>        Install straight into <dir>
  --system           Install into /usr/local/bin (may prompt for sudo)
  --release <tag>    Install a specific release tag instead of latest
  -h, --help         Show this help

Environment:
  PREFIX, BINARY_DIR, SMITH_RELEASE are honored when the flags are omitted.
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || { echo "Error: --prefix requires a directory" >&2; exit 1; }
            PREFIX="$2"; BINARY_DIR=""; shift 2 ;;
        --prefix=*)
            PREFIX="${1#*=}"; BINARY_DIR=""; shift ;;
        --dir)
            [ $# -ge 2 ] || { echo "Error: --dir requires a directory" >&2; exit 1; }
            BINARY_DIR="$2"; shift 2 ;;
        --dir=*)
            BINARY_DIR="${1#*=}"; shift ;;
        --system)
            PREFIX="/usr/local"; BINARY_DIR=""; shift ;;
        --release)
            [ $# -ge 2 ] || { echo "Error: --release requires a tag" >&2; exit 1; }
            RELEASE="$2"; shift 2 ;;
        --release=*)
            RELEASE="${1#*=}"; shift ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            echo "Error: unknown option '$1'" >&2
            usage >&2
            exit 1 ;;
    esac
done

BINARY_DIR="${BINARY_DIR:-${PREFIX}/bin}"

TMP_DIR="$(mktemp -d)"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT

can_write_target() {
    local dir="$1"
    while [ ! -e "$dir" ]; do
        dir="$(dirname "$dir")"
    done
    [ -w "$dir" ]
}

echo "==> Smith Installer"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$OS" in
    linux)  OS="linux" ;;
    darwin) OS="macos" ;;
    *)      echo "Error: unsupported OS '$OS'" >&2; exit 1 ;;
esac

case "$ARCH" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *)             echo "Error: unsupported architecture '$ARCH'" >&2; exit 1 ;;
esac

ARTIFACT="smith-${ARCH}-${OS}"
if [ "$RELEASE" = "latest" ]; then
    URL="https://github.com/${REPO}/releases/latest/download/${ARTIFACT}.tar.gz"
else
    URL="https://github.com/${REPO}/releases/download/${RELEASE}/${ARTIFACT}.tar.gz"
fi

echo "    OS:   ${OS}"
echo "    Arch: ${ARCH}"
if [ "$OS" = "linux" ]; then
    echo "    Libc: static musl (portable)"
fi
echo "    Fetching: ${URL}"

if ! curl -fsSL "$URL" -o "${TMP_DIR}/${ARTIFACT}.tar.gz"; then
    echo "Error: failed to download ${URL}" >&2
    echo "There may not be a release for this platform yet." >&2
    echo "Build from source instead:  cargo install --path crates/smith-cli" >&2
    exit 1
fi

tar -xzf "${TMP_DIR}/${ARTIFACT}.tar.gz" -C "$TMP_DIR"

echo "==> Installing smith to ${BINARY_DIR}"

install_mode=""
if ! can_write_target "$BINARY_DIR"; then
    install_mode="sudo"
    echo "    (${BINARY_DIR} is not writable; using sudo)"
fi

$install_mode mkdir -p "$BINARY_DIR"
$install_mode install -m 755 "${TMP_DIR}/smith" "${BINARY_DIR}/smith"

echo "==> Installed:"
echo "    smith -> ${BINARY_DIR}/smith"

case ":${PATH}:" in
    *":${BINARY_DIR}:"*)
        echo ""
        echo "Run 'smith --help' to get started."
        ;;
    *)
        echo ""
        echo "${BINARY_DIR} is not on your PATH. Add it:"
        echo "    export PATH=\"${BINARY_DIR}:\$PATH\""
        echo "Then run 'smith --help' to get started."
        ;;
esac
