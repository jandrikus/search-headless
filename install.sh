#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"

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

# ── Install obscura if missing ──────────────────────────────────────────────
if ! command -v obscura >/dev/null 2>&1; then
  info "obscura not found. Installing..."

  mkdir -p "$BIN_DIR"

  OS="$(uname -s)"
  ARCH="$(uname -m)"

  case "$OS" in
    Linux)  OBS_OS="linux" ;;
    Darwin) OBS_OS="macos" ;;
    *)      fail "Unsupported OS for obscura auto-install: $OS. Install manually from https://github.com/h4ckf0r0day/obscura" ;;
  esac

  case "$ARCH" in
    x86_64|amd64)  OBS_ARCH="x86_64" ;;
    aarch64|arm64) OBS_ARCH="aarch64" ;;
    *)             fail "Unsupported architecture for obscura auto-install: $ARCH. Install manually from https://github.com/h4ckf0r0day/obscura" ;;
  esac

  OBS_TARBALL="obscura-${OBS_ARCH}-${OBS_OS}.tar.gz"
  OBS_URL="https://github.com/h4ckf0r0day/obscura/releases/latest/download/${OBS_TARBALL}"

  info "Downloading obscura from ${OBS_URL}..."
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$OBS_URL" | tar -xz -C "$BIN_DIR"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$OBS_URL" | tar -xz -C "$BIN_DIR"
  else
    fail "Neither curl nor wget found. Install obscura manually from https://github.com/h4ckf0r0day/obscura"
  fi

  # Ensure the binary is on PATH for this session.
  if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
    export PATH="${BIN_DIR}:${PATH}"
  fi

  if command -v obscura >/dev/null 2>&1; then
    info "obscura installed -> $(command -v obscura)"
  else
    fail "obscura installation failed. Install manually from https://github.com/h4ckf0r0day/obscura"
  fi
else
  info "obscura found -> $(command -v obscura)"
fi

# ── Build ───────────────────────────────────────────────────────────────────
command -v cargo >/dev/null 2>&1 || fail "cargo was not found on PATH.\n  Install Rust: https://rustup.rs"

info "Building with cargo ($(cargo --version))..."
cd "$PROJECT_DIR"
cargo build --release

# ── Install the binary and its aliases ──────────────────────────────────────
mkdir -p "$BIN_DIR"
install -m 755 "${PROJECT_DIR}/target/release/search-headless" "${BIN_DIR}/search-headless"

# The binary dispatches on the name it is invoked under, so these symlinks are
# the whole of `search-web` and `fetch-content`.
for alias in search-web fetch-content; do
  ln -sf "${BIN_DIR}/search-headless" "${BIN_DIR}/${alias}"
  info "${alias} -> ${BIN_DIR}/search-headless"
done

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
  warn "${BIN_DIR} is not on PATH."
  warn "Add this to your shell config:"
  warn "  export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ── Verify ──────────────────────────────────────────────────────────────────
info "Verifying installed commands..."
for cmd in search-headless search-web fetch-content; do
  if command -v "$cmd" >/dev/null 2>&1; then
    info "$cmd -> $(command -v "$cmd")"
  else
    warn "$cmd is not visible on PATH in this shell yet. Open a new shell or update PATH."
  fi
done

# ── Done ────────────────────────────────────────────────────────────────────
info "Done! Try:"
printf '  search-web "bun typescript" --limit 5 --timeout 15000\n'
printf '  fetch-content https://example.com\n'
printf '\nThe binary at %s is self-contained apart from libc.\n' "${BIN_DIR}/search-headless"
printf 'See the README for a fully static musl build.\n'
