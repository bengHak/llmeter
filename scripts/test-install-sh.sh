#!/bin/sh
# Fixture harness for scripts/install.sh — drives the real entrypoint.
# Usage: sh scripts/test-install-sh.sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
INSTALL_SH="${ROOT}/scripts/install.sh"
SCRATCH_DEFAULT="${ROOT}/.tmp-install-harness"
WORK="${HARNESS_WORK:-$SCRATCH_DEFAULT}"
PASS=0
FAIL=0

log() { printf '%s\n' "$*"; }
ok() { PASS=$((PASS + 1)); log "PASS: $*"; }
bad() { FAIL=$((FAIL + 1)); log "FAIL: $*"; }

die_harness() {
  log "harness error: $*"
  exit 2
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die_harness "need $1"
}

need_cmd python3
need_cmd tar
need_cmd shasum 2>/dev/null || need_cmd sha256sum
need_cmd awk
need_cmd curl

rm -rf "$WORK"
mkdir -p "$WORK"

# --- syntax check first ---
if sh -n "$INSTALL_SH"; then
  ok "sh -n scripts/install.sh"
else
  bad "sh -n scripts/install.sh"
fi

# --- prepare fake binary + tarball for a known target ---
VERSION="0.1.0"
TARGET="aarch64-apple-darwin"
ARCHIVE_NAME="llmeter-${VERSION}-${TARGET}.tar.gz"
ASSET_DIR="${WORK}/assets/v${VERSION}"
mkdir -p "$ASSET_DIR" "${WORK}/bin-src"

# Fake "binary": small shell script that reports a version (executable).
cat >"${WORK}/bin-src/llmeter" <<'EOF'
#!/bin/sh
if [ "${1:-}" = "--version" ] || [ "${1:-}" = "-V" ]; then
  echo "llmeter 0.1.0 (fixture)"
  exit 0
fi
echo "llmeter fixture binary"
EOF
chmod +x "${WORK}/bin-src/llmeter"

(
  cd "${WORK}/bin-src"
  tar -czf "${ASSET_DIR}/${ARCHIVE_NAME}" llmeter
)

checksum_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

GOOD_HASH=$(checksum_file "${ASSET_DIR}/${ARCHIVE_NAME}")
printf '%s  %s\n' "$GOOD_HASH" "$ARCHIVE_NAME" >"${ASSET_DIR}/SHA256SUMS"

# Bad checksums file (wrong hash)
printf '%s  %s\n' "0000000000000000000000000000000000000000000000000000000000000000" "$ARCHIVE_NAME" \
  >"${ASSET_DIR}/SHA256SUMS.bad"

# Latest API fixture
mkdir -p "${WORK}/api/repos/bengHak/llmeter/releases"
printf '%s\n' '{"tag_name":"v0.1.0","name":"llmeter 0.1.0"}' \
  >"${WORK}/api/repos/bengHak/llmeter/releases/latest"

# --- start local HTTP server ---
# Layout:
#   /v0.1.0/<assets>           via LLMETER_DOWNLOAD_BASE=http://host:port
#   /repos/.../releases/latest via LLMETER_API_BASE=http://host:port
#
# We serve WORK as root with two roots merged via a tiny python server.

PORT_FILE="${WORK}/port"
SERVER_LOG="${WORK}/server.log"
SERVER_PID=""

python3 - "$WORK" "$PORT_FILE" <<'PY' >"$SERVER_LOG" 2>&1 &
import http.server
import os
import sys

root = sys.argv[1]
port_file = sys.argv[2]
assets = os.path.abspath(os.path.join(root, "assets"))
api = os.path.abspath(os.path.join(root, "api"))

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def _safe_join(self, base, url_path):
        # url_path starts with /
        candidate = os.path.abspath(os.path.join(base, url_path.lstrip("/")))
        if not candidate.startswith(base + os.sep) and candidate != base:
            return None
        return candidate

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path.startswith("/repos/"):
            fs = self._safe_join(api, path)
            if fs is None or not os.path.isfile(fs):
                self.send_error(404, "missing %s" % path)
                return
            with open(fs, "rb") as fh:
                data = fh.read()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return

        if path.startswith("/v"):
            fs = self._safe_join(assets, path)
            if fs is None or not os.path.isfile(fs):
                self.send_error(404, "missing asset %s" % path)
                return
            with open(fs, "rb") as fh:
                data = fh.read()
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return

        self.send_error(404, path)

httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
port = httpd.server_address[1]
with open(port_file, "w") as f:
    f.write(str(port))
httpd.serve_forever()
PY
SERVER_PID=$!

