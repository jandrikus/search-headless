#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUN_BIN_DIR="${HOME}/.bun/bin"

info() {
  printf '\033[1;34m[search-headless]\033[0m %s\n' "$*"
}

warn() {
  printf '\033[1;33m[search-headless]\033[0m %s\n' "$*" >&2
}

fail() {
  printf '\033[1;31m[search-headless]\033[0m %s\n' "$*" >&2
  exit 1
}

command -v bun >/dev/null 2>&1 || fail "Bun is required but was not found on PATH. Install Bun first: https://bun.sh"

cd "$PROJECT_DIR"

info "Installing dependencies with Bun..."
bun install

info "Installing Playwright Chromium if needed..."
bunx playwright install chromium

info "Linking package globally with bun link..."
bun link

if [[ ":${PATH}:" != *":${BUN_BIN_DIR}:"* ]]; then
  warn "${BUN_BIN_DIR} is not currently on PATH."
  warn "Add this to your shell config:"
  warn "  export PATH=\"\$HOME/.bun/bin:\$PATH\""
else
  info "Bun global bin directory is already on PATH."
fi

info "Verifying linked commands..."
if command -v search-web >/dev/null 2>&1; then
  info "search-web -> $(command -v search-web)"
else
  warn "search-web is not visible on PATH in this shell yet. Open a new shell or update PATH."
fi

if command -v fetch-content >/dev/null 2>&1; then
  info "fetch-content -> $(command -v fetch-content)"
else
  warn "fetch-content is not visible on PATH in this shell yet. Open a new shell or update PATH."
fi

info "Done. Try:"
printf '  search-web "bun typescript playwright" --limit 5 --timeout 15000\n'
printf '  fetch-content https://example.com --max-chars 500\n'
