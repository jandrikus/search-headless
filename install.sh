#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
CLI_VERSION="v0.1.0"
INSTALL_BINARY=false
if [[ "${1:-}" == "--binary" && $# -eq 1 ]]; then
  INSTALL_BINARY=true
elif [[ $# -ne 0 ]]; then
  printf 'Usage: bash install.sh [--binary]\n' >&2
  exit 2
fi

# Release digests below are from the upstream GitHub release API.
OBSCURA_VERSION="${OBSCURA_VERSION:-v0.2.6}"

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

  # search-headless uses --stealth, which requires the stealth build in v0.2.6.
  OBS_TARBALL="obscura-${OBS_ARCH}-${OBS_OS}-stealth.tar.gz"
  OBS_URL="https://github.com/h4ckf0r0day/obscura/releases/download/${OBSCURA_VERSION}/${OBS_TARBALL}"
  OBS_EXPECTED="${OBSCURA_SHA256:-}"
  if [[ -z "$OBS_EXPECTED" && "$OBSCURA_VERSION" == "v0.2.6" ]]; then
    case "${OBS_ARCH}-${OBS_OS}" in
      x86_64-linux)   OBS_EXPECTED="ac8057374ec1b5cabb78ea2d08b2ed890d21d156a42b1ac23357fd049edc08c5" ;;
      aarch64-linux)  OBS_EXPECTED="403b4c8e3bcf777748a870fbb32c9281e997a6c30c5f8377c17305287fc3811f" ;;
      x86_64-macos)   OBS_EXPECTED="23ececed3162e337f4cce97eb3c69611b253fc32f383753bee40f817a1220953" ;;
      aarch64-macos)  OBS_EXPECTED="01a29bb9eb3a7eda89c036db187bbdd9a01b76d3d8e4860117c00fc881ac40bf" ;;
    esac
  fi
  [[ -n "$OBS_EXPECTED" ]] || fail "Set OBSCURA_SHA256 when selecting a different Obscura version."

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

  OBS_SUM=""
  if command -v sha256sum >/dev/null 2>&1; then
    OBS_SUM="$(sha256sum "${OBS_TMP}/${OBS_TARBALL}" | cut -d' ' -f1)"
  elif command -v shasum >/dev/null 2>&1; then
    OBS_SUM="$(shasum -a 256 "${OBS_TMP}/${OBS_TARBALL}" | cut -d' ' -f1)"
  fi

  [[ -n "$OBS_SUM" ]] || fail "Neither sha256sum nor shasum is available to verify the download."
  [[ "$OBS_SUM" == "$OBS_EXPECTED" ]] || fail "Checksum mismatch for ${OBS_TARBALL}.
  expected ${OBS_EXPECTED}
  got      ${OBS_SUM}"
  info "Checksum verified."

  tar -xzf "${OBS_TMP}/${OBS_TARBALL}" -C "$OBS_TMP"
  [[ -f "${OBS_TMP}/obscura" ]] || fail "The Obscura archive did not contain the expected binary."
  install -m 755 "${OBS_TMP}/obscura" "${BIN_DIR}/obscura"

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
  OBS_INSTALLED_VERSION="$(obscura --version 2>/dev/null || true)"
  if [[ "$OBS_INSTALLED_VERSION" != "obscura ${OBSCURA_VERSION#v}" ]]; then
    fail "Found ${OBS_INSTALLED_VERSION:-an unknown Obscura version}, but this installer expects ${OBSCURA_VERSION}.
  Install the expected version or remove the old executable from PATH, then retry."
  fi
  info "obscura found -> $(command -v obscura) (${OBS_INSTALLED_VERSION})"
fi

# ── Install search-headless ─────────────────────────────────────────────────
mkdir -p "$BIN_DIR"
if "$INSTALL_BINARY"; then
  command -v curl >/dev/null 2>&1 || fail "curl is required for the prebuilt installer."
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64|Linux-amd64) CLI_TARGET="x86_64-unknown-linux-gnu" ;;
    Darwin-arm64|Darwin-aarch64) CLI_TARGET="aarch64-apple-darwin" ;;
    *) fail "No prebuilt search-headless release for this OS and architecture. Run bash install.sh to build from source." ;;
  esac
  CLI_TARBALL="search-headless-${CLI_TARGET}.tar.gz"
  CLI_URL="https://github.com/jandrikus/search-headless/releases/download/${CLI_VERSION}"
  CLI_TMP="$(mktemp -d)"
  # The Obscura temporary directory may already exist.
  trap 'rm -rf "${OBS_TMP:-}" "$CLI_TMP"' EXIT
  info "Downloading search-headless ${CLI_VERSION}..."
  curl -fsSL "${CLI_URL}/SHA256SUMS" -o "${CLI_TMP}/SHA256SUMS"
  curl -fsSL "${CLI_URL}/${CLI_TARBALL}" -o "${CLI_TMP}/${CLI_TARBALL}"
  CLI_EXPECTED="$(awk -v file="$CLI_TARBALL" '$2 == file { print $1 }' "${CLI_TMP}/SHA256SUMS")"
  [[ "$CLI_EXPECTED" =~ ^[0-9a-f]{64}$ ]] || fail "Release checksum is missing or invalid."
  if command -v sha256sum >/dev/null 2>&1; then
    CLI_SUM="$(sha256sum "${CLI_TMP}/${CLI_TARBALL}" | cut -d' ' -f1)"
  elif command -v shasum >/dev/null 2>&1; then
    CLI_SUM="$(shasum -a 256 "${CLI_TMP}/${CLI_TARBALL}" | cut -d' ' -f1)"
  else
    fail "Neither sha256sum nor shasum is available to verify the download."
  fi
  [[ "$CLI_SUM" == "$CLI_EXPECTED" ]] || fail "Checksum mismatch for ${CLI_TARBALL}."
  tar -xzf "${CLI_TMP}/${CLI_TARBALL}" -C "$CLI_TMP" search-headless
  install -m 755 "${CLI_TMP}/search-headless" "${BIN_DIR}/search-headless"
else
  command -v cargo >/dev/null 2>&1 || fail "cargo was not found on PATH. Install Rust: https://rustup.rs"
  info "Building with cargo ($(cargo --version))..."
  cd "$PROJECT_DIR"
  cargo build --locked --release
  install -m 755 "${PROJECT_DIR}/target/release/search-headless" "${BIN_DIR}/search-headless"
fi

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
printf '\nInstalled at %s.\n' "${BIN_DIR}/search-headless"