cleanup() {
  if [ -n "${SERVER_PID:-}" ]; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT HUP TERM

# Wait for port file
i=0
while [ ! -f "$PORT_FILE" ]; do
  i=$((i + 1))
  if [ "$i" -gt 50 ]; then
    cat "$SERVER_LOG" || true
    die_harness "server did not start"
  fi
  sleep 0.1
done
PORT=$(cat "$PORT_FILE")
BASE="http://127.0.0.1:${PORT}"

# Sanity: fetch latest + asset
curl -fsSL "${BASE}/repos/bengHak/llmeter/releases/latest" >/dev/null
curl -fsSL "${BASE}/v${VERSION}/${ARCHIVE_NAME}" >/dev/null

run_install() {
  # Args become env assignments before install — pass as KEY=VAL pairs then --
  # Usage: run_install OUTDIR [extra env]
  _outdir="$1"
  shift
  mkdir -p "$_outdir"
  env \
    INSTALL_DIR="$_outdir" \
    LLMETER_REPO="bengHak/llmeter" \
    LLMETER_API_BASE="${BASE}" \
    LLMETER_DOWNLOAD_BASE="${BASE}" \
    LLMETER_UNAME_S="Darwin" \
    LLMETER_UNAME_M="arm64" \
    "$@" \
    sh "$INSTALL_SH"
}

# --- Test 1: good checksum installs executable ---
GOOD_DIR="${WORK}/install-good"
if run_install "$GOOD_DIR" LLMETER_VERSION="$VERSION" >"${WORK}/good.out" 2>"${WORK}/good.err"; then
  if [ -x "${GOOD_DIR}/llmeter" ]; then
    VER_OUT=$("${GOOD_DIR}/llmeter" --version 2>/dev/null || true)
    case "$VER_OUT" in
      *0.1.0*) ok "good checksum installs executable binary (got: ${VER_OUT})" ;;
      *) bad "good install: binary exists but --version unexpected: ${VER_OUT}" ;;
    esac
  else
    bad "good checksum: missing executable ${GOOD_DIR}/llmeter"
  fi
else
  bad "good checksum install exited non-zero"
  log "--- good.out ---"; cat "${WORK}/good.out" || true
  log "--- good.err ---"; cat "${WORK}/good.err" || true
fi

# --- Test 2: bad checksum → non-zero, no bad binary left ---
BAD_DIR="${WORK}/install-bad"
# Point at bad sums by swapping file served: temporarily replace SHA256SUMS
cp "${ASSET_DIR}/SHA256SUMS" "${ASSET_DIR}/SHA256SUMS.good.bak"
cp "${ASSET_DIR}/SHA256SUMS.bad" "${ASSET_DIR}/SHA256SUMS"
set +e
run_install "$BAD_DIR" LLMETER_VERSION="$VERSION" >"${WORK}/bad.out" 2>"${WORK}/bad.err"
BAD_RC=$?
set -e
# restore good sums
mv "${ASSET_DIR}/SHA256SUMS.good.bak" "${ASSET_DIR}/SHA256SUMS"

if [ "$BAD_RC" -ne 0 ]; then
  if [ -e "${BAD_DIR}/llmeter" ]; then
    bad "bad checksum left a binary at ${BAD_DIR}/llmeter"
  else
    ok "bad checksum exits non-zero and leaves no binary"
  fi
else
  bad "bad checksum should exit non-zero (rc=0)"
fi

# Confirm error mentions checksum
if grep -qi 'SHA256\|checksum\|mismatch' "${WORK}/bad.err"; then
  ok "bad checksum error message mentions checksum"
else
  bad "bad checksum stderr missing mismatch text"
  log "--- bad.err ---"; cat "${WORK}/bad.err" || true
fi

# --- Test 3: unsupported arch (Darwin x86_64) ---
UNSUP_DIR="${WORK}/install-unsup"
set +e
env \
  INSTALL_DIR="$UNSUP_DIR" \
  LLMETER_REPO="bengHak/llmeter" \
  LLMETER_API_BASE="${BASE}" \
  LLMETER_DOWNLOAD_BASE="${BASE}" \
  LLMETER_VERSION="$VERSION" \
  LLMETER_UNAME_S="Darwin" \
  LLMETER_UNAME_M="x86_64" \
  sh "$INSTALL_SH" >"${WORK}/unsup.out" 2>"${WORK}/unsup.err"
UNSUP_RC=$?
set -e

if [ "$UNSUP_RC" -ne 0 ]; then
  if [ -e "${UNSUP_DIR}/llmeter" ]; then
    bad "unsupported arch left a binary"
  else
    ok "unsupported Darwin x86_64 exits non-zero"
  fi
else
  bad "unsupported arch should exit non-zero"
fi

if grep -qi 'not supported\|unsupported' "${WORK}/unsup.err"; then
  ok "unsupported arch error message is clear"
else
  bad "unsupported arch stderr missing clear message"
  log "--- unsup.err ---"; cat "${WORK}/unsup.err" || true
fi

# --- Test 4: latest resolution (no LLMETER_VERSION) ---
LATEST_DIR="${WORK}/install-latest"
if run_install "$LATEST_DIR" >"${WORK}/latest.out" 2>"${WORK}/latest.err"; then
  if [ -x "${LATEST_DIR}/llmeter" ]; then
    ok "latest release resolution installs binary"
  else
    bad "latest resolution: binary missing"
  fi
else
  bad "latest resolution install failed"
  log "--- latest.err ---"; cat "${WORK}/latest.err" || true
fi

log ""
log "Results: ${PASS} passed, ${FAIL} failed"
if [ "$FAIL" -ne 0 ]; then
  exit 1
fi
exit 0
