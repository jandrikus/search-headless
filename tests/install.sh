#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/fake-bin" "$work/archive" "$work/home"

cat > "$work/fake-bin/obscura" <<'EOF'
#!/usr/bin/env bash
printf 'obscura 0.2.6\n'
EOF
cat > "$work/archive/search-headless" <<'EOF'
#!/usr/bin/env bash
printf 'test search-headless\n'
EOF
cat > "$work/fake-bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
url="$2"
output="$4"
case "$url" in
  */SHA256SUMS)
    case "$(uname -s)-$(uname -m)" in
      Linux-x86_64|Linux-amd64) target=x86_64-unknown-linux-gnu ;;
      Darwin-arm64|Darwin-aarch64) target=aarch64-apple-darwin ;;
      *) exit 1 ;;
    esac
    digest="$MOCK_DIGEST"
    if [[ "${MOCK_BAD_HASH:-}" == 1 ]]; then digest="$(printf '%064d' 0)"; fi
    printf '%s  search-headless-%s.tar.gz\n' "$digest" "$target" > "$output"
    ;;
  */search-headless-*.tar.gz) cp "$MOCK_ARCHIVE" "$output" ;;
  *) exit 1 ;;
esac
EOF
chmod 755 "$work/fake-bin/obscura" "$work/fake-bin/curl" "$work/archive/search-headless"
tar -C "$work/archive" -czf "$work/cli.tar.gz" search-headless
if command -v sha256sum >/dev/null 2>&1; then
  digest="$(sha256sum "$work/cli.tar.gz" | cut -d' ' -f1)"
else
  digest="$(shasum -a 256 "$work/cli.tar.gz" | cut -d' ' -f1)"
fi
export HOME="$work/home" PATH="$work/fake-bin:$PATH" MOCK_ARCHIVE="$work/cli.tar.gz" MOCK_DIGEST="$digest"

MOCK_BAD_HASH=1 bash "$root/install.sh" --binary > "$work/bad.log" 2>&1 && { echo 'Accepted a bad checksum' >&2; exit 1; }
test ! -e "$HOME/.local/bin/search-headless"
bash "$root/install.sh" --binary > "$work/good.log" 2>&1
test "$("$HOME/.local/bin/search-headless")" = 'test search-headless'
printf 'Prebuilt installer accepts a verified archive and rejects a bad checksum.\n'
