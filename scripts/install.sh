#!/bin/sh
# llmeter install script — installs a prebuilt binary from GitHub Releases.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/bengHak/llmeter/master/scripts/install.sh | sh
#   curl -fsSL .../install.sh | LLMETER_VERSION=0.1.0 sh
#
# Env:
#   LLMETER_VERSION   optional; pin version (0.1.0 or v0.1.0). Default: latest release.
#   INSTALL_DIR       default: $HOME/.local/bin
#   LLMETER_REPO      default: bengHak/llmeter
#   GITHUB_TOKEN      optional; Authorization for API/rate limits (never logged)
#
# Test overrides (not for end users):
#   LLMETER_UNAME_S / LLMETER_UNAME_M  fake uname results
#   LLMETER_API_BASE                   default https://api.github.com
#   LLMETER_DOWNLOAD_BASE              default https://github.com/<repo>/releases/download
#
# Supported targets:
#   aarch64-apple-darwin
#   x86_64-unknown-linux-gnu
#   aarch64-unknown-linux-gnu

set -eu

REPO="${LLMETER_REPO:-bengHak/llmeter}"
INSTALL_DIR="${INSTALL_DIR:-${HOME}/.local/bin}"
API_BASE="${LLMETER_API_BASE:-https://api.github.com}"
# DOWNLOAD_BASE is set after REPO is known if unset
DOWNLOAD_BASE="${LLMETER_DOWNLOAD_BASE:-}"

log() {
  printf '%s\n' "$*"
}

err() {
  printf 'llmeter-install: %s\n' "$*" >&2
}

die() {
  err "$*"
  exit 1
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    die "required command not found: $1"
  fi
}

http_get() {
  # http_get URL [output_file]
  # If output_file is set, write body there; else print body to stdout.
  _url="$1"
  _out="${2:-}"

  if command -v curl >/dev/null 2>&1; then
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      if [ -n "$_out" ]; then
        curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN}" -o "$_out" "$_url" || return 1
      else
        curl -fsSL -H "Authorization: Bearer ${GITHUB_TOKEN}" "$_url" || return 1
      fi
    else
      if [ -n "$_out" ]; then
        curl -fsSL -o "$_out" "$_url" || return 1
      else
        curl -fsSL "$_url" || return 1
      fi
    fi
  elif command -v wget >/dev/null 2>&1; then
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      if [ -n "$_out" ]; then
        wget -q -O "$_out" --header="Authorization: Bearer ${GITHUB_TOKEN}" "$_url" || return 1
      else
        wget -q -O - --header="Authorization: Bearer ${GITHUB_TOKEN}" "$_url" || return 1
      fi
    else
      if [ -n "$_out" ]; then
        wget -q -O "$_out" "$_url" || return 1
      else
        wget -q -O - "$_url" || return 1
      fi
    fi
  else
    die "need curl or wget to download releases"
  fi
}

normalize_version() {
  # strip leading v
  _v="$1"
  case "$_v" in
    v*|V*) printf '%s\n' "${_v#?}" ;;
    *) printf '%s\n' "$_v" ;;
  esac
}

detect_target() {
  _os="${LLMETER_UNAME_S:-$(uname -s)}"
  _arch="${LLMETER_UNAME_M:-$(uname -m)}"

  case "$_arch" in
    arm64|aarch64) _arch=aarch64 ;;
    x86_64|amd64) _arch=x86_64 ;;
    *)
      die "unsupported architecture: ${_arch} (supported: aarch64/arm64, x86_64/amd64)"
      ;;
  esac

  case "$_os" in
    Darwin)
      if [ "$_arch" = "x86_64" ]; then
        die "macOS Intel (x86_64) is not supported yet. Supported: macOS arm64, Linux x86_64, Linux arm64."
      fi
      if [ "$_arch" = "aarch64" ]; then
        printf '%s\n' "aarch64-apple-darwin"
        return 0
      fi
      die "unsupported Darwin architecture: ${_arch}"
      ;;
    Linux)
      if [ "$_arch" = "x86_64" ]; then
        printf '%s\n' "x86_64-unknown-linux-gnu"
        return 0
      fi
      if [ "$_arch" = "aarch64" ]; then
        printf '%s\n' "aarch64-unknown-linux-gnu"
        return 0
      fi
      die "unsupported Linux architecture: ${_arch}"
      ;;
    *)
      die "unsupported OS: ${_os} (supported: macOS arm64, Linux x86_64, Linux arm64; no Windows)"
      ;;
  esac
}

