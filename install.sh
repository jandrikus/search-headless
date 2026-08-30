#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"

# Tracks obscura's newest release. Set OBSCURA_VERSION to a tag to hold a
# specific one instead, e.g. OBSCURA_VERSION=v0.2.1 ./install.sh
OBSCURA_VERSION="${OBSCURA_VERSION:-latest}"

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
  # GitHub spells the two cases differently: `latest/download/ASSET`, but
  # `download/TAG/ASSET` for a named release.
  if [[ "$OBSCURA_VERSION" == "latest" ]]; then
    OBS_URL="https://github.com/h4ckf0r0day/obscura/releases/latest/download/${OBS_TARBALL}"
  else
    OBS_URL="https://github.com/h4ckf0r0day/obscura/releases/download/${OBSCURA_VERSION}/${OBS_TARBALL}"
  fi

  # Land the archive on disk before unpacking it. Piping the download straight
  # into tar extracts a stream nobody has seen, and leaves a truncated transfer
  # indistinguishable from a complete one.
  OBS_TMP="$(mktemp -d)"
  trap 'rm -rf "$OBS_TMP"' EXIT

  info "Downloading obscura ${OBSCURA_VERSION} from ${OBS_URL}..."
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$OBS_URL" -o "${OBS_TMP}/${OBS_TARBALL}"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "${OBS_TMP}/${OBS_TARBALL}" "$OBS_URL"
  else
    fail "Neither curl nor wget found. Install obscura manually from https://github.com/h4ckf0r0day/obscura"
  fi

  # Upstream publishes no checksum manifest, so we cannot verify against the
  # project. Print what we got, and enforce a digest when the caller pins one.
  OBS_SUM=""
  if command -v sha256sum >/dev/null 2>&1; then
    OBS_SUM="$(sha256sum "${OBS_TMP}/${OBS_TARBALL}" | cut -d' ' -f1)"
  elif command -v shasum >/dev/null 2>&1; then
    OBS_SUM="$(shasum -a 256 "${OBS_TMP}/${OBS_TARBALL}" | cut -d' ' -f1)"
  fi

  if [[ -n "${OBSCURA_SHA256:-}" ]]; then
    [[ -n "$OBS_SUM" ]] || fail "OBSCURA_SHA256 is set but no sha256sum/shasum is available to check it."
    [[ "$OBS_SUM" == "$OBSCURA_SHA256" ]] || fail "Checksum mismatch for ${OBS_TARBALL}.
  expected ${OBSCURA_SHA256}
  got      ${OBS_SUM}"
    info "Checksum verified."
  elif [[ -n "$OBS_SUM" ]]; then
    info "SHA256 ${OBS_SUM}"
    warn "Not verified. Pin it with OBSCURA_SHA256=<digest> to enforce this exact build."
  fi

  tar -xzf "${OBS_TMP}/${OBS_TARBALL}" -C "$BIN_DIR"

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

# ── Install the binary ──────────────────────────────────────────────────────
mkdir -p "$BIN_DIR"
install -m 755 "${PROJECT_DIR}/target/release/search-headless" "${BIN_DIR}/search-headless"

if [[ ":${PATH}:" != *":${BIN_DIR}:"* ]]; then
  warn "${BIN_DIR} is not on PATH."
  warn "Add this to your shell config:"
  warn "  export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ── Verify ──────────────────────────────────────────────────────────────────
info "Verifying the installed command..."
if command -v search-headless >/dev/null 2>&1; then
  info "search-headless -> $(command -v search-headless)"
else
  warn "search-headless is not visible on PATH in this shell yet. Open a new shell or update PATH."
fi

# ── Done ────────────────────────────────────────────────────────────────────
info "Done! Try:"
printf '  search-headless search "rust async" --limit 5 --timeout 15000\n'
printf '  search-headless fetch https://example.com\n'
printf '\nThe binary at %s is self-contained apart from libc.\n' "${BIN_DIR}/search-headless"
printf 'See the README for a fully static musl build.\n'
