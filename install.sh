#!/usr/bin/env bash
set -euo pipefail

REPO="ForgeAILab/smith"
PREFIX="${PREFIX:-/usr/local}"
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

if [ "$OS" = "linux" ]; then
    LIBC="${SMITH_LIBC:-}"
    if [ -z "$LIBC" ] && command -v ldd >/dev/null 2>&1; then
        LDD_OUTPUT="$(ldd --version 2>&1 || true)"
        if printf '%s' "$LDD_OUTPUT" | grep -qi musl; then
            LIBC="musl"
        fi
    fi

    case "$LIBC" in
        musl)
            echo "Error: Smith release binaries currently require glibc Linux; musl is not supported yet." >&2
            echo "Build from source instead: cargo install --path crates/smith-cli" >&2
            exit 1
            ;;
        gnu|"") LIBC="gnu" ;;
        *) echo "Error: unsupported SMITH_LIBC '$LIBC' (expected 'gnu' or 'musl')" >&2; exit 1 ;;
    esac
fi

ARTIFACT="smith-${ARCH}-${OS}"
URL="https://github.com/${REPO}/releases/latest/download/${ARTIFACT}.tar.gz"

echo "    OS:   ${OS}"
echo "    Arch: ${ARCH}"
if [ "$OS" = "linux" ]; then
    echo "    Libc: ${LIBC}"
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
    echo "    (requires sudo for ${PREFIX})"
fi

$install_mode mkdir -p "$BINARY_DIR"
$install_mode install -m 755 "${TMP_DIR}/smith" "${BINARY_DIR}/smith"

echo "==> Installed:"
echo "    smith -> ${BINARY_DIR}/smith"
echo ""
echo "Run 'smith --help' to get started."
