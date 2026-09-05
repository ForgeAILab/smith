#!/usr/bin/env bash
# Exercise Linux installation without downloading releases or writing outside a temp directory.
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/bin" "$test_dir/archive"
printf '#!/bin/sh\nprintf "smith installer fixture\\n"\n' > "$test_dir/archive/smith"
tar -czf "$test_dir/release.tar.gz" -C "$test_dir/archive" smith

cat > "$test_dir/bin/uname" <<'EOF'
#!/usr/bin/env bash
case "$1" in
    -s) echo Linux ;;
    -m) echo "$TEST_ARCH" ;;
    *) exit 1 ;;
esac
EOF
cat > "$test_dir/bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "$#" = 4 && "$1" = -fsSL && "$3" = -o ]]
[[ "$2" = "https://github.com/ForgeAILab/smith/releases/latest/download/smith-${TEST_ARCH}-linux.tar.gz" ]]
cp "$TEST_ARCHIVE" "$4"
EOF
chmod +x "$test_dir/bin/uname" "$test_dir/bin/curl"

for arch in x86_64 aarch64; do
    destination="$test_dir/install-$arch/bin"
    PATH="$test_dir/bin:$PATH" TEST_ARCH="$arch" TEST_ARCHIVE="$test_dir/release.tar.gz" \
        SMITH_LIBC=musl BINARY_DIR="$destination" bash "$repo_dir/install.sh"
    [[ -x "$destination/smith" ]]
    [[ "$("$destination/smith")" = "smith installer fixture" ]]
done
