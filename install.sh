#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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

# ── Detect package manager ──────────────────────────────────────────────────
PACKAGE_MANAGER=""
if command -v bun >/dev/null 2>&1; then
  PACKAGE_MANAGER="bun"
  info "Found bun ($(command -v bun))"
elif command -v npm >/dev/null 2>&1; then
  PACKAGE_MANAGER="npm"
  info "Found npm ($(command -v npm))"
else
  fail "Neither bun nor npm was found on PATH.\n  Install bun (recommended): https://bun.sh\n  Or install npm (comes with Node.js): https://nodejs.org"
fi

# ── Install obscura if missing ──────────────────────────────────────────────
if ! command -v obscura >/dev/null 2>&1; then
  info "obscura not found. Installing..."

  # Determine install directory
  OBS_BIN_DIR="${HOME}/.local/bin"
  mkdir -p "$OBS_BIN_DIR"

  # Detect OS + arch
  OS="$(uname -s)"
  ARCH="$(uname -m)"

  case "$OS" in
    Linux)  OBS_OS="unknown-linux-gnu" ;;
    Darwin) OBS_OS="apple-darwin" ;;
    *)      fail "Unsupported OS for obscura auto-install: $OS. Install manually from https://github.com/jandrikus/obscura" ;;
  esac

  case "$ARCH" in
    x86_64|amd64)  OBS_ARCH="x86_64" ;;
    aarch64|arm64) OBS_ARCH="aarch64" ;;
    *)             fail "Unsupported architecture for obscura auto-install: $ARCH. Install manually from https://github.com/jandrikus/obscura" ;;
  esac

  OBS_TARBALL="obscura-${OBS_ARCH}-${OBS_OS}.tar.gz"
  OBS_URL="https://github.com/jandrikus/obscura/releases/latest/download/${OBS_TARBALL}"

  info "Downloading obscura from ${OBS_URL}..."
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$OBS_URL" | tar -xz -C "$OBS_BIN_DIR"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$OBS_URL" | tar -xz -C "$OBS_BIN_DIR"
  else
    fail "Neither curl nor wget found. Install obscura manually from https://github.com/jandrikus/obscura"
  fi

  # Ensure the binary is on PATH for this session
  if [[ ":${PATH}:" != *":${OBS_BIN_DIR}:"* ]]; then
    export PATH="${OBS_BIN_DIR}:${PATH}"
    warn "Added ${OBS_BIN_DIR} to PATH for this session."
    warn "Add this to your shell config for persistence:"
    warn "  export PATH=\"\$HOME/.local/bin:\$PATH\""
  fi

  # Verify installation
  if command -v obscura >/dev/null 2>&1; then
    info "obscura installed -> $(command -v obscura)"
  else
    fail "obscura installation failed. Install manually from https://github.com/jandrikus/obscura"
  fi
else
  info "obscura found -> $(command -v obscura)"
fi

# ── Install dependencies ───────────────────────────────────────────────────
cd "$PROJECT_DIR"

info "Installing dependencies with ${PACKAGE_MANAGER}..."
if [[ "$PACKAGE_MANAGER" == "bun" ]]; then
  bun install
else
  npm install
fi

# ── Install Playwright Chromium ────────────────────────────────────────────
info "Installing Playwright Chromium (required for search-web)..."
if [[ "$PACKAGE_MANAGER" == "bun" ]]; then
  bunx playwright install chromium
else
  npx playwright install chromium
fi

# ── Link commands globally ─────────────────────────────────────────────────
info "Linking commands globally with ${PACKAGE_MANAGER}..."
if [[ "$PACKAGE_MANAGER" == "bun" ]]; then
  BUN_BIN_DIR="${HOME}/.bun/bin"
  bun unlink 2>/dev/null || true
  bun link

  if [[ ":${PATH}:" != *":${BUN_BIN_DIR}:"* ]]; then
    warn "${BUN_BIN_DIR} is not currently on PATH."
    warn "Add this to your shell config:"
    warn "  export PATH=\"\$HOME/.bun/bin:\$PATH\""
  else
    info "Bun global bin directory is already on PATH."
  fi
else
  npm link
  NPM_BIN_DIR="$(npm bin -g 2>/dev/null || true)"
  if [[ -n "$NPM_BIN_DIR" ]] && [[ ":${PATH}:" != *":${NPM_BIN_DIR}:"* ]]; then
    warn "${NPM_BIN_DIR} is not currently on PATH."
    warn "Add it to your shell config for persistent access."
  fi
fi

# ── Verify linked commands ─────────────────────────────────────────────────
info "Verifying linked commands..."
for cmd in search-web fetch-content; do
  if command -v "$cmd" >/dev/null 2>&1; then
    info "$cmd -> $(command -v "$cmd")"
  else
    warn "$cmd is not visible on PATH in this shell yet. Open a new shell or update PATH."
  fi
done

# ── Done ────────────────────────────────────────────────────────────────────
info "Done! Try:"
printf '  search-web "bun typescript playwright" --limit 5 --timeout 15000\n'
printf '  fetch-content https://example.com\n'
