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
command -v obscura >/dev/null 2>&1 || fail "obscura is required but was not found on PATH. Install it from https://github.com/jandrikus/obscura"

cd "$PROJECT_DIR"

info "Installing dependencies with Bun..."
bun install

info "Installing Playwright Chromium (required for search-web)..."
bunx playwright install chromium

info "obscura -> $(command -v obscura)"

info "Linking package globally with bun link..."
bun unlink 2>/dev/null || true
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

info "Installing Pi skill..."
PI_SKILL_DIR="${HOME}/.pi/agent/skills/search-fetch"
mkdir -p "$PI_SKILL_DIR"
cp "$PROJECT_DIR/SKILL.md" "$PI_SKILL_DIR/SKILL.md"
info "Skill installed at $PI_SKILL_DIR/SKILL.md"

# Remove old separate fetch skill if it exists
if [ -d "${HOME}/.pi/agent/skills/fetch" ]; then
  rm -rf "${HOME}/.pi/agent/skills/fetch"
  info "Removed obsolete fetch skill."
fi
# Remove old search-web symlink if it exists
if [ -L "${HOME}/.pi/agent/skills/search-web" ]; then
  rm "${HOME}/.pi/agent/skills/search-web"
  info "Removed obsolete search-web skill symlink."
fi

info "Done. Try:"
printf '  search-web "bun typescript playwright" --limit 5 --timeout 15000\n'
printf '  fetch-content https://example.com\n'