resolve_version() {
  if [ -n "${LLMETER_VERSION:-}" ]; then
    normalize_version "$LLMETER_VERSION"
    return 0
  fi

  _api_url="${API_BASE}/repos/${REPO}/releases/latest"
  _json=
  if ! _json=$(http_get "$_api_url"); then
    die "failed to fetch latest release from ${_api_url} (is there a published release?)"
  fi

  # Prefer tag_name from JSON without requiring jq.
  _tag=
  _tag=$(printf '%s\n' "$_json" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
  if [ -z "$_tag" ]; then
    die "could not parse tag_name from latest release response"
  fi
  normalize_version "$_tag"
}

sha256_file() {
  _file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$_file" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$_file" | awk '{print $1}'
  else
    die "need sha256sum or shasum to verify downloads"
  fi
}

verify_checksum() {
  _tarball="$1"
  _sums_file="$2"
  _name="$3"

  _expected=
  # Match filename at end of line; accept two spaces or space-asterisk (BSD/GNU).
  _expected=$(awk -v f="$_name" '
    $2 == f || $2 == ("*" f) { print $1; exit }
  ' "$_sums_file")

  if [ -z "$_expected" ]; then
    die "no checksum entry for ${_name} in SHA256SUMS"
  fi

  _actual=$(sha256_file "$_tarball")
  if [ "$_actual" != "$_expected" ]; then
    die "SHA256 mismatch for ${_name}: expected ${_expected}, got ${_actual}"
  fi
}

path_has_dir() {
  _dir="$1"
  case ":${PATH}:" in
    *":${_dir}:"*) return 0 ;;
    *) return 1 ;;
  esac
}

main() {
  need_cmd uname
  need_cmd mktemp
  need_cmd tar
  need_cmd mkdir
  need_cmd mv
  need_cmd chmod
  need_cmd awk
  need_cmd sed
  need_cmd head

  if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
    die "need curl or wget to download releases"
  fi
  if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
    die "need sha256sum or shasum to verify downloads"
  fi

  if [ -z "$DOWNLOAD_BASE" ]; then
    DOWNLOAD_BASE="https://github.com/${REPO}/releases/download"
  fi

  TARGET=$(detect_target)
  VERSION=$(resolve_version)
  if [ -z "$VERSION" ]; then
    die "empty version after resolve"
  fi

  ARCHIVE_NAME="llmeter-${VERSION}-${TARGET}.tar.gz"
  TAG="v${VERSION}"
  TARBALL_URL="${DOWNLOAD_BASE}/${TAG}/${ARCHIVE_NAME}"
  SUMS_URL="${DOWNLOAD_BASE}/${TAG}/SHA256SUMS"

  TMPDIR_INSTALL=$(mktemp -d)
  cleanup() {
    rm -rf "$TMPDIR_INSTALL"
  }
  trap cleanup EXIT INT HUP TERM

  log "installing llmeter ${VERSION} (${TARGET}) → ${INSTALL_DIR}/llmeter"

  if ! http_get "$TARBALL_URL" "${TMPDIR_INSTALL}/${ARCHIVE_NAME}"; then
    die "failed to download ${TARBALL_URL}"
  fi
  if ! http_get "$SUMS_URL" "${TMPDIR_INSTALL}/SHA256SUMS"; then
    die "failed to download ${SUMS_URL}"
  fi

  verify_checksum \
    "${TMPDIR_INSTALL}/${ARCHIVE_NAME}" \
    "${TMPDIR_INSTALL}/SHA256SUMS" \
    "$ARCHIVE_NAME"

  if ! tar -xzf "${TMPDIR_INSTALL}/${ARCHIVE_NAME}" -C "$TMPDIR_INSTALL"; then
    die "failed to extract ${ARCHIVE_NAME}"
  fi

  if [ ! -f "${TMPDIR_INSTALL}/llmeter" ]; then
    die "tarball did not contain ./llmeter at archive root"
  fi

  mkdir -p "$INSTALL_DIR"
  # Install via temp name then rename for near-atomic replace.
  _dest="${INSTALL_DIR}/llmeter"
  _tmp_dest="${INSTALL_DIR}/.llmeter.install.$$"
  if ! cp "${TMPDIR_INSTALL}/llmeter" "$_tmp_dest"; then
    rm -f "$_tmp_dest"
    die "failed to write ${_tmp_dest} (check permissions on ${INSTALL_DIR})"
  fi
  chmod +x "$_tmp_dest"
  if ! mv -f "$_tmp_dest" "$_dest"; then
    rm -f "$_tmp_dest"
    die "failed to install to ${_dest}"
  fi

  log "installed: ${_dest}"

  if "$_dest" --version >/dev/null 2>&1; then
    log "version: $("$_dest" --version 2>/dev/null || true)"
  elif "$_dest" -V >/dev/null 2>&1; then
    log "version: $("$_dest" -V 2>/dev/null || true)"
  else
    err "warning: installed binary could not report --version (still installed)"
  fi

  if ! path_has_dir "$INSTALL_DIR"; then
    log ""
    log "${INSTALL_DIR} is not on your PATH. Add it, for example:"
    log "  export PATH=\"${INSTALL_DIR}:\$PATH\""
  fi
}

main "$@"
